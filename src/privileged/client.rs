use crate::Outgoing;
use shared::Message;
use tokio::sync::mpsc;

pub const DEFAULT_GATE_SOCKET: &str = "/run/shellfleet/approval-gate.sock";

pub struct RelaySession {
    sender: mpsc::UnboundedSender<Vec<u8>>,
}

impl RelaySession {
    pub fn send(&self, payload: Vec<u8>) -> Result<(), String> {
        if payload.is_empty() || payload.len() > shared::trusted::MAX_TRUSTED_FRAME_BYTES {
            return Err("trusted relay payload size is invalid".into());
        }
        self.sender
            .send(payload)
            .map_err(|_| "trusted host broker disconnected".into())
    }
}

pub async fn connect(
    request_id: String,
    first_payload: Vec<u8>,
    outgoing: Outgoing,
) -> Result<RelaySession, String> {
    let socket =
        std::env::var("SHELLFLEET_GATE_SOCKET").unwrap_or_else(|_| DEFAULT_GATE_SOCKET.to_string());
    let stream = tokio::net::UnixStream::connect(&socket)
        .await
        .map_err(|error| format!("connect approval gate: {error}"))?;
    let (mut reader, mut writer) = stream.into_split();
    let (sender, mut receiver) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(payload) = receiver.recv().await {
            if super::framing::write_frame(&mut writer, &payload)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let response_request_id = request_id.clone();
    tokio::spawn(async move {
        loop {
            match super::framing::read_frame(&mut reader).await {
                Ok(payload) => {
                    let complete = matches!(
                        shared::trusted::decode_host(&payload),
                        Ok(shared::trusted::TrustedHostFrame::Closed)
                            | Ok(shared::trusted::TrustedHostFrame::Error { .. })
                    );
                    let _ = outgoing.send(Message::TrustedOperationHost {
                        request_id: response_request_id.clone(),
                        complete,
                        payload,
                    });
                    if complete {
                        break;
                    }
                }
                Err(error) => {
                    let payload =
                        shared::trusted::encode_host(&shared::trusted::TrustedHostFrame::Error {
                            message: error,
                        })
                        .unwrap_or_default();
                    let _ = outgoing.send(Message::TrustedOperationHost {
                        request_id: response_request_id.clone(),
                        complete: true,
                        payload,
                    });
                    break;
                }
            }
        }
    });
    let session = RelaySession { sender };
    session.send(first_payload)?;
    Ok(session)
}
