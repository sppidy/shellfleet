use futures_util::{SinkExt, StreamExt};
use shared::UiMessage;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};

pub async fn connect(
    url: &str,
    auth_token: &str,
) -> Result<
    (
        mpsc::UnboundedSender<UiMessage>,
        mpsc::UnboundedReceiver<UiMessage>,
    ),
    String,
> {
    let mut request = url
        .into_client_request()
        .map_err(|error| format!("invalid WebSocket URL: {error}"))?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {auth_token}")
            .parse()
            .map_err(|_| "invalid authentication token")?,
    );
    request.headers_mut().insert(
        "x-shellfleet-cli",
        "1".parse().map_err(|_| "invalid CLI marker")?,
    );
    let (stream, _) = connect_async(request)
        .await
        .map_err(|error| format!("connect Operator Cockpit: {error}"))?;
    let (mut writer, mut reader) = stream.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (incoming_tx, incoming_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<Message>();
    let wire_from_ui = wire_tx.clone();
    tokio::spawn(async move {
        while let Some(message) = outgoing_rx.recv().await {
            let Ok(json) = serde_json::to_string(&message) else {
                continue;
            };
            if wire_from_ui.send(Message::Text(json.into())).is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(message) = wire_rx.recv().await {
            if writer.send(message).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(frame) = reader.next().await {
            match frame {
                Ok(Message::Text(text)) => {
                    if let Ok(message) = serde_json::from_str(&text) {
                        let _ = incoming_tx.send(message);
                    }
                }
                Ok(Message::Ping(payload)) => {
                    let _ = wire_tx.send(Message::Pong(payload));
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });
    Ok((outgoing_tx, incoming_rx))
}
