use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use shared::trusted::{
    SignedTrustedManifest, TRUSTED_PROTOCOL_VERSION, TrustedClientFrame, TrustedManifest,
};
use std::{collections::BTreeMap, path::Path};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct GateState {
    pub host_signing: SigningKey,
    pub transport_secret: StaticSecret,
    pub host_id: String,
    db_path: String,
    approvers_path: String,
}

impl GateState {
    pub fn load_or_create(state_dir: &Path, approvers_path: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(state_dir).map_err(|error| error.to_string())?;
        set_mode(state_dir, 0o700)?;
        let host_signing =
            SigningKey::from_bytes(&load_or_create_key(&state_dir.join("host-ed25519.key"))?);
        let transport_secret =
            StaticSecret::from(load_or_create_key(&state_dir.join("host-x25519.key"))?);
        let host_id = hex(&host_signing.verifying_key().to_bytes()[..16]);
        let db_path = state_dir.join("approvals.sqlite3");
        let connection = Connection::open(&db_path).map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS trusted_requests (
                    request_id TEXT PRIMARY KEY,
                    manifest BLOB NOT NULL,
                    status TEXT NOT NULL,
                    expires_at INTEGER NOT NULL,
                    consumed_at INTEGER
                 );",
            )
            .map_err(|error| error.to_string())?;
        set_mode(&db_path, 0o600)?;
        Ok(Self {
            host_signing,
            transport_secret,
            host_id,
            db_path: db_path.to_string_lossy().into_owned(),
            approvers_path: approvers_path.to_string_lossy().into_owned(),
        })
    }

    pub fn transport_public(&self) -> [u8; 32] {
        PublicKey::from(&self.transport_secret).to_bytes()
    }

    pub fn challenge(
        &self,
        start: &TrustedClientFrame,
        now: i64,
    ) -> Result<SignedTrustedManifest, String> {
        let TrustedClientFrame::Start {
            request_id,
            operation,
            client_ephemeral_public,
        } = start
        else {
            return Err("first trusted frame must be Start".into());
        };
        validate_request_id(request_id)?;
        crate::privileged::policy::classify(operation)?;
        let requested_ttl = match operation {
            shared::trusted::TrustedOperation::RootCommand { timeout_secs, .. } => {
                i64::from(*timeout_secs) + 60
            }
            shared::trusted::TrustedOperation::RootPty { ttl_secs, .. } => i64::from(*ttl_secs),
        };
        let mut nonce = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let manifest = TrustedManifest {
            version: TRUSTED_PROTOCOL_VERSION,
            request_id: request_id.clone(),
            host_id: self.host_id.clone(),
            operation: operation.clone(),
            client_ephemeral_public: *client_ephemeral_public,
            broker_transport_public: self.transport_public(),
            nonce,
            created_at: now,
            expires_at: now + requested_ttl.clamp(60, 3600),
            policy_version: crate::privileged::policy::policy_version(),
        };
        let canonical = manifest
            .canonical_bytes()
            .map_err(|error| error.to_string())?;
        let signature = self.host_signing.sign(&canonical).to_bytes().to_vec();
        let connection = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO trusted_requests(request_id, manifest, status, expires_at) VALUES (?1, ?2, 'pending', ?3)",
                params![request_id, canonical, manifest.expires_at],
            )
            .map_err(|_| "request id already exists or approval database failed")?;
        Ok(SignedTrustedManifest {
            manifest,
            host_identity_public: self.host_signing.verifying_key().to_bytes(),
            signature,
        })
    }

    pub fn approve(
        &self,
        signed: &SignedTrustedManifest,
        approver_public: [u8; 32],
        approver_signature: &[u8],
        now: i64,
    ) -> Result<TrustedManifest, String> {
        if signed.manifest.version != TRUSTED_PROTOCOL_VERSION
            || signed.manifest.host_id != self.host_id
            || signed.host_identity_public != self.host_signing.verifying_key().to_bytes()
            || signed.manifest.expires_at < now
            || signed.manifest.policy_version != crate::privileged::policy::policy_version()
        {
            return Err("trusted manifest is stale or belongs to another host/policy".into());
        }
        crate::privileged::policy::classify(&signed.manifest.operation)?;
        let canonical = signed
            .manifest
            .canonical_bytes()
            .map_err(|error| error.to_string())?;
        let host_signature = Signature::from_slice(&signed.signature)
            .map_err(|_| "invalid host manifest signature")?;
        self.host_signing
            .verifying_key()
            .verify(&canonical, &host_signature)
            .map_err(|_| "invalid host manifest signature")?;

        let approvers = load_approvers(Path::new(&self.approvers_path))?;
        if !approvers.values().any(|key| key == &approver_public) {
            return Err("approver key is not enrolled on this host".into());
        }
        let verifying = VerifyingKey::from_bytes(&approver_public)
            .map_err(|_| "invalid approver public key")?;
        let signature =
            Signature::from_slice(approver_signature).map_err(|_| "invalid approver signature")?;
        verifying
            .verify(&canonical, &signature)
            .map_err(|_| "invalid approver signature")?;

        let mut connection = Connection::open(&self.db_path).map_err(|error| error.to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let stored: Option<(Vec<u8>, String, i64)> = transaction
            .query_row(
                "SELECT manifest, status, expires_at FROM trusted_requests WHERE request_id = ?1",
                [&signed.manifest.request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((stored_manifest, status, expires_at)) = stored else {
            return Err("trusted request is unknown".into());
        };
        if stored_manifest != canonical || status != "pending" || expires_at < now {
            return Err("trusted approval was altered, expired, or already consumed".into());
        }
        let changed = transaction
            .execute(
                "UPDATE trusted_requests SET status='consumed', consumed_at=?2 WHERE request_id=?1 AND status='pending'",
                params![signed.manifest.request_id, now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("trusted approval was already consumed".into());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(signed.manifest.clone())
    }
}

fn load_or_create_key(path: &Path) -> Result<[u8; 32], String> {
    if let Ok(bytes) = std::fs::read(path) {
        return bytes
            .try_into()
            .map_err(|_| format!("{} must contain exactly 32 bytes", path.display()));
    }
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(
        &mut options.open(path).map_err(|error| error.to_string())?,
        &key,
    )
    .map_err(|error| error.to_string())?;
    Ok(key)
}

fn load_approvers(path: &Path) -> Result<BTreeMap<String, [u8; 32]>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read approver keyring {}: {error}", path.display()))?;
    let encoded: BTreeMap<String, String> =
        serde_json::from_str(&raw).map_err(|_| "invalid approver keyring JSON")?;
    if encoded.is_empty() {
        return Err("approver keyring is empty".into());
    }
    encoded
        .into_iter()
        .map(|(id, value)| {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(|_| "invalid approver key encoding")?;
            let key = decoded
                .try_into()
                .map_err(|_| "approver public keys must be 32 bytes")?;
            Ok((id, key))
        })
        .collect()
}

fn validate_request_id(id: &str) -> Result<(), String> {
    if (8..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        Ok(())
    } else {
        Err("invalid trusted request id".into())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn signed_approval_is_exact_single_use_and_persistent() {
        let temp = TempDir::new().unwrap();
        let approver = SigningKey::from_bytes(&[9; 32]);
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(approver.verifying_key().to_bytes());
        let approvers = temp.path().join("approvers.json");
        std::fs::write(&approvers, format!(r#"{{"operator":"{encoded}"}}"#)).unwrap();
        let state = GateState::load_or_create(temp.path(), &approvers).unwrap();
        let start = TrustedClientFrame::Start {
            request_id: "request-123".into(),
            operation: shared::trusted::TrustedOperation::RootCommand {
                program: "/usr/bin/id".into(),
                args: vec![],
                timeout_secs: 10,
            },
            client_ephemeral_public: [3; 32],
        };
        let challenge = state.challenge(&start, 100).unwrap();
        let canonical = challenge.manifest.canonical_bytes().unwrap();
        let signature = approver.sign(&canonical).to_bytes();
        assert!(
            state
                .approve(
                    &challenge,
                    approver.verifying_key().to_bytes(),
                    &signature,
                    101,
                )
                .is_ok()
        );
        assert!(
            state
                .approve(
                    &challenge,
                    approver.verifying_key().to_bytes(),
                    &signature,
                    102,
                )
                .is_err()
        );
    }
}
