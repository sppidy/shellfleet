use std::collections::BTreeMap;
use std::fmt;

use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroizing;

pub const HEADER_KEY_ID: &str = "x-shellfleet-key-id";
pub const HEADER_TIMESTAMP: &str = "x-shellfleet-timestamp";
pub const HEADER_NONCE: &str = "x-shellfleet-nonce";
pub const HEADER_SIGNATURE: &str = "x-shellfleet-signature";
pub const HEADER_RESPONSE_SIGNATURE: &str = "x-shellfleet-response-signature";
pub const MAX_TIMESTAMP_SKEW_SECS: u64 = 120;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    CeToEe,
    EeToCe,
}

impl Direction {
    fn label(self) -> &'static str {
        match self {
            Self::CeToEe => "ce-to-ee",
            Self::EeToCe => "ee-to-ce",
        }
    }
}

#[derive(Clone)]
pub struct InternalKey {
    pub id: String,
    secret: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for InternalKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InternalKey")
            .field("id", &self.id)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl InternalKey {
    pub fn new(id: impl Into<String>, secret: impl Into<Vec<u8>>) -> Result<Self, AuthError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(AuthError::InvalidKeyId);
        }
        let secret = secret.into();
        if secret.len() < 32 {
            return Err(AuthError::ShortKey);
        }
        Ok(Self {
            id,
            secret: Zeroizing::new(secret),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedRequest {
    pub key_id: String,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
    pub direction: Direction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedResponse {
    pub key_id: String,
    pub request_nonce: String,
    pub signature: String,
    pub direction: Direction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    InvalidKeyId,
    ShortKey,
    InvalidKeyring,
    UnknownKey,
    InvalidNonce,
    StaleTimestamp,
    WrongDirection,
    InvalidSignature,
    InvalidField,
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidKeyId => "invalid key id",
            Self::ShortKey => "internal auth keys must be at least 32 bytes",
            Self::InvalidKeyring => "invalid internal auth keyring",
            Self::UnknownKey => "unknown internal auth key id",
            Self::InvalidNonce => "invalid internal auth nonce",
            Self::StaleTimestamp => "internal auth timestamp outside allowed skew",
            Self::WrongDirection => "internal auth direction mismatch",
            Self::InvalidSignature => "invalid internal auth signature",
            Self::InvalidField => "internal auth field exceeds canonical encoding limits",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for AuthError {}

fn append_field(output: &mut Vec<u8>, field: &[u8]) -> Result<(), AuthError> {
    let length = u32::try_from(field.len()).map_err(|_| AuthError::InvalidField)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(field);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn canonical_request(
    direction: Direction,
    key_id: &str,
    timestamp: i64,
    nonce: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
    content_type: &str,
    login: &str,
    role: &str,
) -> Result<Vec<u8>, AuthError> {
    let mut output = Vec::with_capacity(body.len().min(1024) + 256);
    for field in [
        b"shellfleet-internal-request-v1".as_slice(),
        direction.label().as_bytes(),
        key_id.as_bytes(),
        timestamp.to_string().as_bytes(),
        nonce.as_bytes(),
        method.as_bytes(),
        path_and_query.as_bytes(),
        body,
        content_type.as_bytes(),
        login.as_bytes(),
        role.as_bytes(),
    ] {
        append_field(&mut output, field)?;
    }
    Ok(output)
}

fn canonical_response(
    direction: Direction,
    key_id: &str,
    request_nonce: &str,
    status: u16,
    body: &[u8],
) -> Result<Vec<u8>, AuthError> {
    let mut output = Vec::with_capacity(body.len().min(1024) + 160);
    for field in [
        b"shellfleet-internal-response-v1".as_slice(),
        direction.label().as_bytes(),
        key_id.as_bytes(),
        request_nonce.as_bytes(),
        status.to_string().as_bytes(),
        body,
    ] {
        append_field(&mut output, field)?;
    }
    Ok(output)
}

fn signature(key: &InternalKey, canonical: &[u8]) -> Result<String, AuthError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&key.secret)
        .map_err(|_| AuthError::InvalidSignature)?;
    mac.update(canonical);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn verify_signature(key: &InternalKey, canonical: &[u8], encoded: &str) -> Result<(), AuthError> {
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| AuthError::InvalidSignature)?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&key.secret)
        .map_err(|_| AuthError::InvalidSignature)?;
    mac.update(canonical);
    mac.verify_slice(&signature)
        .map_err(|_| AuthError::InvalidSignature)
}

fn find_key<'a>(keys: &'a [InternalKey], id: &str) -> Result<&'a InternalKey, AuthError> {
    keys.iter()
        .find(|key| key.id == id)
        .ok_or(AuthError::UnknownKey)
}

fn validate_nonce(encoded: &str) -> Result<(), AuthError> {
    let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| AuthError::InvalidNonce)?;
    if nonce.len() == 32 {
        Ok(())
    } else {
        Err(AuthError::InvalidNonce)
    }
}

pub fn new_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

pub fn parse_keyring(json: &str) -> Result<Vec<InternalKey>, AuthError> {
    let encoded: BTreeMap<String, String> =
        serde_json::from_str(json).map_err(|_| AuthError::InvalidKeyring)?;
    if encoded.is_empty() {
        return Err(AuthError::InvalidKeyring);
    }
    encoded
        .into_iter()
        .map(|(id, secret)| {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&secret)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&secret))
                .map_err(|_| AuthError::InvalidKeyring)?;
            InternalKey::new(id, decoded)
        })
        .collect()
}

pub fn active_key<'a>(keys: &'a [InternalKey], id: &str) -> Result<&'a InternalKey, AuthError> {
    find_key(keys, id)
}

#[allow(clippy::too_many_arguments)]
pub fn sign_request(
    key: &InternalKey,
    direction: Direction,
    timestamp: i64,
    nonce: &[u8; 32],
    method: &str,
    path_and_query: &str,
    body: &[u8],
    content_type: &str,
    login: &str,
    role: &str,
) -> Result<SignedRequest, AuthError> {
    let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce);
    let canonical = canonical_request(
        direction,
        &key.id,
        timestamp,
        &nonce,
        method,
        path_and_query,
        body,
        content_type,
        login,
        role,
    )?;
    Ok(SignedRequest {
        key_id: key.id.clone(),
        timestamp,
        nonce,
        signature: signature(key, &canonical)?,
        direction,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn verify_request(
    keys: &[InternalKey],
    direction: Direction,
    signed: &SignedRequest,
    now: i64,
    method: &str,
    path_and_query: &str,
    body: &[u8],
    content_type: &str,
    login: &str,
    role: &str,
) -> Result<(), AuthError> {
    if signed.direction != direction {
        return Err(AuthError::WrongDirection);
    }
    if now.abs_diff(signed.timestamp) > MAX_TIMESTAMP_SKEW_SECS {
        return Err(AuthError::StaleTimestamp);
    }
    validate_nonce(&signed.nonce)?;
    let key = find_key(keys, &signed.key_id)?;
    let canonical = canonical_request(
        direction,
        &signed.key_id,
        signed.timestamp,
        &signed.nonce,
        method,
        path_and_query,
        body,
        content_type,
        login,
        role,
    )?;
    verify_signature(key, &canonical, &signed.signature)
}

pub fn sign_response(
    key: &InternalKey,
    direction: Direction,
    request_nonce: &str,
    status: u16,
    body: &[u8],
) -> Result<SignedResponse, AuthError> {
    validate_nonce(request_nonce)?;
    let canonical = canonical_response(direction, &key.id, request_nonce, status, body)?;
    Ok(SignedResponse {
        key_id: key.id.clone(),
        request_nonce: request_nonce.to_string(),
        signature: signature(key, &canonical)?,
        direction,
    })
}

pub fn verify_response(
    keys: &[InternalKey],
    direction: Direction,
    signed: &SignedResponse,
    status: u16,
    body: &[u8],
) -> Result<(), AuthError> {
    if signed.direction != direction {
        return Err(AuthError::WrongDirection);
    }
    validate_nonce(&signed.request_nonce)?;
    let key = find_key(keys, &signed.key_id)?;
    let canonical = canonical_response(
        direction,
        &signed.key_id,
        &signed.request_nonce,
        status,
        body,
    )?;
    verify_signature(key, &canonical, &signed.signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, byte: u8) -> InternalKey {
        InternalKey::new(id, vec![byte; 32]).unwrap()
    }

    #[test]
    fn request_signature_binds_body_path_identity_and_direction() {
        let key = key("k1", 7);
        let signed = sign_request(
            &key,
            Direction::CeToEe,
            100,
            &[9; 32],
            "POST",
            "/api/ee/acl?x=1",
            br#"{"a":1}"#,
            "application/json",
            "alice",
            "admin",
        )
        .unwrap();

        assert!(
            verify_request(
                std::slice::from_ref(&key),
                Direction::CeToEe,
                &signed,
                100,
                "POST",
                "/api/ee/acl?x=1",
                br#"{"a":1}"#,
                "application/json",
                "alice",
                "admin",
            )
            .is_ok()
        );
        assert!(
            verify_request(
                std::slice::from_ref(&key),
                Direction::CeToEe,
                &signed,
                100,
                "POST",
                "/api/ee/acl?x=1",
                br#"{"a":2}"#,
                "application/json",
                "alice",
                "admin",
            )
            .is_err()
        );
        assert!(
            verify_request(
                &[key],
                Direction::EeToCe,
                &signed,
                100,
                "POST",
                "/api/ee/acl?x=1",
                br#"{"a":1}"#,
                "application/json",
                "alice",
                "admin",
            )
            .is_err()
        );
    }

    #[test]
    fn response_signature_binds_request_nonce_status_and_body() {
        let key = key("response", 11);
        let response = sign_response(
            &key,
            Direction::EeToCe,
            "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk",
            200,
            b"ok",
        )
        .unwrap();

        assert!(
            verify_response(
                std::slice::from_ref(&key),
                Direction::EeToCe,
                &response,
                200,
                b"ok"
            )
            .is_ok()
        );
        assert!(
            verify_response(
                std::slice::from_ref(&key),
                Direction::EeToCe,
                &response,
                403,
                b"ok"
            )
            .is_err()
        );
        assert!(verify_response(&[key], Direction::EeToCe, &response, 200, b"changed").is_err());
    }

    #[test]
    fn request_rejects_stale_timestamp_and_malformed_nonce() {
        let key = key("k1", 7);
        let mut signed = sign_request(
            &key,
            Direction::CeToEe,
            100,
            &[9; 32],
            "GET",
            "/api/ee/audit",
            b"",
            "",
            "",
            "",
        )
        .unwrap();
        assert_eq!(
            verify_request(
                std::slice::from_ref(&key),
                Direction::CeToEe,
                &signed,
                221,
                "GET",
                "/api/ee/audit",
                b"",
                "",
                "",
                "",
            ),
            Err(AuthError::StaleTimestamp)
        );
        signed.nonce = "short".into();
        assert_eq!(
            verify_request(
                &[key],
                Direction::CeToEe,
                &signed,
                100,
                "GET",
                "/api/ee/audit",
                b"",
                "",
                "",
                "",
            ),
            Err(AuthError::InvalidNonce)
        );
    }

    #[test]
    fn keyring_requires_named_256_bit_secrets_and_redacts_debug() {
        let encoded = base64::engine::general_purpose::STANDARD.encode([3u8; 32]);
        let keys = parse_keyring(&format!(r#"{{"active":"{encoded}"}}"#)).unwrap();
        assert_eq!(keys[0].id, "active");
        assert!(!format!("{:?}", keys[0]).contains(&encoded));
        assert!(matches!(
            parse_keyring(r#"{"bad":"YQ=="}"#),
            Err(AuthError::ShortKey)
        ));
        assert!(matches!(
            parse_keyring("{}"),
            Err(AuthError::InvalidKeyring)
        ));
    }
}
