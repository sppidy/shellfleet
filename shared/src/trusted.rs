use serde::{Deserialize, Serialize};

pub const TRUSTED_PROTOCOL_VERSION: u32 = 1;
pub const MAX_TRUSTED_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrustedOperation {
    RootCommand {
        program: String,
        args: Vec<String>,
        timeout_secs: u32,
    },
    RootPty {
        shell: String,
        ttl_secs: u32,
        cols: u16,
        rows: u16,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TrustedManifest {
    pub version: u32,
    pub request_id: String,
    pub host_id: String,
    pub operation: TrustedOperation,
    pub client_ephemeral_public: [u8; 32],
    pub broker_transport_public: [u8; 32],
    pub nonce: [u8; 32],
    pub created_at: i64,
    pub expires_at: i64,
    pub policy_version: String,
}

impl TrustedManifest {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignedTrustedManifest {
    pub manifest: TrustedManifest,
    pub host_identity_public: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum TrustedClientFrame {
    Start {
        request_id: String,
        operation: TrustedOperation,
        client_ephemeral_public: [u8; 32],
    },
    Approve {
        signed_manifest: Box<SignedTrustedManifest>,
        approver_public: [u8; 32],
        approver_signature: Vec<u8>,
    },
    Ciphertext {
        counter: u64,
        data: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Close,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum TrustedHostFrame {
    Challenge(Box<SignedTrustedManifest>),
    Ciphertext { counter: u64, data: Vec<u8> },
    Result { exit_code: i32, message: String },
    Error { message: String },
    Closed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum TrustedPlaintext {
    Output { stream: String, data: Vec<u8> },
    Exit { code: i32, message: String },
}

/// Local Unix-socket protocol between the unprivileged network agent and the
/// root-owned terminal broker. These frames never cross the public network;
/// the server continues to authorize and audit the browser's terminal session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RootTerminalClientFrame {
    Start {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Input {
        data: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Close,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RootTerminalHostFrame {
    Output { data: Vec<u8> },
    Exit { code: i32 },
    Error { message: String },
}

pub fn encode_client(frame: &TrustedClientFrame) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(frame).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_TRUSTED_FRAME_BYTES {
        return Err("trusted client frame too large".into());
    }
    Ok(encoded)
}

pub fn decode_client(bytes: &[u8]) -> Result<TrustedClientFrame, String> {
    if bytes.len() > MAX_TRUSTED_FRAME_BYTES {
        return Err("trusted client frame too large".into());
    }
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

pub fn encode_host(frame: &TrustedHostFrame) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(frame).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_TRUSTED_FRAME_BYTES {
        return Err("trusted host frame too large".into());
    }
    Ok(encoded)
}

pub fn decode_host(bytes: &[u8]) -> Result<TrustedHostFrame, String> {
    if bytes.len() > MAX_TRUSTED_FRAME_BYTES {
        return Err("trusted host frame too large".into());
    }
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

pub fn encode_root_terminal_client(frame: &RootTerminalClientFrame) -> Result<Vec<u8>, String> {
    encode_bounded(frame)
}

pub fn decode_root_terminal_client(bytes: &[u8]) -> Result<RootTerminalClientFrame, String> {
    decode_bounded(bytes)
}

pub fn encode_root_terminal_host(frame: &RootTerminalHostFrame) -> Result<Vec<u8>, String> {
    encode_bounded(frame)
}

pub fn decode_root_terminal_host(bytes: &[u8]) -> Result<RootTerminalHostFrame, String> {
    decode_bounded(bytes)
}

fn encode_bounded<T: Serialize>(frame: &T) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(frame).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_TRUSTED_FRAME_BYTES {
        return Err("trusted frame too large".into());
    }
    Ok(encoded)
}

fn decode_bounded<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > MAX_TRUSTED_FRAME_BYTES {
        return Err("trusted frame too large".into());
    }
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_manifest_binds_operation_transport_and_expiry() {
        let manifest = TrustedManifest {
            version: TRUSTED_PROTOCOL_VERSION,
            request_id: "r1".into(),
            host_id: "h1".into(),
            operation: TrustedOperation::RootCommand {
                program: "/usr/bin/id".into(),
                args: vec!["-u".into()],
                timeout_secs: 10,
            },
            client_ephemeral_public: [1; 32],
            broker_transport_public: [2; 32],
            nonce: [3; 32],
            created_at: 100,
            expires_at: 160,
            policy_version: "p1".into(),
        };
        let original = manifest.canonical_bytes().unwrap();
        let mut changed = manifest.clone();
        changed.expires_at += 1;
        assert_ne!(original, changed.canonical_bytes().unwrap());
        let frame = TrustedHostFrame::Challenge(Box::new(SignedTrustedManifest {
            manifest,
            host_identity_public: [4; 32],
            signature: vec![5; 64],
        }));
        assert_eq!(decode_host(&encode_host(&frame).unwrap()).unwrap(), frame);
    }

    #[test]
    fn oversized_relay_frame_is_rejected_before_deserialization() {
        assert!(decode_client(&vec![0; MAX_TRUSTED_FRAME_BYTES + 1]).is_err());
    }

    #[test]
    fn local_root_terminal_frames_are_bounded_and_roundtrip() {
        let start = RootTerminalClientFrame::Start {
            session_id: "browser-session".into(),
            cols: 120,
            rows: 40,
        };
        assert_eq!(
            decode_root_terminal_client(&encode_root_terminal_client(&start).unwrap()).unwrap(),
            start
        );

        let output = RootTerminalHostFrame::Output {
            data: b"root prompt".to_vec(),
        };
        assert_eq!(
            decode_root_terminal_host(&encode_root_terminal_host(&output).unwrap()).unwrap(),
            output
        );
        assert!(decode_root_terminal_client(&vec![0; MAX_TRUSTED_FRAME_BYTES + 1]).is_err());
    }
}
