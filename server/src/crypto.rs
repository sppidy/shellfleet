//! At-rest encryption for sensitive `users` columns.
//!
//! Threat model: a backup leak that doesn't include the live env. The
//! attacker has `shellfleet.db` but not `JWT_SECRET`. Without
//! encryption they could read TOTP secrets directly and bypass 2FA;
//! with encryption the column ciphertext is opaque without the key.
//!
//! New ciphertexts derive their key with HKDF-SHA256 from the dedicated
//! `TOTP_ENCRYPTION_KEY`, rather than the signing key used for JWTs. That lets
//! operators rotate JWTs without invalidating TOTP enrollment. Existing v1
//! ciphertext remains readable with the historical JWT-derived key, so the
//! migration happens naturally when a secret is next written.
//!
//! Format on disk: `v2:<base64-no-pad nonce>.<base64-no-pad ciphertext>`.
//! Plaintexts that don't carry a recognized version prefix are rejected; there
//! is no transparent fallback to plaintext.

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use data_encoding::BASE64URL_NOPAD;
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::env;

const PREFIX_V1: &str = "v1:";
const PREFIX_V2: &str = "v2:";
const NONCE_LEN: usize = 12; // AES-GCM standard

fn legacy_key() -> Option<Key<Aes256Gcm>> {
    let secret = env::var("JWT_SECRET").ok()?;
    let mut h = Sha256::new();
    h.update(b"shellfleet-aead-v1");
    h.update(secret.as_bytes());
    // Sha256::finalize() returns exactly 32 bytes, which is also the
    // key size for Aes256Gcm — convert through a fixed-size array
    // (avoids the deprecated GenericArray::from_slice call from the
    // generic-array 0.14 era).
    let digest: [u8; 32] = h.finalize().into();
    Some(Key::<Aes256Gcm>::from(digest))
}

fn current_key() -> Result<Key<Aes256Gcm>, String> {
    let secret = env::var("TOTP_ENCRYPTION_KEY").map_err(|_| {
        "TOTP_ENCRYPTION_KEY is required to encrypt TOTP credentials".to_string()
    })?;
    if secret.len() < 32 {
        return Err("TOTP_ENCRYPTION_KEY must be at least 32 characters".to_string());
    }
    let hkdf = Hkdf::<Sha256>::new(Some(b"shellfleet-totp-aead-v2"), secret.as_bytes());
    let mut key_bytes = [0u8; 32];
    hkdf.expand(b"aes-256-gcm", &mut key_bytes)
        .map_err(|_| "failed to derive TOTP encryption key".to_string())?;
    Ok(Key::<Aes256Gcm>::from(key_bytes))
}

pub fn encrypt(plaintext: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new(&current_key()?);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| "failed to encrypt TOTP credential".to_string())?;
    Ok(format!(
        "{PREFIX_V2}{}.{}",
        BASE64URL_NOPAD.encode(&nonce_bytes),
        BASE64URL_NOPAD.encode(&ct),
    ))
}

/// Returns the plaintext on success. Returns `None` for any decoding
/// or authentication failure — callers should treat that as
/// equivalent to "the secret has been tampered with" and refuse to
/// authenticate the user.
pub fn decrypt(ciphertext: &str) -> Option<String> {
    let (prefix, rest) = if let Some(rest) = ciphertext.strip_prefix(PREFIX_V2) {
        (PREFIX_V2, rest)
    } else {
        (PREFIX_V1, ciphertext.strip_prefix(PREFIX_V1)?)
    };
    let (nonce_b64, ct_b64) = rest.split_once('.')?;
    let nonce_bytes = BASE64URL_NOPAD.decode(nonce_b64.as_bytes()).ok()?;
    let ct_bytes = BASE64URL_NOPAD.decode(ct_b64.as_bytes()).ok()?;
    if nonce_bytes.len() != NONCE_LEN {
        return None;
    }
    let key = if prefix == PREFIX_V2 {
        current_key().ok()?
    } else {
        legacy_key()?
    };
    let cipher = Aes256Gcm::new(&key);
    // Length was validated above, so this conversion is infallible.
    let nonce_arr: [u8; NONCE_LEN] = nonce_bytes.as_slice().try_into().ok()?;
    let nonce = Nonce::from(nonce_arr);
    let pt = cipher.decrypt(&nonce, ct_bytes.as_ref()).ok()?;
    String::from_utf8(pt).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();

    /// Set deterministic legacy and current key material for the round-trip
    /// if the environment does not already provide it.
    fn ensure_secret() {
        INIT.call_once(|| {
            if env::var("JWT_SECRET").is_err() {
                // SAFETY: one-time test setup; no other thread is reading
                // JWT_SECRET at this point in the crypto tests.
                unsafe {
                    env::set_var(
                        "JWT_SECRET",
                        "test-jwt-secret-0123456789abcdef0123456789abcdef",
                    );
                }
            }
            if env::var("TOTP_ENCRYPTION_KEY").is_err() {
                // SAFETY: one-time test setup; no other thread is reading
                // TOTP_ENCRYPTION_KEY at this point in these tests.
                unsafe {
                    env::set_var(
                        "TOTP_ENCRYPTION_KEY",
                        "test-totp-encryption-key-0123456789abcdef0123456789abcdef",
                    );
                }
            }
        });
    }

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        ensure_secret();
        let pt = "hunter2 · the TOTP secret";
        let ct = encrypt(pt).expect("test key material is configured");
        assert!(
            ct.starts_with(PREFIX_V2),
            "ciphertext must carry the v2 prefix"
        );
        assert_ne!(ct, pt);
        assert_eq!(decrypt(&ct).as_deref(), Some(pt));
    }

    #[test]
    fn decrypt_rejects_garbage_and_tampering() {
        ensure_secret();
        assert_eq!(decrypt("no-prefix-here"), None);
        assert_eq!(decrypt("v1:bogus.bogus"), None);
        // Flip the final ciphertext char: AEAD authentication must fail.
        let ct = encrypt("secret").expect("test key material is configured");
        let mut chars: Vec<char> = ct.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert_eq!(
            decrypt(&tampered),
            None,
            "tampered ciphertext must not decrypt"
        );
    }
}
