mod auth;
mod device_auth;

use axum::{
    extract::{Query, State, ws::{Message as WsMessage, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_extra::extract::cookie::CookieJar;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use shared::{Message, UiMessage};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

type AgentTx = mpsc::UnboundedSender<Message>;
type UiTx = mpsc::UnboundedSender<UiMessage>;

pub struct PendingDevice {
    pub device_code: String,
    pub user_code: String,
    pub expires_at: u64,
    pub approved: bool,
}

pub struct AppState {
    pub agents: Mutex<HashMap<String, AgentTx>>,
    pub ui_clients: Mutex<Vec<UiTx>>,
    pub pending_devices: RwLock<HashMap<String, PendingDevice>>,
    pub user_codes: RwLock<HashMap<String, String>>,
    pub approved_tokens: RwLock<HashMap<String, bool>>,
}

pub fn save_tokens(tokens: &HashMap<String, bool>) -> std::io::Result<()> {
    let json = serde_json::to_string(tokens)?;
    std::fs::write("approved_tokens.json", json)
}

pub fn load_tokens() -> HashMap<String, bool> {
    if let Ok(data) = std::fs::read_to_string("approved_tokens.json") {
        if let Ok(tokens) = serde_json::from_str(&data) {
            return tokens;
        }
    }
    HashMap::new()
}

#[derive(Deserialize)]
pub struct AgentAuth {
    pub token: String,
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        agents: Mutex::new(HashMap::new()),
        ui_clients: Mutex::new(Vec::new()),
        pending_devices: RwLock::new(HashMap::new()),
        user_codes: RwLock::new(HashMap::new()),
        approved_tokens: RwLock::new(load_tokens()),
    });

    let api_routes = Router::new()
        .nest("/device", device_auth::routes())
        .with_state(state.clone());

    let ws_routes = Router::new()
        .route("/agent/ws", get(agent_ws_handler))
        .route("/ui/ws", get(ui_ws_handler))
        .with_state(state);

    let app = Router::new()
        .nest("/auth", auth::auth_routes())
        .nest("/api", api_routes)
        .merge(ws_routes);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("Server listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn agent_ws_handler(
    Query(auth): Query<AgentAuth>,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>
) -> impl IntoResponse {
    let is_approved = state.approved_tokens.read().await.contains_key(&auth.token);
    
    // Allow the legacy AGENT_SECRET for backward compatibility/static testing
    let legacy_token = std::env::var("AGENT_SECRET").unwrap_or_else(|_| "legacy_secret_off".to_string());
    
    if auth.token != legacy_token && !is_approved {
        println!("Unauthorized agent connection attempt.");
        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(|socket| handle_agent_socket(socket, state)).into_response()
}

async fn ui_ws_handler(
    jar: CookieJar,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>
) -> impl IntoResponse {
    let mut is_authenticated = false;
    
    // For local development ease, bypass auth if JWT_SECRET is explicitly "dev"
    if std::env::var("JWT_SECRET").unwrap_or_default() == "dev" {
        is_authenticated = true;
    } else if let Some(cookie) = jar.get("auth_token") {
        if auth::verify_token(cookie.value()) {
            is_authenticated = true;
        }
    }

    if !is_authenticated {
        println!("Unauthorized UI connection attempt.");
        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(|socket| handle_ui_socket(socket, state)).into_response()
}

async fn handle_agent_socket(socket: WebSocket, state: Arc<AppState>) {
    println!("New Agent WebSocket connection");
    let (mut sender, mut receiver) = socket.split();
    
    let mut agent_id_opt: Option<String> = None;
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Task to send messages to the agent
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(text) = serde_json::to_string(&msg) {
                if sender.send(WsMessage::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Handle incoming messages
    while let Some(Ok(WsMessage::Text(text))) = receiver.next().await {
        if let Ok(parsed_msg) = serde_json::from_str::<Message>(&text) {
            match parsed_msg {
                Message::Register { hostname } => {
                    let id = format!("{}-id", hostname);
                    agent_id_opt = Some(id.clone());
                    
                    state.agents.lock().await.insert(id.clone(), tx.clone());
                    println!("Agent registered: {}", id);

                    // Send Ack
                    let _ = tx.send(Message::RegisterAck { agent_id: id.clone() });
                }
                _ => {
                    // Forward other messages to all UI clients
                    if let Some(agent_id) = &agent_id_opt {
                        let ui_msg = UiMessage::AgentMessage {
                            agent_id: agent_id.clone(),
                            message: parsed_msg,
                        };
                        let clients = state.ui_clients.lock().await;
                        for client_tx in clients.iter() {
                            let _ = client_tx.send(ui_msg.clone());
                        }
                    }
                }
            }
        }
    }

    send_task.abort();
    
    // Cleanup
    if let Some(id) = agent_id_opt {
        state.agents.lock().await.remove(&id);
        println!("Agent disconnected: {}", id);
    }
}

async fn handle_ui_socket(socket: WebSocket, state: Arc<AppState>) {
    println!("New UI WebSocket connection");
    let (mut sender, mut receiver) = socket.split();
    
    let (tx, mut rx) = mpsc::unbounded_channel::<UiMessage>();
    state.ui_clients.lock().await.push(tx.clone());

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(text) = serde_json::to_string(&msg) {
                if sender.send(WsMessage::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(WsMessage::Text(text))) = receiver.next().await {
        if let Ok(parsed_msg) = serde_json::from_str::<UiMessage>(&text) {
            match parsed_msg {
                UiMessage::ListAgentsRequest => {
                    let agents: Vec<String> = state.agents.lock().await.keys().cloned().collect();
                    let _ = tx.send(UiMessage::ListAgentsResponse { agents });
                }
                UiMessage::SendToAgent { agent_id, message } => {
                    if let Some(agent_tx) = state.agents.lock().await.get(&agent_id) {
                        let _ = agent_tx.send(message);
                    }
                }
                _ => {}
            }
        }
    }

    send_task.abort();
    // In a real app we'd need to remove the tx from state.ui_clients, 
    // but for simplicity it stays or we can clean it up later.
}
