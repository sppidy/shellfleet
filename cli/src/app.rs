use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared::trusted::{
    SignedTrustedManifest, TrustedClientFrame, TrustedHostFrame, TrustedOperation, TrustedPlaintext,
};
use std::{collections::BTreeMap, path::PathBuf};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::session::ClientTransport;

pub enum Mode {
    Fleet,
    Command,
    Review,
    Terminal,
}

pub struct Pending {
    pub agent: String,
    pub request_id: String,
    pub operation: TrustedOperation,
    ephemeral: StaticSecret,
    pub challenge: Option<SignedTrustedManifest>,
    pub pinned: bool,
    pub transport: Option<ClientTransport>,
}

pub struct App {
    pub agents: Vec<String>,
    pub selected: usize,
    pub mode: Mode,
    pub command: String,
    pub status: String,
    pub output: Vec<String>,
    pub pending: Option<Pending>,
    signer: SigningKey,
    pins_path: PathBuf,
    pins: BTreeMap<String, String>,
}

impl App {
    pub fn new(signer: SigningKey, pins_path: PathBuf) -> Self {
        let pins = std::fs::read(&pins_path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default();
        Self {
            agents: Vec::new(),
            selected: 0,
            mode: Mode::Fleet,
            command: String::new(),
            status: "Connected. Select a host, then : for root command or r for root PTY.".into(),
            output: Vec::new(),
            pending: None,
            signer,
            pins_path,
            pins,
        }
    }

    pub fn selected_agent(&self) -> Option<&str> {
        self.agents.get(self.selected).map(String::as_str)
    }

    pub fn begin(&mut self, operation: TrustedOperation) -> Result<shared::Message, String> {
        let agent = self
            .selected_agent()
            .ok_or("no online agent selected")?
            .to_owned();
        let mut random = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let request_id = format!("tui-{}", hex(&random));
        let ephemeral = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let start = TrustedClientFrame::Start {
            request_id: request_id.clone(),
            operation: operation.clone(),
            client_ephemeral_public: PublicKey::from(&ephemeral).to_bytes(),
        };
        let payload = shared::trusted::encode_client(&start)?;
        self.pending = Some(Pending {
            agent: agent.clone(),
            request_id: request_id.clone(),
            operation,
            ephemeral,
            challenge: None,
            pinned: false,
            transport: None,
        });
        self.mode = Mode::Review;
        self.status = format!("Waiting for host-signed manifest from {agent}");
        Ok(shared::Message::TrustedOperationClient {
            request_id,
            start: true,
            close: false,
            payload,
        })
    }

    pub fn challenge(&mut self, agent: &str, signed: SignedTrustedManifest) -> Result<(), String> {
        let pending = self.pending.as_mut().ok_or("no trusted request pending")?;
        if signed.manifest.request_id != pending.request_id
            || signed.manifest.operation != pending.operation
            || signed.manifest.client_ephemeral_public
                != PublicKey::from(&pending.ephemeral).to_bytes()
        {
            return Err("host challenge does not match the requested transaction".into());
        }
        let verifying = VerifyingKey::from_bytes(&signed.host_identity_public)
            .map_err(|_| "invalid host identity key")?;
        let signature = Signature::from_slice(&signed.signature)
            .map_err(|_| "invalid host signature encoding")?;
        verifying
            .verify(
                &signed
                    .manifest
                    .canonical_bytes()
                    .map_err(|error| error.to_string())?,
                &signature,
            )
            .map_err(|_| "host manifest signature is invalid")?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(signed.host_identity_public);
        pending.pinned = match self.pins.get(agent) {
            Some(existing) if existing == &encoded => true,
            Some(_) => return Err("HOST IDENTITY CHANGED; explicit re-pairing is required".into()),
            None => false,
        };
        self.status = if pending.pinned {
            "Verified host identity. Review exact fields and press a to approve.".into()
        } else {
            format!(
                "UNPAIRED HOST {}. Verify out of band, then press p to pin.",
                fingerprint(&signed.host_identity_public)
            )
        };
        pending.challenge = Some(signed);
        Ok(())
    }

    pub fn pin_current(&mut self, agent: &str) -> Result<(), String> {
        let pending = self.pending.as_mut().ok_or("no manifest to pin")?;
        let challenge = pending.challenge.as_ref().ok_or("no manifest to pin")?;
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(challenge.host_identity_public);
        self.pins.insert(agent.to_owned(), encoded);
        if let Some(parent) = self.pins_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(
            &self.pins_path,
            serde_json::to_vec_pretty(&self.pins).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        pending.pinned = true;
        self.status = "Host identity pinned. Press a to approve exact transaction.".into();
        Ok(())
    }

    pub fn approve(&mut self) -> Result<shared::Message, String> {
        let pending = self.pending.as_mut().ok_or("no trusted request pending")?;
        if !pending.pinned {
            return Err("host identity is not pinned".into());
        }
        let challenge = pending
            .challenge
            .clone()
            .ok_or("no host challenge received")?;
        let canonical = challenge
            .manifest
            .canonical_bytes()
            .map_err(|error| error.to_string())?;
        let signature = self.signer.sign(&canonical).to_bytes().to_vec();
        pending.transport = Some(ClientTransport::new(
            &pending.ephemeral,
            challenge.manifest.broker_transport_public,
            &canonical,
            &pending.request_id,
        )?);
        let payload = shared::trusted::encode_client(&TrustedClientFrame::Approve {
            signed_manifest: Box::new(challenge),
            approver_public: self.signer.verifying_key().to_bytes(),
            approver_signature: signature,
        })?;
        if matches!(pending.operation, TrustedOperation::RootPty { .. }) {
            self.mode = Mode::Terminal;
        }
        self.status = "Signed approval sent directly to host gate; root channel active.".into();
        Ok(shared::Message::TrustedOperationClient {
            request_id: pending.request_id.clone(),
            start: false,
            close: false,
            payload,
        })
    }

    pub fn encrypted_input(&mut self, bytes: &[u8]) -> Result<shared::Message, String> {
        let pending = self.pending.as_mut().ok_or("no active root session")?;
        let transport = pending
            .transport
            .as_mut()
            .ok_or("root session not approved")?;
        let (counter, data) = transport.encrypt(bytes)?;
        let payload =
            shared::trusted::encode_client(&TrustedClientFrame::Ciphertext { counter, data })?;
        Ok(shared::Message::TrustedOperationClient {
            request_id: pending.request_id.clone(),
            start: false,
            close: false,
            payload,
        })
    }

    pub fn host_frame(&mut self, frame: TrustedHostFrame) -> Result<(), String> {
        match frame {
            TrustedHostFrame::Ciphertext { counter, data } => {
                let pending = self.pending.as_mut().ok_or("unexpected root ciphertext")?;
                let transport = pending
                    .transport
                    .as_mut()
                    .ok_or("root session not approved")?;
                let plaintext = transport.decrypt(counter, &data)?;
                let event: TrustedPlaintext = serde_json::from_slice(&plaintext)
                    .map_err(|_| "invalid root plaintext frame")?;
                match event {
                    TrustedPlaintext::Output { stream, data } => {
                        let text = String::from_utf8_lossy(&data);
                        self.output.extend(text.lines().map(|line| {
                            if stream == "stderr" {
                                format!("! {line}")
                            } else {
                                line.to_owned()
                            }
                        }));
                    }
                    TrustedPlaintext::Exit { code, message } => {
                        self.status = format!("{message} (exit {code})");
                        self.mode = Mode::Fleet;
                    }
                }
            }
            TrustedHostFrame::Error { message } => return Err(message),
            TrustedHostFrame::Closed => {
                self.status = "Trusted root session closed.".into();
                self.mode = Mode::Fleet;
                self.pending = None;
            }
            TrustedHostFrame::Result { exit_code, message } => {
                self.status = format!("{message} ({exit_code})");
            }
            TrustedHostFrame::Challenge(_) => return Err("duplicate host challenge".into()),
        }
        Ok(())
    }
}

pub fn fingerprint(public: &[u8; 32]) -> String {
    let digest = Sha256::digest(public);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Serialize, Deserialize)]
struct _Pins(BTreeMap<String, String>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_uses_signed_operation_not_server_display_text() {
        let signer = SigningKey::from_bytes(&[9; 32]);
        let mut app = App::new(signer, PathBuf::from("/nonexistent/pins"));
        app.agents.push("host-a".into());
        let operation = TrustedOperation::RootPty {
            shell: "/bin/bash".into(),
            ttl_secs: 600,
            cols: 80,
            rows: 24,
        };
        app.begin(operation.clone()).unwrap();
        assert_eq!(app.pending.as_ref().unwrap().operation, operation);
    }
}
