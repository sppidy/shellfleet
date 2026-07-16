use crate::credentials::Connection;
use futures_util::{SinkExt, StreamExt};
use shared::{
    UiMessage,
    fleet::{CoreEvent, FleetResponse},
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportState {
    Connecting,
    Live,
    Degraded(String),
}

#[derive(Debug)]
pub enum ClientEvent {
    Fleet(FleetResponse),
    Core(CoreEvent),
    WebSocket(Box<UiMessage>),
    DataState(TransportState),
    EventState(TransportState),
    WebSocketState(TransportState),
}

#[derive(Default)]
struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<CoreEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        self.buffer = self.buffer.replace("\r\n", "\n");
        let mut events = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let frame = self.buffer[..end].to_string();
            self.buffer.drain(..end + 2);
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty()
                && let Ok(event) = serde_json::from_str(&data)
            {
                events.push(event);
            }
        }
        events
    }
}

pub fn connect(
    connection: Connection,
) -> (
    mpsc::UnboundedSender<UiMessage>,
    mpsc::UnboundedReceiver<ClientEvent>,
) {
    let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
    let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
    let (refresh_tx, refresh_rx) = mpsc::channel(1);

    tokio::spawn(websocket_loop(
        connection.clone(),
        outgoing_rx,
        incoming_tx.clone(),
    ));
    tokio::spawn(fleet_refresh_loop(
        connection.clone(),
        refresh_rx,
        incoming_tx.clone(),
    ));
    tokio::spawn(sse_loop(connection, refresh_tx, incoming_tx));

    (outgoing_tx, incoming_rx)
}

async fn fleet_refresh_loop(
    connection: Connection,
    mut refresh: mpsc::Receiver<()>,
    incoming: mpsc::UnboundedSender<ClientEvent>,
) {
    let client = match reqwest::Client::builder()
        .https_only(connection.dashboard_url.starts_with("https://"))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            let _ = incoming.send(ClientEvent::DataState(TransportState::Degraded(
                error.to_string(),
            )));
            return;
        }
    };
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let mut last_refresh = tokio::time::Instant::now() - Duration::from_secs(10);
    loop {
        let requested = tokio::select! {
            _ = interval.tick() => true,
            value = refresh.recv() => value.is_some(),
        };
        if !requested {
            return;
        }
        let remaining = Duration::from_secs(10).saturating_sub(last_refresh.elapsed());
        if !remaining.is_zero() {
            tokio::time::sleep(remaining).await;
        }
        while refresh.try_recv().is_ok() {}
        let _ = incoming.send(ClientEvent::DataState(TransportState::Connecting));
        let url = format!("{}/api/core/v1/fleet", connection.dashboard_url);
        match client
            .get(url)
            .bearer_auth(&connection.access_token)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<FleetResponse>().await {
                    Ok(fleet) => {
                        let _ = incoming.send(ClientEvent::Fleet(fleet));
                        let _ = incoming.send(ClientEvent::DataState(TransportState::Live));
                    }
                    Err(error) => {
                        let _ = incoming.send(ClientEvent::DataState(TransportState::Degraded(
                            format!("invalid fleet response: {error}"),
                        )));
                    }
                }
            }
            Ok(response) => {
                let _ = incoming.send(ClientEvent::DataState(TransportState::Degraded(format!(
                    "fleet HTTP {}",
                    response.status()
                ))));
            }
            Err(error) => {
                let _ = incoming.send(ClientEvent::DataState(TransportState::Degraded(
                    error.to_string(),
                )));
            }
        }
        last_refresh = tokio::time::Instant::now();
    }
}

async fn sse_loop(
    connection: Connection,
    refresh: mpsc::Sender<()>,
    incoming: mpsc::UnboundedSender<ClientEvent>,
) {
    let client = match reqwest::Client::builder()
        .https_only(connection.dashboard_url.starts_with("https://"))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            let _ = incoming.send(ClientEvent::EventState(TransportState::Degraded(
                error.to_string(),
            )));
            return;
        }
    };
    let url = format!("{}/api/core/v1/events", connection.dashboard_url);
    loop {
        let _ = incoming.send(ClientEvent::EventState(TransportState::Connecting));
        let response = client
            .get(&url)
            .bearer_auth(&connection.access_token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let _ = incoming.send(ClientEvent::EventState(TransportState::Live));
                let mut decoder = SseDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => {
                            for event in decoder.push(&chunk) {
                                let _ = incoming.send(ClientEvent::Core(event));
                                let _ = refresh.try_send(());
                            }
                        }
                        Err(error) => {
                            let _ = incoming.send(ClientEvent::EventState(
                                TransportState::Degraded(error.to_string()),
                            ));
                            break;
                        }
                    }
                }
            }
            Ok(response) => {
                let _ = incoming.send(ClientEvent::EventState(TransportState::Degraded(format!(
                    "events HTTP {}",
                    response.status()
                ))));
            }
            Err(error) => {
                let _ = incoming.send(ClientEvent::EventState(TransportState::Degraded(
                    error.to_string(),
                )));
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn websocket_loop(
    connection: Connection,
    mut outgoing: mpsc::UnboundedReceiver<UiMessage>,
    incoming: mpsc::UnboundedSender<ClientEvent>,
) {
    loop {
        let _ = incoming.send(ClientEvent::WebSocketState(TransportState::Connecting));
        let request = websocket_request(&connection);
        let stream = match request {
            Ok(request) => connect_async(request).await,
            Err(error) => {
                let _ = incoming.send(ClientEvent::WebSocketState(TransportState::Degraded(error)));
                return;
            }
        };
        match stream {
            Ok((stream, _)) => {
                let _ = incoming.send(ClientEvent::WebSocketState(TransportState::Live));
                let (mut writer, mut reader) = stream.split();
                let list = serde_json::to_string(&UiMessage::ListAgentsRequest)
                    .expect("ListAgentsRequest is serializable");
                if writer.send(Message::Text(list.into())).await.is_err() {
                    continue;
                }
                loop {
                    tokio::select! {
                        outbound = outgoing.recv() => {
                            let Some(outbound) = outbound else { return; };
                            let Ok(json) = serde_json::to_string(&outbound) else { continue; };
                            if writer.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        frame = reader.next() => {
                            match frame {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(message) = serde_json::from_str(&text) {
                                        let _ = incoming.send(ClientEvent::WebSocket(Box::new(message)));
                                    }
                                }
                                Some(Ok(Message::Ping(payload))) => {
                                    match writer.send(Message::Pong(payload)).await {
                                        Ok(()) => {}
                                        Err(_) => break,
                                    }
                                }
                                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                                _ => {}
                            }
                        }
                    }
                }
                let _ = incoming.send(ClientEvent::WebSocketState(TransportState::Degraded(
                    "interactive channel disconnected".into(),
                )));
            }
            Err(error) => {
                let _ = incoming.send(ClientEvent::WebSocketState(TransportState::Degraded(
                    error.to_string(),
                )));
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn websocket_request(
    connection: &Connection,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
    let mut request = connection
        .ws_url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("invalid WebSocket URL: {error}"))?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", connection.access_token)
            .parse()
            .map_err(|_| "invalid authentication token")?,
    );
    request.headers_mut().insert(
        "x-shellfleet-cli",
        "1".parse().map_err(|_| "invalid CLI marker")?,
    );
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::SseDecoder;

    #[test]
    fn sse_decoder_accepts_fragmented_events() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"id: 1\nevent: fleet\ndata: {\"id\":1,")
                .is_empty()
        );
        let events =
            decoder.push(b"\"kind\":\"host_updated\",\"agent_id\":\"a\",\"observed_at\":9}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].agent_id.as_deref(), Some("a"));
    }

    #[test]
    fn sse_decoder_accepts_crlf_and_multiple_events() {
        let mut decoder = SseDecoder::default();
        let events = decoder.push(
            b"event: fleet\r\ndata: {\"id\":1,\"kind\":\"host_connected\",\"agent_id\":null,\"observed_at\":1}\r\n\r\nevent: fleet\ndata: {\"id\":2,\"kind\":\"host_disconnected\",\"agent_id\":\"b\",\"observed_at\":2}\n\n",
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].id, 2);
    }
}
