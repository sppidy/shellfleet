use crate::Outgoing;
use shared::Message;
use tokio::sync::mpsc;

pub const DEFAULT_GATE_SOCKET: &str = "/run/shellfleet/approval-gate.sock";

pub struct RelaySession {
    sender: mpsc::UnboundedSender<Vec<u8>>,
}

pub struct RootTerminalSession {
    pub tx_input: mpsc::UnboundedSender<Vec<u8>>,
    pub tx_resize: mpsc::UnboundedSender<(u16, u16)>,
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

/// Open the browser's host terminal through the root-owned local broker.
/// Network authorization and audit happen on the server; the broker still
/// verifies that its Unix-socket peer is the dedicated `shellfleet` uid.
pub async fn connect_root_terminal(
    session_id: String,
    outgoing: Outgoing,
) -> Result<RootTerminalSession, String> {
    let socket =
        std::env::var("SHELLFLEET_GATE_SOCKET").unwrap_or_else(|_| DEFAULT_GATE_SOCKET.to_string());
    let stream = tokio::net::UnixStream::connect(&socket)
        .await
        .map_err(|error| format!("connect root terminal broker: {error}"))?;
    let (mut reader, mut writer) = stream.into_split();
    let start = shared::trusted::RootTerminalClientFrame::Start {
        session_id: session_id.clone(),
        cols: 80,
        rows: 24,
    };
    super::framing::write_frame(
        &mut writer,
        &shared::trusted::encode_root_terminal_client(&start)?,
    )
    .await?;

    let (tx_input, mut rx_input) = mpsc::unbounded_channel::<Vec<u8>>();
    let (tx_resize, mut rx_resize) = mpsc::unbounded_channel::<(u16, u16)>();
    tokio::spawn(async move {
        loop {
            let frame = tokio::select! {
                input = rx_input.recv() => match input {
                    Some(data) => shared::trusted::RootTerminalClientFrame::Input { data },
                    None => shared::trusted::RootTerminalClientFrame::Close,
                },
                resize = rx_resize.recv() => match resize {
                    Some((cols, rows)) => shared::trusted::RootTerminalClientFrame::Resize { cols, rows },
                    None => shared::trusted::RootTerminalClientFrame::Close,
                },
            };
            let close = matches!(frame, shared::trusted::RootTerminalClientFrame::Close);
            let encoded = match shared::trusted::encode_root_terminal_client(&frame) {
                Ok(encoded) => encoded,
                Err(_) => break,
            };
            if super::framing::write_frame(&mut writer, &encoded)
                .await
                .is_err()
                || close
            {
                break;
            }
        }
    });

    tokio::spawn(async move {
        loop {
            let payload = match super::framing::read_frame(&mut reader).await {
                Ok(payload) => payload,
                Err(_) => break,
            };
            match shared::trusted::decode_root_terminal_host(&payload) {
                Ok(shared::trusted::RootTerminalHostFrame::Output { data }) => {
                    let _ = outgoing.send(Message::TerminalData {
                        session_id: session_id.clone(),
                        data,
                    });
                }
                Ok(shared::trusted::RootTerminalHostFrame::Exit { code }) => {
                    let _ = outgoing.send(Message::TerminalData {
                        session_id: session_id.clone(),
                        data: format!("\r\n[root terminal exited: {code}]\r\n").into_bytes(),
                    });
                    break;
                }
                Ok(shared::trusted::RootTerminalHostFrame::Error { message }) => {
                    let _ = outgoing.send(Message::TerminalData {
                        session_id: session_id.clone(),
                        data: format!("\r\n[root terminal error: {message}]\r\n").into_bytes(),
                    });
                    break;
                }
                Err(_) => break,
            }
        }
    });

    Ok(RootTerminalSession {
        tx_input,
        tx_resize,
    })
}
