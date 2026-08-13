use crate::credentials::Connection;
use futures_util::StreamExt;
use shared::fleet::{CoreEvent, FleetResponse};
use std::time::Duration;
use tokio::sync::mpsc;

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
    DataState(TransportState),
    EventState(TransportState),
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

pub fn connect(connection: Connection) -> mpsc::UnboundedReceiver<ClientEvent> {
    let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
    let (refresh_tx, refresh_rx) = mpsc::channel(1);

    tokio::spawn(fleet_refresh_loop(
        connection.clone(),
        refresh_rx,
        incoming_tx.clone(),
    ));
    tokio::spawn(sse_loop(connection, refresh_tx, incoming_tx));

    incoming_rx
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
