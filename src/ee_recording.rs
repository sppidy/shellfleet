//! Session-recording tap. When `EE_RECORD_TERMINALS` is enabled (and EE is
//! active), CE mirrors terminal frames to the EE sidecar's `/internal/recording/*`
//! pipeline — the piece the recording feature was missing (EE could ingest, but
//! nothing fed it). Best-effort by design: frames go through an unbounded channel
//! (non-blocking send on the terminal hot path) and a per-session drain task does
//! the HTTP, so recording can never stall or break a live terminal.

use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};

/// Recording is opt-in (default off) and only when the EE sidecar is active.
pub fn enabled() -> bool {
    crate::ee::ee_active()
        && matches!(
            std::env::var("EE_RECORD_TERMINALS")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
}

fn secret() -> String {
    std::env::var("EE_INTERNAL_SECRET").unwrap_or_default()
}

/// Standard base64 (the EE frame handler base64-decodes `data`). Inline to avoid
/// a new dependency.
fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[((n >> 18) & 63) as usize] as char);
        out.push(A[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            A[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// A terminal frame: (direction "i"/"o", bytes).
type Frame = (String, Vec<u8>);

#[derive(Default)]
pub struct Recorder {
    sessions: Mutex<HashMap<String, mpsc::UnboundedSender<Frame>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin recording a terminal session (idempotent per session_id). No-op
    /// when disabled or for the singleton container-exec session ("").
    pub async fn start(&self, session_id: &str, agent_id: &str, login: &str, session_type: &str) {
        if !enabled() || session_id.is_empty() {
            return;
        }
        let Some(url) = crate::ee::ee_sidecar_url() else {
            return;
        };
        let mut map = self.sessions.lock().await;
        if map.contains_key(session_id) {
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel::<(String, Vec<u8>)>();
        map.insert(session_id.to_string(), tx);
        drop(map);
        tokio::spawn(drain(
            url,
            session_id.to_string(),
            agent_id.to_string(),
            login.to_string(),
            session_type.to_string(),
            rx,
        ));
    }

    /// Mirror a terminal frame (`dir` = "i" input / "o" output). Non-blocking;
    /// dropped if the session isn't being recorded.
    pub async fn frame(&self, session_id: &str, dir: &str, data: &[u8]) {
        if session_id.is_empty() {
            return;
        }
        let map = self.sessions.lock().await;
        if let Some(tx) = map.get(session_id) {
            let _ = tx.send((dir.to_string(), data.to_vec()));
        }
    }

    /// End a session: drop the sender so the drain task flushes and posts stop.
    pub async fn stop(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }
}

async fn drain(
    url: String,
    session_id: String,
    agent_id: String,
    login: String,
    session_type: String,
    mut rx: mpsc::UnboundedReceiver<(String, Vec<u8>)>,
) {
    let base = url.trim_end_matches('/').to_string();
    let secret = secret();
    let client = reqwest::Client::new();
    let post = |path: String, body: serde_json::Value| {
        let c = client.clone();
        let s = secret.clone();
        async move {
            let _ = c
                .post(path)
                .bearer_auth(&s)
                .json(&body)
                .timeout(Duration::from_secs(5))
                .send()
                .await;
        }
    };

    post(
        format!("{base}/internal/recording/start"),
        serde_json::json!({ "session_id": session_id, "agent_id": agent_id, "login": login, "type": session_type }),
    )
    .await;

    while let Some((dir, data)) = rx.recv().await {
        post(
            format!("{base}/internal/recording/frame"),
            serde_json::json!({ "session_id": session_id, "direction": dir, "data": b64(&data) }),
        )
        .await;
    }

    post(
        format!("{base}/internal/recording/stop"),
        serde_json::json!({ "session_id": session_id }),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::b64;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foob"), "Zm9vYg==");
        assert_eq!(b64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }
}
