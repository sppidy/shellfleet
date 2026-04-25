mod auth;
mod csrf;
mod db;
mod device_auth;
mod fan_out;
mod health;
mod notifications;
mod tokens;
mod update_windows;
mod webhook;

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
use sqlx::SqlitePool;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, Mutex};
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

type AgentTx = mpsc::UnboundedSender<Message>;
type UiTx = mpsc::UnboundedSender<UiMessage>;

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct AppState {
    pub agents: Mutex<HashMap<String, AgentTx>>,
    pub ui_clients: Mutex<HashMap<u64, UiTx>>,
    pub ui_id_counter: AtomicU64,
    pub db: SqlitePool,
    /// agent_ids currently expecting an `AptUpgradeResponse` that should
    /// be attributed to the auto-update scheduler (or `run_now` button)
    /// rather than a UI-driven upgrade. Cleared when the response lands.
    pub scheduled_apt_runs: Mutex<HashSet<String>>,
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

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<i64>,
}

async fn audit_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    if std::env::var("JWT_SECRET").unwrap_or_default() != "dev" {
        let cookie = match jar.get("auth_token") {
            Some(c) => c,
            None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
        };
        if !auth::verify_token(cookie.value()) {
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    match db::recent_audit(&state.db, limit).await {
        Ok(rows) => axum::Json(rows).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "audit query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
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

    let pool = match db::init().await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to initialise sqlite");
            std::process::exit(1);
        }
    };

    // GC expired pending device-auth requests every minute so we don't
    // accumulate stale rows.
    {
        let pool = pool.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                if let Ok(removed) = db::purge_expired_devices(&pool, now_unix()).await {
                    if removed > 0 {
                        tracing::debug!(%removed, "purged expired pending device-auth rows");
                    }
                }
            }
        });
    }

    let state = Arc::new(AppState {
        agents: Mutex::new(HashMap::new()),
        ui_clients: Mutex::new(HashMap::new()),
        ui_id_counter: AtomicU64::new(0),
        db: pool,
        scheduled_apt_runs: Mutex::new(HashSet::new()),
    });

    update_windows::spawn_scheduler(state.clone());

    let api_routes = Router::new()
        .nest("/device", device_auth::routes())
        .nest("/tokens", tokens::routes())
        .nest("/update-windows", update_windows::routes())
        .nest("/health-probes", health::routes())
        .nest("/fan-out", fan_out::routes())
        .nest("/notifications", notifications::routes())
        .route("/me", get(me_handler))
        .route("/healthz", get(healthz))
        .route("/audit", get(audit_handler))
        .layer(axum::middleware::from_fn(csrf::middleware))
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
    let is_approved = db::token_exists(&state.db, &auth.token).await;

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

    // Read timeout — if neither a real message nor a Pong reply arrives in
    // 75s the TCP connection is dead and we should reap the agent. Without
    // this the receiver loop can hang forever when Cloudflare/the kernel
    // drops the socket without delivering an error.
    let read_timeout = std::time::Duration::from_secs(75);
    loop {
        let next = tokio::time::timeout(read_timeout, receiver.next()).await;
        let frame = match next {
            Ok(Some(Ok(f))) => f,
            Ok(Some(Err(e))) => {
                tracing::info!(error = %e, "agent ws receive error, closing");
                break;
            }
            Ok(None) => break, // peer closed
            Err(_) => {
                tracing::warn!("agent ws idle for {read_timeout:?}, closing");
                break;
            }
        };
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

                    // Stamp the token's hostname + last_seen (and seed
                    // created_at on first contact) for the operator UI.
                    if let Err(e) = db::upsert_token_seen(
                        &state.db,
                        &token,
                        &hostname,
                        now_unix(),
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "failed to persist token metadata");
                    }

                    db::record_audit(
                        &state.db,
                        now_unix(),
                        None,
                        Some(&id),
                        "agent.register",
                        true,
                        Some(&format!("protocol_version={protocol_version}")),
                    )
                    .await;

                    let _ = tx.send(Message::RegisterAck { agent_id: id.clone() });

                    broadcast_agent_list(&state).await;

                    // Push the agent's current probe set so it picks
                    // up any rules added while it was offline.
                    health::push_to_agent(&state, &id).await;
                }
                other => {
                    if let Some(agent_id) = &agent_id_opt {
                        // Health probe reports — persist last_* fields
                        // and audit on green→red / red→green transitions.
                        if let Message::HealthProbeReport { results } = &other {
                            for r in results {
                                let Ok(probe_id) = r.id.parse::<i64>() else {
                                    continue;
                                };
                                let state_str = match r.state {
                                    shared::HealthProbeState::Green => "green",
                                    shared::HealthProbeState::Red => "red",
                                };
                                // Look up prior state for transition
                                // detection before overwriting.
                                let prev: Option<String> = sqlx::query_scalar(
                                    "SELECT last_state FROM health_probes WHERE id = ?",
                                )
                                .bind(probe_id)
                                .fetch_optional(&state.db)
                                .await
                                .ok()
                                .flatten();
                                if let Err(e) = db::record_health_probe_result(
                                    &state.db,
                                    probe_id,
                                    r.at,
                                    state_str,
                                    r.latency_ms as i64,
                                    &r.detail,
                                )
                                .await
                                {
                                    tracing::warn!(error = %e, "failed to persist health probe result");
                                }
                                let prev_str = prev.as_deref();
                                if prev_str != Some(state_str) {
                                    db::record_audit(
                                        &state.db,
                                        now_unix(),
                                        Some("health"),
                                        Some(agent_id),
                                        &format!("health_probe.{state_str}"),
                                        matches!(r.state, shared::HealthProbeState::Green),
                                        Some(&format!("id={} {}", probe_id, r.detail)),
                                    )
                                    .await;
                                    // Skip the noisy "first sample"
                                    // notification — emit only on real
                                    // transitions or when going red.
                                    let real_transition = prev_str.is_some();
                                    let going_red = state_str == "red";
                                    if real_transition || going_red {
                                        let level = if state_str == "red" { "error" } else { "info" };
                                        let title = format!(
                                            "health probe #{probe_id} on {} → {state_str}",
                                            agent_id.trim_end_matches("-id"),
                                        );
                                        notifications::notify(
                                            &state.db,
                                            &format!("health_probe.{state_str}"),
                                            Some(agent_id),
                                            level,
                                            &title,
                                            Some(&r.detail),
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                        // Fan-out attribution: if a fan_out_run is
                        // pending for (agent_id, kind), stamp it with
                        // the result.
                        fan_out::maybe_attribute_response(&state, agent_id, &other).await;

                        // Auto-update window bookkeeping: if a scheduled
                        // (or run-now) upgrade is in flight for this
                        // agent, attribute the AptUpgradeResponse to it.
                        if let Message::AptUpgradeResponse {
                            success,
                            log,
                            error,
                            ..
                        } = &other
                        {
                            let mut sched = state.scheduled_apt_runs.lock().await;
                            if sched.remove(agent_id) {
                                drop(sched);
                                let status = if *success { "success" } else { "failed" };
                                let log_combined = match error {
                                    Some(e) if !e.is_empty() => {
                                        format!("{log}\n[error] {e}")
                                    }
                                    _ => log.clone(),
                                };
                                if let Err(e) = db::record_update_window_result(
                                    &state.db,
                                    agent_id,
                                    now_unix(),
                                    status,
                                    &log_combined,
                                )
                                .await
                                {
                                    tracing::warn!(error = %e, "failed to persist update_window result");
                                }
                                db::record_audit(
                                    &state.db,
                                    now_unix(),
                                    Some("scheduler"),
                                    Some(agent_id),
                                    "update_window.result",
                                    *success,
                                    error.as_deref(),
                                )
                                .await;
                                webhook::fire_update_result(
                                    state.db.clone(),
                                    agent_id.clone(),
                                    status.to_string(),
                                    log_combined.clone(),
                                    error.clone(),
                                    now_unix(),
                                );
                                let level = if *success { "info" } else { "error" };
                                let title = format!(
                                    "apt upgrade on {} → {}",
                                    agent_id.trim_end_matches("-id"),
                                    status
                                );
                                let body_snippet: String = log_combined
                                    .lines()
                                    .rev()
                                    .take(8)
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                notifications::notify(
                                    &state.db,
                                    "update_window.result",
                                    Some(agent_id),
                                    level,
                                    &title,
                                    if body_snippet.is_empty() {
                                        None
                                    } else {
                                        Some(&body_snippet)
                                    },
                                )
                                .await;
                            }
                        }
                        let ui_msg = UiMessage::AgentMessage {
                            agent_id: agent_id.clone(),
                            message: other,
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

    let read_timeout = std::time::Duration::from_secs(75);
    loop {
        let next = tokio::time::timeout(read_timeout, receiver.next()).await;
        let frame = match next {
            Ok(Some(Ok(f))) => f,
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => {
                tracing::warn!(client_id, "ui ws idle for {read_timeout:?}, closing");
                break;
            }
        };
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
