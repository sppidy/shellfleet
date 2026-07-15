use argon2::Argon2;
use base64::Engine;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct EncryptedKey {
    version: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

pub fn default_key_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/shellfleet/approver.key"))
}

pub fn keygen(path: &Path, passphrase: &str) -> Result<SigningKey, String> {
    if passphrase.len() < 12 {
        return Err("approver key passphrase must be at least 12 characters".into());
    }
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let signing = SigningKey::from_bytes(&secret);
    write_encrypted(path, &secret, passphrase)?;
    Ok(signing)
}

pub fn load(path: &Path, passphrase: &str) -> Result<SigningKey, String> {
    let file: EncryptedKey = serde_json::from_slice(
        &std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|_| "invalid encrypted approver key file")?;
    if file.version != 1 {
        return Err("unsupported approver key-file version".into());
    }
    let salt = decode(&file.salt)?;
    let nonce = decode(&file.nonce)?;
    let ciphertext = decode(&file.ciphertext)?;
    let key = derive(passphrase, &salt)?;
    let nonce: [u8; 24] = nonce.try_into().map_err(|_| "invalid key-file nonce")?;
    let nonce = XNonce::from(nonce);
    let plaintext = XChaCha20Poly1305::new((&key).into())
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| "wrong passphrase or corrupted approver key")?;
    let secret: [u8; 32] = plaintext
        .try_into()
        .map_err(|_| "invalid approver key payload")?;
    Ok(SigningKey::from_bytes(&secret))
}

fn write_encrypted(path: &Path, secret: &[u8; 32], passphrase: &str) -> Result<(), String> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let key = derive(passphrase, &salt)?;
    let xnonce = XNonce::from(nonce);
    let ciphertext = XChaCha20Poly1305::new((&key).into())
        .encrypt(&xnonce, secret.as_ref())
        .map_err(|_| "failed to encrypt approver key")?;
    let file = EncryptedKey {
        version: 1,
        salt: encode(&salt),
        nonce: encode(&nonce),
        ciphertext: encode(&ciphertext),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(
        &mut options
            .open(&temporary)
            .map_err(|error| error.to_string())?,
        &serde_json::to_vec_pretty(&file).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn derive(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let mut output = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut output)
        .map_err(|_| "approver key derivation failed")?;
    Ok(output)
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(value: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| "invalid base64 in approver key file".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_key_roundtrip_and_wrong_passphrase_rejection() {
        let temp = std::env::temp_dir().join(format!("shellfleet-cli-key-{}", std::process::id()));
        let generated = keygen(&temp, "correct horse battery staple").unwrap();
        let loaded = load(&temp, "correct horse battery staple").unwrap();
        assert_eq!(generated.verifying_key(), loaded.verifying_key());
        assert!(load(&temp, "totally wrong passphrase").is_err());
        let _ = std::fs::remove_file(temp);
    }
}
