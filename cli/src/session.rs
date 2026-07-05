use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct ClientTransport {
    send: ChaCha20Poly1305,
    receive: ChaCha20Poly1305,
    send_counter: u64,
    receive_counter: u64,
    request_id: String,
}

impl ClientTransport {
    pub fn new(
        secret: &StaticSecret,
        broker_public: [u8; 32],
        manifest: &[u8],
        request_id: &str,
    ) -> Result<Self, String> {
        use chacha20poly1305::KeyInit;
        let shared = secret.diffie_hellman(&PublicKey::from(broker_public));
        let salt = Sha256::digest(manifest);
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes());
        let mut send = [0; 32];
        let mut receive = [0; 32];
        hkdf.expand(b"shellfleet-root-client-to-host-v1", &mut send)
            .map_err(|_| "transport key derivation failed")?;
        hkdf.expand(b"shellfleet-root-host-to-client-v1", &mut receive)
            .map_err(|_| "transport key derivation failed")?;
        Ok(Self {
            send: ChaCha20Poly1305::new((&send).into()),
            receive: ChaCha20Poly1305::new((&receive).into()),
            send_counter: 0,
            receive_counter: 0,
            request_id: request_id.into(),
        })
    }

    fn nonce(counter: u64) -> [u8; 12] {
        let mut nonce = [0; 12];
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        nonce
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<(u64, Vec<u8>), String> {
        let counter = self.send_counter;
        let nonce = Nonce::from(Self::nonce(counter));
        let data = self
            .send
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: self.request_id.as_bytes(),
                },
            )
            .map_err(|_| "root frame encryption failed")?;
        self.send_counter = self
            .send_counter
            .checked_add(1)
            .ok_or("counter exhausted")?;
        Ok((counter, data))
    }

    pub fn decrypt(&mut self, counter: u64, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        if counter != self.receive_counter {
            return Err("root frame replay or reordering detected".into());
        }
        let nonce = Nonce::from(Self::nonce(counter));
        let plaintext = self
            .receive
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: self.request_id.as_bytes(),
                },
            )
            .map_err(|_| "root frame authentication failed")?;
        self.receive_counter = self
            .receive_counter
            .checked_add(1)
            .ok_or("counter exhausted")?;
        Ok(plaintext)
    }
}
