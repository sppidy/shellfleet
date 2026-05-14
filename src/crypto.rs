//! At-rest encryption for sensitive `users` columns.
//!
//! Threat model: a backup leak that doesn't include the live env. The
//! attacker has `shellfleet.db` but not `JWT_SECRET`. Without
//! encryption they could read TOTP secrets directly and bypass 2FA;
//! with encryption the column ciphertext is opaque without the key.
//!
//! Key derivation: `SHA-256("shellfleet-aead-v1" || JWT_SECRET)`.
//! JWT_SECRET is required to be ≥ 32 chars of random hex (enforced by
//! `auth::assert_jwt_secret_present`), so the input is high-entropy
//! and a single round of SHA-256 produces a uniformly-distributed
//! 256-bit key. No need for HKDF here.
//!
//! Format on disk: `v1:<base64-no-pad nonce>.<base64-no-pad ciphertext>`.
//! The `v1:` prefix is so a future key rotation can be staged
//! (decrypt v1, re-encrypt v2). Plaintexts that don't carry a `v1:`
//! prefix are rejected by `decrypt` — there is no transparent
//! fallback to plaintext.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use data_encoding::BASE64URL_NOPAD;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::env;

const PREFIX: &str = "v1:";
const NONCE_LEN: usize = 12; // AES-GCM standard

fn key() -> Key<Aes256Gcm> {
    let secret = env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let mut h = Sha256::new();
    h.update(b"shellfleet-aead-v1");
    h.update(secret.as_bytes());
    // Sha256::finalize() returns exactly 32 bytes, which is also the
    // key size for Aes256Gcm — convert through a fixed-size array
    // (avoids the deprecated GenericArray::from_slice call from the
    // generic-array 0.14 era).
    let digest: [u8; 32] = h.finalize().into();
    Key::<Aes256Gcm>::from(digest)
}

pub fn encrypt(plaintext: &str) -> String {
    let cipher = Aes256Gcm::new(&key());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .expect("aes-gcm encrypt should not fail with a valid key");
    format!(
        "{PREFIX}{}.{}",
        BASE64URL_NOPAD.encode(&nonce_bytes),
        BASE64URL_NOPAD.encode(&ct),
    )
}

/// Returns the plaintext on success. Returns `None` for any decoding
/// or authentication failure — callers should treat that as
/// equivalent to "the secret has been tampered with" and refuse to
/// authenticate the user.
pub fn decrypt(ciphertext: &str) -> Option<String> {
    let rest = ciphertext.strip_prefix(PREFIX)?;
    let (nonce_b64, ct_b64) = rest.split_once('.')?;
    let nonce_bytes = BASE64URL_NOPAD.decode(nonce_b64.as_bytes()).ok()?;
    let ct_bytes = BASE64URL_NOPAD.decode(ct_b64.as_bytes()).ok()?;
    if nonce_bytes.len() != NONCE_LEN {
        return None;
    }
    let cipher = Aes256Gcm::new(&key());
    // Length was validated above, so this conversion is infallible.
    let nonce_arr: [u8; NONCE_LEN] = nonce_bytes.as_slice().try_into().ok()?;
    let nonce = Nonce::from(nonce_arr);
    let pt = cipher.decrypt(&nonce, ct_bytes.as_ref()).ok()?;
    String::from_utf8(pt).ok()
}
