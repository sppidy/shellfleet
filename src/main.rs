mod auth;
mod device_auth;
mod tokens;

use axum::{
    extract::{Query, State, ws::{Message as WsMessage, WebSocket, WebSocketUpgrade}},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_extra::extract::cookie::CookieJar;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use shared::{Message, UiMessage};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, Mutex, RwLock};
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub use tokens::{now_unix, TokenInfo};

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
    pub ui_clients: Mutex<HashMap<u64, UiTx>>,
    pub ui_id_counter: AtomicU64,
    pub pending_devices: RwLock<HashMap<String, PendingDevice>>,
    pub user_codes: RwLock<HashMap<String, String>>,
    pub approved_tokens: RwLock<HashMap<String, TokenInfo>>,
}

fn tokens_path() -> PathBuf {
    PathBuf::from(
        std::env::var("TOKENS_PATH").unwrap_or_else(|_| "/data/approved_tokens.json".to_string()),
    )
}

pub fn save_tokens(tokens: &HashMap<String, TokenInfo>) -> std::io::Result<()> {
    let path = tokens_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string(tokens)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

pub fn load_tokens() -> HashMap<String, TokenInfo> {
    let path = tokens_path();
    let read_one = |p: &std::path::Path| -> Option<HashMap<String, TokenInfo>> {
        let data = std::fs::read_to_string(p).ok()?;
        if let Ok(parsed) = serde_json::from_str::<HashMap<String, TokenInfo>>(&data) {
            return Some(parsed);
        }
        // Legacy schema: HashMap<String, bool>. Migrate.
        if let Ok(legacy) = serde_json::from_str::<HashMap<String, bool>>(&data) {
            tracing::info!("migrating legacy bool token store at {}", p.display());
            return Some(
                legacy
                    .into_iter()
                    .map(|(k, _)| (k, TokenInfo::default()))
                    .collect(),
            );
        }
        None
    };

    if let Some(t) = read_one(&path) {
        return t;
    }
    if let Some(t) = read_one(std::path::Path::new("approved_tokens.json")) {
        let _ = save_tokens(&t);
        return t;
    }
    HashMap::new()
}

#[derive(Deserialize)]
pub struct AgentAuth {
    pub token: String,
}

async fn broadcast_agent_list(state: &AppState) {
    let agents: Vec<String> = state.agents.lock().await.keys().cloned().collect();
    let msg = UiMessage::ListAgentsResponse { agents };
    let mut clients = state.ui_clients.lock().await;
    clients.retain(|_id, tx| tx.send(msg.clone()).is_ok());
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[derive(Serialize)]
struct MeResponse {
    user: String,
}

async fn me_handler(jar: CookieJar) -> impl IntoResponse {
    if std::env::var("JWT_SECRET").unwrap_or_default() == "dev" {
        return (StatusCode::OK, axum::Json(MeResponse { user: "dev".into() })).into_response();
    }
    if let Some(cookie) = jar.get("auth_token") {
        if let Some(user) = auth::user_from_token(cookie.value()) {
            return (StatusCode::OK, axum::Json(MeResponse { user })).into_response();
        }
    }
    (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
}

fn init_tracing() {
    // RUST_LOG=info,server=debug,tower_http=info gives nice request logs.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,server=info,tower_http=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false).with_level(true))
        .init();
}

#[tokio::main]
async fn main() {
    init_tracing();

    let state = Arc::new(AppState {
        agents: Mutex::new(HashMap::new()),
        ui_clients: Mutex::new(HashMap::new()),
        ui_id_counter: AtomicU64::new(0),
        pending_devices: RwLock::new(HashMap::new()),
        user_codes: RwLock::new(HashMap::new()),
        approved_tokens: RwLock::new(load_tokens()),
    });

    let api_routes = Router::new()
        .nest("/device", device_auth::routes())
        .nest("/tokens", tokens::routes())
        .route("/me", get(me_handler))
        .route("/healthz", get(healthz))
        .with_state(state.clone());

    let ws_routes = Router::new()
        .route("/agent/ws", get(agent_ws_handler))
        .route("/ui/ws", get(ui_ws_handler))
        .with_state(state);

    let trace_layer = TraceLayer::new_for_http()
        .on_response(DefaultOnResponse::new().level(Level::INFO))
        .on_failure(DefaultOnFailure::new().level(Level::WARN));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .nest("/auth", auth::auth_routes())
        .nest("/api", api_routes)
        .merge(ws_routes)
        .layer(trace_layer);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!(%addr, "server listening");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn agent_ws_handler(
    Query(auth): Query<AgentAuth>,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let is_approved = state.approved_tokens.read().await.contains_key(&auth.token);

    let legacy_token = std::env::var("AGENT_SECRET").unwrap_or_default();
    let legacy_match = !legacy_token.is_empty() && auth.token == legacy_token;

    if !legacy_match && !is_approved {
        tracing::warn!("unauthorized agent connection attempt");
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(move |socket| handle_agent_socket(socket, state, auth.token))
        .into_response()
}

async fn ui_ws_handler(
    jar: CookieJar,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let mut is_authenticated = false;

    if std::env::var("JWT_SECRET").unwrap_or_default() == "dev" {
        is_authenticated = true;
    } else if let Some(cookie) = jar.get("auth_token") {
        if auth::verify_token(cookie.value()) {
            is_authenticated = true;
        }
    }

    if !is_authenticated {
        tracing::warn!("unauthorized ui connection attempt");
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(|socket| handle_ui_socket(socket, state))
        .into_response()
}

async fn handle_agent_socket(socket: WebSocket, state: Arc<AppState>, token: String) {
    tracing::info!("new agent websocket connection");
    let (mut sender, mut receiver) = socket.split();

    let mut agent_id_opt: Option<String> = None;
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    let send_task = tokio::spawn(async move {
        // Heartbeat every 25s. Cloudflare/Nginx will drop an idle WebSocket
        // after ~100s on a free CF plan, so without this the connection
        // disappears 1-2 minutes after the last application message.
        let mut hb = tokio::time::interval(std::time::Duration::from_secs(25));
        hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Drop the immediate first tick so we don't ping before any message.
        hb.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => {
                        if let Ok(text) = serde_json::to_string(&msg) {
                            if sender.send(WsMessage::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => break,
                },
                _ = hb.tick() => {
                    if sender.send(WsMessage::Ping(Default::default())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(Ok(frame)) = receiver.next().await {
        let text = match frame {
            WsMessage::Text(t) => t,
            WsMessage::Close(_) => break,
            // Pong/Ping/Binary: keep the connection open and ignore the
            // frame. Without this branch the previous code dropped out of
            // the loop on the first Pong, killing the connection one
            // heartbeat after start.
            _ => continue,
        };
        if let Ok(parsed_msg) = serde_json::from_str::<Message>(&text) {
            match parsed_msg {
                Message::Register { hostname, protocol_version } => {
                    let id = format!("{}-id", hostname);
                    agent_id_opt = Some(id.clone());

                    state.agents.lock().await.insert(id.clone(), tx.clone());
                    tracing::info!(agent_id = %id, %protocol_version, "agent registered");

                    // Touch the token's last_seen + hostname so the operator
                    // can identify which agent owns each row in /api/tokens.
                    let mut tokens = state.approved_tokens.write().await;
                    let info = tokens.entry(token.clone()).or_default();
                    info.hostname = Some(hostname.clone());
                    info.last_seen = now_unix();
                    if info.created_at == 0 {
                        info.created_at = now_unix();
                    }
                    let snapshot = tokens.clone();
                    drop(tokens);
                    if let Err(e) = save_tokens(&snapshot) {
                        tracing::warn!(error = %e, "failed to persist token metadata");
                    }

                    let _ = tx.send(Message::RegisterAck { agent_id: id.clone() });

                    broadcast_agent_list(&state).await;
                }
                _ => {
                    if let Some(agent_id) = &agent_id_opt {
                        let ui_msg = UiMessage::AgentMessage {
                            agent_id: agent_id.clone(),
                            message: parsed_msg,
                        };
                        let mut clients = state.ui_clients.lock().await;
                        clients.retain(|_id, ctx| ctx.send(ui_msg.clone()).is_ok());
                    }
                }
            }
        }
    }

    send_task.abort();

    if let Some(id) = agent_id_opt {
        state.agents.lock().await.remove(&id);
        tracing::info!(agent_id = %id, "agent disconnected");
        broadcast_agent_list(&state).await;
    }
}

async fn handle_ui_socket(socket: WebSocket, state: Arc<AppState>) {
    tracing::info!("new ui websocket connection");
    let (mut sender, mut receiver) = socket.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<UiMessage>();
    let client_id = state.ui_id_counter.fetch_add(1, Ordering::Relaxed);
    state.ui_clients.lock().await.insert(client_id, tx.clone());

    {
        let agents: Vec<String> = state.agents.lock().await.keys().cloned().collect();
        let _ = tx.send(UiMessage::ListAgentsResponse { agents });
    }

    let send_task = tokio::spawn(async move {
        let mut hb = tokio::time::interval(std::time::Duration::from_secs(25));
        hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        hb.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => {
                        if let Ok(text) = serde_json::to_string(&msg) {
                            if sender.send(WsMessage::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => break,
                },
                _ = hb.tick() => {
                    if sender.send(WsMessage::Ping(Default::default())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(Ok(frame)) = receiver.next().await {
        let text = match frame {
            WsMessage::Text(t) => t,
            WsMessage::Close(_) => break,
            _ => continue,
        };
        if let Ok(parsed_msg) = serde_json::from_str::<UiMessage>(&text) {
            match parsed_msg {
                UiMessage::ListAgentsRequest => {
                    let agents: Vec<String> =
                        state.agents.lock().await.keys().cloned().collect();
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
    state.ui_clients.lock().await.remove(&client_id);
    tracing::info!(client_id, "ui client disconnected");
}
