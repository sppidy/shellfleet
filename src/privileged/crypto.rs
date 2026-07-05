use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct Transport {
    send: ChaCha20Poly1305,
    receive: ChaCha20Poly1305,
    send_counter: u64,
    receive_counter: u64,
    request_id: String,
}

impl Transport {
    pub fn broker(
        secret: &StaticSecret,
        client_public: [u8; 32],
        manifest: &[u8],
        request_id: &str,
    ) -> Result<Self, String> {
        Self::derive(
            secret
                .diffie_hellman(&PublicKey::from(client_public))
                .as_bytes(),
            manifest,
            request_id,
            false,
        )
    }

    pub fn client(
        secret: &StaticSecret,
        broker_public: [u8; 32],
        manifest: &[u8],
        request_id: &str,
    ) -> Result<Self, String> {
        Self::derive(
            secret
                .diffie_hellman(&PublicKey::from(broker_public))
                .as_bytes(),
            manifest,
            request_id,
            true,
        )
    }

    fn derive(
        shared: &[u8; 32],
        manifest: &[u8],
        request_id: &str,
        client: bool,
    ) -> Result<Self, String> {
        use chacha20poly1305::KeyInit;
        let salt = Sha256::digest(manifest);
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared);
        let mut client_to_host = [0u8; 32];
        let mut host_to_client = [0u8; 32];
        hkdf.expand(b"shellfleet-root-client-to-host-v1", &mut client_to_host)
            .map_err(|_| "transport key derivation failed")?;
        hkdf.expand(b"shellfleet-root-host-to-client-v1", &mut host_to_client)
            .map_err(|_| "transport key derivation failed")?;
        let (send, receive) = if client {
            (client_to_host, host_to_client)
        } else {
            (host_to_client, client_to_host)
        };
        Ok(Self {
            send: ChaCha20Poly1305::new((&send).into()),
            receive: ChaCha20Poly1305::new((&receive).into()),
            send_counter: 0,
            receive_counter: 0,
            request_id: request_id.to_owned(),
        })
    }

    fn nonce(counter: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        nonce
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<(u64, Vec<u8>), String> {
        let counter = self.send_counter;
        let nonce = Self::nonce(counter);
        let nonce = Nonce::from(nonce);
        let data = self
            .send
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: self.request_id.as_bytes(),
                },
            )
            .map_err(|_| "root transport encryption failed")?;
        self.send_counter = self
            .send_counter
            .checked_add(1)
            .ok_or("root transport counter exhausted")?;
        Ok((counter, data))
    }

    pub fn decrypt(&mut self, counter: u64, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        if counter != self.receive_counter {
            return Err("root transport frame replay or reordering detected".into());
        }
        let nonce = Self::nonce(counter);
        let nonce = Nonce::from(nonce);
        let plaintext = self
            .receive
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: self.request_id.as_bytes(),
                },
            )
            .map_err(|_| "root transport authentication failed")?;
        self.receive_counter = self
            .receive_counter
            .checked_add(1)
            .ok_or("root transport counter exhausted")?;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_sees_only_ciphertext_and_cannot_replay_or_tamper() {
        let broker_secret = StaticSecret::from([7; 32]);
        let client_secret = StaticSecret::from([8; 32]);
        let mut broker = Transport::broker(
            &broker_secret,
            PublicKey::from(&client_secret).to_bytes(),
            b"manifest",
            "request",
        )
        .unwrap();
        let mut client = Transport::client(
            &client_secret,
            PublicKey::from(&broker_secret).to_bytes(),
            b"manifest",
            "request",
        )
        .unwrap();
        let marker = b"KNOWN-ROOT-PLAINTEXT";
        let (counter, ciphertext) = client.encrypt(marker).unwrap();
        assert!(
            !ciphertext
                .windows(marker.len())
                .any(|window| window == marker)
        );
        assert_eq!(broker.decrypt(counter, &ciphertext).unwrap(), marker);
        assert!(broker.decrypt(counter, &ciphertext).is_err());
    }
}
