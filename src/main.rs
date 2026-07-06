// W0 safety-net baseline: this crate predates a clippy gate. The lints
// below are pre-existing, purely stylistic/pedantic, and are allowed
// crate-wide so the CI `-D warnings` gate enforces the substantive lints
// (correctness / perf / security) without a repo-wide cosmetic reformat.
// Tracked for a focused cleanup in W6.
#![allow(
    clippy::collapsible_if,
    clippy::redundant_pattern_matching,
    clippy::match_like_matches_macro,
    clippy::identity_op,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::too_many_arguments
)]

mod agent_dispatch;
mod anon_limiter;
mod api_auth;
mod api_v1;
mod auth;
mod backups;
mod crypto;
mod csrf;
mod db;
mod device_auth;
mod ee;
mod ee_recording;
mod fan_out;
mod health;
mod internal_auth;
mod invites;
mod ip_allowlist;
mod labels;
mod metrics;
mod mfa;
mod notifications;
mod operation_routing;
mod passkey_mfa;
mod probe_library;
mod rbac;
mod security_headers;
mod telemetry;
mod throttle;
mod tls;
mod tokens;
mod update_windows;
mod users;
mod webhook;

/// Hard CE seat cap. Enforced on new sign-ins in `auth::callback_handler`
/// and surfaced through `/api/users` so the dashboard can show
/// headroom. Existing seats always get through; the cap only blocks
/// *adding* a seat. EE will lift this with a license-keyed cap.
pub const CE_USER_LIMIT: usize = 3;

use axum::{
    Router,
    extract::{
        DefaultBodyLimit, Query, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
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
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex, mpsc};
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

type AgentTx = mpsc::UnboundedSender<Message>;
type UiTx = mpsc::UnboundedSender<UiMessage>;

pub struct UiClient {
    tx: UiTx,
    login: String,
    client_ip: String,
    agent_access: AgentAccess,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum AgentAccess {
    Unrestricted,
    Restricted { allowed_agents: HashSet<String> },
}

/// One row in the live-agents map. Combining the WS sender with the
/// reported capabilities under a single Mutex closes the race that
/// existed when these were two parallel maps: a UI client could
/// observe an agent in `agents` without yet appearing in
/// `agent_capabilities` (or vice versa, on disconnect).
#[derive(Clone)]
pub struct AgentEntry {
    pub tx: AgentTx,
    pub capabilities: Vec<String>,
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub type PendingBackupListTx = tokio::sync::oneshot::Sender<shared::Message>;
pub type PendingBackupRestoreTx = tokio::sync::oneshot::Sender<shared::Message>;

/// Disconnect debounce state. The agent webhook fan-out distinguishes
/// "transient WS blip" from "agent is actually offline" by gating the
/// `agent.disconnect` webhook behind a grace window
/// (`DISCONNECT_GRACE_SECS`, default 50). On WS teardown we move the
/// agent into `Pending(handle)`; if it re-registers before the handle
/// fires we abort the handle and stay silent. If the handle fires, we
/// post the disconnect webhook and transition to `Confirmed`. The next
/// successful re-register on a `Confirmed` entry posts an
/// `agent.reconnect` webhook so the operator knows it's back.
pub enum DisconnectState {
    /// Awaiting reconnect. The JoinHandle is the debounce task; abort
    /// it on a fresh register to suppress both webhooks.
    Pending(tokio::task::JoinHandle<()>),
    /// Disconnect webhook has fired and the agent is still gone. `since` is
    /// the unix time it was confirmed offline; `last_alert` is the last time a
    /// recurring `agent.still_offline` reminder fired (initially == `since`).
    /// The next register clears this and posts the matching reconnect webhook.
    Confirmed { since: i64, last_alert: i64 },
}

pub struct AppState {
    /// Live-agent registry: agent_id → (tx, capabilities). Single
    /// map (not two parallel ones) so a UI snapshot built under the
    /// lock is internally consistent. Cleared on disconnect.
    pub agents: Mutex<HashMap<String, AgentEntry>>,
    /// Per-agent debounce state for the disconnect/reconnect webhook
    /// pair. See `DisconnectState`.
    pub disconnect_states: Mutex<HashMap<String, DisconnectState>>,
    pub ui_clients: Mutex<HashMap<u64, UiClient>>,
    pub operation_owners: Mutex<operation_routing::OperationOwners>,
    pub ui_id_counter: AtomicU64,
    pub db: SqlitePool,
    /// Per-login throttle for /api/auth/mfa/verify failures. Defends
    /// against an attacker who has stolen a pending-MFA cookie and is
    /// brute-forcing the 6-digit TOTP space.
    pub mfa_throttle: throttle::Throttle,
    /// Per-IP throttle for /api/device/approve. The 8-char user_code
    /// has only ~10^9 entropy and could otherwise be brute-forced.
    pub device_approve_throttle: throttle::Throttle,
    /// Per-IP token-bucket limiter for the anonymous-attacker surface
    /// (/auth/*, /api/me, /api/auth/mfa/verify). This is the
    /// defence-in-depth layer that catches unauthenticated brute force
    /// when the edge (Cloudflare WAF) is bypassed via Tailscale-direct
    /// or a misconfigured DNS record.
    pub anon_ip_limiter: throttle::IpBucketLimiter,
    /// agent_ids currently expecting an `AptUpgradeResponse` that should
    /// be attributed to the auto-update scheduler (or `run_now` button)
    /// rather than a UI-driven upgrade. Cleared when the response lands.
    pub scheduled_apt_runs: Mutex<HashSet<String>>,
    /// Per-agent oneshot waiters for `BackupListArchivesResponse`. The
    /// HTTP handler pushes a sender here and awaits the channel; the
    /// WS receive loop pops the most recent waiter for that agent and
    /// forwards the response.
    pub pending_backup_lists:
        Mutex<HashMap<String, std::collections::VecDeque<PendingBackupListTx>>>,
    pub pending_backup_restores:
        Mutex<HashMap<String, std::collections::VecDeque<PendingBackupRestoreTx>>>,
    /// Session-recording tap (EE). No-op unless EE_RECORD_TERMINALS is enabled.
    pub recorder: ee_recording::Recorder,
    /// In-flight EE runbook command executions: request_id → oneshot waiter.
    /// `/internal/exec-command` inserts a waiter and awaits it; the agent WS
    /// receive loop resolves it when the matching `RunCommandResponse` lands.
    pub pending_exec:
        Mutex<HashMap<String, (String, tokio::sync::oneshot::Sender<shared::Message>)>>,
    /// Cached global IP allowlist `(cidr, enabled)`, refreshed from EE every
    /// 30s. Empty / all-disabled = no restriction. Enforced on dashboard paths
    /// by `ip_allowlist::middleware`.
    pub ip_allowlist: Mutex<Vec<(String, bool)>>,
}

/// Grace window between an agent's WS read-loop exiting and the
/// `agent.disconnect` webhook actually firing. Reconnects within
/// this window are silent (no disconnect, no reconnect webhook).
///
/// 50s is derived from the existing architectural pair: the agent
/// pings on a 25s cadence (matching the server-side WS heartbeat)
/// and a single missed beat is normal (Cloudflare jitter, brief TCP
/// retransmit). 50s = ~two missed heartbeats, which is a real "this
/// agent is gone" signal, but still under the agent's 75s idle
/// watchdog so a healthy systemd / kubelet restart cycle reconnects
/// inside the grace and stays silent. Not an operator knob — it
/// tracks those two underlying timeouts.
const DISCONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(50);

/// Re-fires an `agent.still_offline` webhook on an interval while an agent
/// stays confirmed-offline, so a silently-stranded agent (e.g. one stuck
/// failing to reconnect) is surfaced instead of sitting unnoticed for days —
/// the one-shot `agent.disconnect` is easy to miss. Interval is
/// `STALE_AGENT_REALERT_SECS` (default 3600; 0 disables). Routes via the same
/// `DISCONNECT_*` webhook sinks.
fn spawn_stale_agent_alerter(state: std::sync::Arc<AppState>) {
    let realert_secs: i64 = std::env::var("STALE_AGENT_REALERT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    if realert_secs <= 0 {
        tracing::info!("stale-agent alerter disabled (STALE_AGENT_REALERT_SECS=0)");
        return;
    }
    tracing::info!(realert_secs, "stale-agent alerter enabled");
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        tick.tick().await; // skip the immediate first tick
        loop {
            tick.tick().await;
            let now = now_unix();
            // Collect the due agents under the lock; fire the webhooks outside it.
            let mut due: Vec<(String, i64)> = Vec::new();
            {
                let mut states = state.disconnect_states.lock().await;
                for (id, st) in states.iter_mut() {
                    if let DisconnectState::Confirmed { since, last_alert } = st {
                        if now - *last_alert >= realert_secs {
                            *last_alert = now;
                            due.push((id.clone(), *since));
                        }
                    }
                }
            }
            for (id, since) in due {
                tracing::warn!(agent_id = %id, down_secs = now - since, "agent still offline — re-alerting");
                webhook::fire_agent_still_offline(state.db.clone(), id, now - since, now);
            }
        }
    });
}

async fn broadcast_agent_list(state: &AppState) {
    let (agents, capabilities) = {
        let map = state.agents.lock().await;
        let agents: Vec<String> = map.keys().cloned().collect();
        let capabilities: HashMap<String, Vec<String>> = map
            .iter()
            .map(|(id, entry)| (id.clone(), entry.capabilities.clone()))
            .collect();
        (agents, capabilities)
    };
    let identities: Vec<(u64, String, String)> = state
        .ui_clients
        .lock()
        .await
        .iter()
        .map(|(id, client)| (*id, client.login.clone(), client.client_ip.clone()))
        .collect();
    for (client_id, login, client_ip) in identities {
        let access = ui_agent_access(state, &login, &client_ip, &agents).await;
        let mut clients = state.ui_clients.lock().await;
        let Some(client) = clients.get_mut(&client_id) else {
            continue;
        };
        client.agent_access = access;
        let (visible_agents, visible_capabilities) =
            ee_filter_agent_list(agents.clone(), capabilities.clone(), &client.agent_access);
        let msg = UiMessage::ListAgentsResponse {
            agents: visible_agents,
            capabilities: visible_capabilities,
        };
        if client.tx.send(msg).is_err() {
            clients.remove(&client_id);
        }
    }
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Whether the optional in-tree backup machinery is enabled. Off by
/// default — most operators back up at the hypervisor / VM level
/// (Proxmox, vSphere, etc.) and don't want a redundant in-VM tar.
/// Toggle via `BACKUPS_ENABLED=true` in the docker host's `.env`.
fn backups_enabled() -> bool {
    matches!(
        std::env::var("BACKUPS_ENABLED")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[derive(Serialize)]
struct FeaturesResponse {
    backups_enabled: bool,
}

async fn features_handler() -> impl IntoResponse {
    axum::Json(FeaturesResponse {
        backups_enabled: backups_enabled(),
    })
}

#[derive(Serialize)]
struct MeResponse {
    user: String,
    role: String,
    mfa_enabled: bool,
    /// True when the active session is fully verified. False during the
    /// pending-MFA window between OAuth and successful TOTP entry.
    mfa_verified: bool,
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<i64>,
}

async fn audit_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    if let Some(ee_url) = ee::ee_sidecar_url() {
        let limit = q.limit.unwrap_or(200).clamp(1, 1000);
        let url = format!(
            "{}/api/ee/audit?limit={}",
            ee_url.trim_end_matches('/'),
            limit
        );
        match internal_auth::send(
            &reqwest::Client::new(),
            reqwest::Method::GET,
            &url,
            Vec::new(),
            "",
            "system",
            "admin",
            std::time::Duration::from_secs(10),
        )
        .await
        {
            Ok(resp) => {
                let status =
                    StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                let body = resp.text();
                return (
                    status,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                )
                    .into_response();
            }
            Err(e) => {
                tracing::warn!(error = %e, "EE audit proxy failed, falling back to CE");
            }
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

async fn me_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if auth::is_dev_mode() {
        return (
            StatusCode::OK,
            axum::Json(MeResponse {
                user: "dev".into(),
                role: "admin".into(),
                mfa_enabled: false,
                mfa_verified: true,
            }),
        )
            .into_response();
    }
    let Some(cookie) = jar.get("auth_token") else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let Some(claims) = auth::claims_from_token(cookie.value()) else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let (role, mfa_enabled) = match db::get_user(&state.db, &claims.sub).await {
        Ok(Some(row)) => (row.role, row.totp_enabled != 0),
        _ => (claims.role.clone(), false),
    };
    (
        StatusCode::OK,
        axum::Json(MeResponse {
            user: claims.sub,
            role,
            mfa_enabled,
            mfa_verified: claims.mfa,
        }),
    )
        .into_response()
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

    // Refuse to start with an obviously-bad JWT_SECRET. Catches the
    // most common deployment footgun: a freshly cloned compose stanza
    // launched without an `.env` override.
    auth::assert_jwt_secret_present();
    if ee::ee_active() {
        if let Err(error) = internal_auth::validate_config() {
            tracing::error!(%error, "directional CE/EE authentication is not configured");
            std::process::exit(1);
        }
    }

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

    // CE: 7-day local audit retention. Runs once at startup so a server
    // that comes up after a long downtime drops stale rows immediately
    // (rather than waiting an hour), then hourly thereafter.
    {
        let pool = pool.clone();
        tokio::spawn(async move {
            const RETENTION_SECS: i64 = 7 * 24 * 3600;
            // Eager first sweep.
            let cutoff = now_unix() - RETENTION_SECS;
            match db::purge_audit_before(&pool, cutoff).await {
                Ok(removed) if removed > 0 => {
                    tracing::info!(%removed, %cutoff, "startup audit retention sweep");
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "startup audit retention sweep failed");
                }
            }
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await; // drop the immediate first tick
            loop {
                tick.tick().await;
                let cutoff = now_unix() - RETENTION_SECS;
                match db::purge_audit_before(&pool, cutoff).await {
                    Ok(removed) if removed > 0 => {
                        tracing::info!(%removed, %cutoff, "purged audit rows older than retention window");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "audit retention sweep failed");
                    }
                }
            }
        });
    }

    let state = Arc::new(AppState {
        agents: Mutex::new(HashMap::new()),
        disconnect_states: Mutex::new(HashMap::new()),
        ui_clients: Mutex::new(HashMap::new()),
        operation_owners: Mutex::new(operation_routing::OperationOwners::default()),
        ui_id_counter: AtomicU64::new(0),
        db: pool,
        mfa_throttle: throttle::Throttle::new(),
        device_approve_throttle: throttle::Throttle::new(),
        anon_ip_limiter: throttle::IpBucketLimiter::new(),
        scheduled_apt_runs: Mutex::new(HashSet::new()),
        pending_backup_lists: Mutex::new(HashMap::new()),
        pending_backup_restores: Mutex::new(HashMap::new()),
        pending_exec: Mutex::new(HashMap::new()),
        recorder: ee_recording::Recorder::new(),
        ip_allowlist: Mutex::new(Vec::new()),
    });

    update_windows::spawn_scheduler(state.clone());
    telemetry::spawn_reporter(state.clone());
    spawn_stale_agent_alerter(state.clone());
    if backups_enabled() {
        backups::spawn_scheduler(state.clone());
        tracing::info!("backups: enabled (BACKUPS_ENABLED is set)");
    } else {
        tracing::info!("backups: disabled (set BACKUPS_ENABLED=true to enable)");
    }

    let mut api_routes = Router::new()
        .nest("/device", device_auth::routes())
        .nest("/tokens", tokens::routes())
        .nest("/update-windows", update_windows::routes())
        .nest("/health-probes", health::routes())
        .nest("/fan-out", fan_out::routes())
        .nest("/notifications", notifications::routes())
        .nest("/agent-labels", labels::routes());
    if backups_enabled() {
        api_routes = api_routes.nest("/backups", backups::routes());
    }
    let mut api_routes = api_routes
        .nest("/probe-library", probe_library::routes())
        .nest("/auth/mfa", mfa::routes())
        .nest("/auth/passkey", passkey_mfa::routes())
        .nest("/users", users::routes())
        .nest("/invites", invites::routes())
        .nest("/metrics", metrics::routes())
        .nest("/telemetry", telemetry::routes())
        .route("/me", get(me_handler))
        .route("/healthz", get(healthz))
        .route("/audit", get(audit_handler))
        .route("/features", get(features_handler));
    if ee::ee_active() {
        // EE proxy lives INSIDE the authenticated /api nest so it inherits
        // the api auth + CSRF + RBAC layers below. The handler resolves the
        // session and injects verified `x-shellfleet-{login,role}` headers;
        // it builds a fresh upstream request, so any client-supplied
        // identity headers are never forwarded. EE then trusts those
        // headers because they only arrive behind the internal-secret gate.
        api_routes = api_routes.route("/ee/{*path}", axum::routing::any(ee::ee_proxy_handler));
    }
    let api_routes = api_routes
        // Cap any single JSON / form body. The largest legitimate
        // request is a SwarmStackDeployRequest carrying a compose YAML,
        // realistically well under 256 KiB. 1 MiB is generous and
        // closes the unbounded-body footgun.
        .layer(DefaultBodyLimit::max(1 * 1024 * 1024))
        // RBAC runs after CSRF: CSRF rejects forged cross-site writes,
        // RBAC rejects insufficiently-privileged callers (viewer-on-
        // mutating, pending-MFA-on-anything-but-/verify, etc.).
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rbac::middleware,
        ))
        .layer(axum::middleware::from_fn(csrf::middleware))
        .with_state(state.clone());

    let v1_routes = api_v1::routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            api_auth::middleware,
        ))
        .with_state(state.clone());

    let dev_mode = crate::auth::is_dev_mode();

    // In production `/agent/ws` is served ONLY on the mTLS listener
    // (see the AgentTlsListener spawned below), so the plain HTTP port
    // can't be used to bypass mutual TLS. In dev mode the mTLS listener
    // isn't started (no operator certs locally), so `/agent/ws` stays on
    // the plain port for local end-to-end testing.
    let ws_routes = Router::new().route("/ui/ws", get(ui_ws_handler));
    let ws_routes = if dev_mode {
        ws_routes.route("/agent/ws", get(agent_ws_handler))
    } else {
        ws_routes
    }
    .with_state(state.clone());

    let trace_layer = TraceLayer::new_for_http()
        .on_response(DefaultOnResponse::new().level(Level::INFO))
        .on_failure(DefaultOnFailure::new().level(Level::WARN));

    let mut app = Router::new()
        .route("/healthz", get(healthz))
        .nest("/auth", auth::auth_routes(state.clone()))
        .nest("/api", api_routes)
        .nest("/api/v1", v1_routes)
        .merge(ws_routes)
        .merge(invites::public_routes().with_state(state.clone()));

    if ee::ee_active() {
        tracing::info!(
            url = %ee::ee_sidecar_url().unwrap_or_default(),
            "EE sidecar detected — mounting /internal routes (the /api/ee proxy is mounted inside the authenticated /api nest)"
        );
        app = app.nest(
            "/internal",
            ee::routes(state.clone()).with_state(state.clone()),
        );
        // Keep the global IP allowlist cache warm so the gate can enforce it.
        ip_allowlist::spawn_refresher(state.clone());
    }

    let app = app
        // Per-real-IP token-bucket limiter on the anonymous-attacker
        // surface (/auth/*, /api/me, /api/auth/mfa/verify,
        // /api/auth/mfa/status). Defence-in-depth on top of the edge
        // limiter (Cloudflare WAF) — closes the gap when the edge is
        // bypassed via direct Tailscale.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            anon_limiter::middleware,
        ))
        // Security headers: HSTS, frame-options, etc.
        .layer(axum::middleware::from_fn(security_headers::middleware))
        // Global IP allowlist (EE) — gates dashboard/user paths when populated.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ip_allowlist::middleware,
        ))
        .layer(trace_layer);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!(%addr, "server listening");

    // Production agent mTLS listener. Required (not staged): agents
    // present a client cert verified against `AGENT_MTLS_CA_PATH`; the
    // server presents `SERVER_TLS_CERT_PATH`/`SERVER_TLS_KEY_PATH`.
    // `/agent/ws` is NOT mounted on the plain port in prod (above), so
    // this is the only path agents can reach the WS handler on. If the
    // certs aren't configured the listener isn't started and agents
    // cannot connect — fail-closed, while the browser UI on :8080 stays
    // up.
    if !dev_mode {
        let cert = std::env::var("SERVER_TLS_CERT_PATH").ok().filter(|s| !s.is_empty());
        let key = std::env::var("SERVER_TLS_KEY_PATH").ok().filter(|s| !s.is_empty());
        let ca = std::env::var("AGENT_MTLS_CA_PATH").ok().filter(|s| !s.is_empty());
        match (cert, key, ca) {
            (Some(cert), Some(key), Some(ca)) => {
                let cert = std::path::PathBuf::from(cert);
                let key = std::path::PathBuf::from(key);
                let ca = std::path::PathBuf::from(ca);
                match tls::build_server_config(&cert, &key, &ca) {
                    Ok(config) => {
                        let port: u16 = std::env::var("AGENT_MTLS_PORT")
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(8443);
                        let tls_addr = SocketAddr::from(([0, 0, 0, 0], port));
                        match tls::AgentTlsListener::bind(tls_addr, config).await {
                            Ok(listener) => {
                                tracing::info!(
                                    addr = %tls_addr,
                                    "agent mTLS listener started (client cert required)"
                                );
                                let agent_app = Router::new()
                                    .route("/agent/ws", get(agent_ws_handler))
                                    .with_state(state.clone());
                                tokio::spawn(async move {
                                    if let Err(e) =
                                        axum::serve(listener, agent_app.into_make_service()).await
                                    {
                                        tracing::error!(%e, "agent mTLS listener stopped");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!(
                                    %e, addr = %tls_addr,
                                    "agent mTLS listener failed to bind — agents cannot connect"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            %e,
                            "agent mTLS listener NOT started — fix SERVER_TLS_CERT_PATH / \
                             SERVER_TLS_KEY_PATH / AGENT_MTLS_CA_PATH; agents cannot connect"
                        );
                    }
                }
            }
            _ => {
                tracing::error!(
                    "agent mTLS listener NOT started — SERVER_TLS_CERT_PATH / \
                     SERVER_TLS_KEY_PATH / AGENT_MTLS_CA_PATH are not all set; agents cannot \
                     connect. Configure them and restart."
                );
            }
        }
    }

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    // `into_make_service_with_connect_info` is required so the
    // `ConnectInfo<SocketAddr>` extractor in `anon_limiter::middleware`
    // gets the peer address. Without it the middleware would 500 on
    // every request that goes through the IP limiter.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn agent_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let token = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| {
            s.strip_prefix("Bearer ")
                .or_else(|| s.strip_prefix("bearer "))
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(t) => t,
        None => {
            tracing::warn!("agent ws upgrade missing Authorization header");
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    };

    let is_approved = db::token_exists(&state.db, &token).await;

    let legacy_token = std::env::var("AGENT_SECRET").unwrap_or_default();
    let legacy_match = !legacy_token.is_empty() && token == legacy_token;

    if !legacy_match && !is_approved {
        tracing::warn!("unauthorized agent connection attempt");
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(move |socket| handle_agent_socket(socket, state, token))
        .into_response()
}

/// Returns the allowed Origin set for /ui/ws. Always includes the
/// `UI_URL` origin; `WS_ALLOWED_ORIGINS` (comma-separated) is appended.
fn ws_allowed_origins() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(ui_url) = std::env::var("UI_URL") {
        if let Some(origin) = url::Url::parse(&ui_url).ok().and_then(|u| {
            let scheme = u.scheme();
            let host = u.host_str()?;
            let port = u.port();
            Some(match port {
                Some(p) => format!("{scheme}://{host}:{p}"),
                None => format!("{scheme}://{host}"),
            })
        }) {
            out.push(origin);
        }
    }
    if let Ok(extra) = std::env::var("WS_ALLOWED_ORIGINS") {
        for s in extra.split(',') {
            let t = s.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    out
}

async fn ui_ws_handler(
    jar: CookieJar,
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let dev_mode = std::env::var("JWT_SECRET").unwrap_or_default() == "dev";

    // 1. Origin allow-list. Browsers always send Origin on a WS upgrade
    //    they initiated; absence of Origin from a browser is a strong
    //    indicator of a non-browser client and we reject it. Skip the
    //    check entirely in dev mode so local tooling still works.
    if !dev_mode {
        let allowed = ws_allowed_origins();
        let origin = headers
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let ok = match &origin {
            Some(o) => allowed.iter().any(|a| a.eq_ignore_ascii_case(o)),
            None => false,
        };
        if !ok {
            tracing::warn!(
                origin = ?origin,
                allowed = ?allowed,
                "ui ws upgrade rejected: origin not allowed"
            );
            return (StatusCode::FORBIDDEN, "origin").into_response();
        }
    }

    // 2. Cookie auth + role capture. We pin the user's role at connect
    //    time *and* re-check it on each mutating message in
    //    handle_ui_socket — neither alone is enough. Pinning catches the
    //    "log in then upgrade WS while still admin" race; re-check
    //    catches the "demoted while WS still open" case.
    let (login, initial_role) = if dev_mode {
        ("dev".to_string(), auth::Role::Admin)
    } else {
        let Some(cookie) = jar.get("auth_token") else {
            tracing::warn!("ui ws: no auth cookie");
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        };
        let Some(claims) = auth::claims_from_token(cookie.value()) else {
            tracing::warn!("ui ws: invalid jwt");
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        };
        if !claims.mfa {
            tracing::warn!(login = %claims.sub, "ui ws: pending-mfa cookie rejected");
            return (StatusCode::FORBIDDEN, "MFA required").into_response();
        }
        (claims.sub.clone(), auth::Role::parse(&claims.role))
    };

    let client_ip = throttle::real_client_ip(&headers, Some(peer.ip()));
    ws.on_upgrade(move |socket| handle_ui_socket(socket, state, login, initial_role, client_ip))
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
        let parsed = serde_json::from_str::<Message>(&text);
        if let Err(ref e) = parsed {
            tracing::warn!(error = %e, "dropped un-parseable agent message");
        }
        if let Ok(parsed_msg) = parsed {
            match parsed_msg {
                Message::Register {
                    hostname,
                    protocol_version,
                    capabilities,
                    metadata,
                } => {
                    // Identity-binding guard: a per-agent token is bound to the
                    // first hostname it registers with. Reject a token reused
                    // under a DIFFERENT hostname — prevents one valid token from
                    // impersonating another agent's id and hijacking its command
                    // stream. The shared legacy AGENT_SECRET is exempt (it
                    // intentionally serves many hosts).
                    let legacy_token = std::env::var("AGENT_SECRET").unwrap_or_default();
                    let is_legacy = !legacy_token.is_empty() && token == legacy_token;
                    if !is_legacy {
                        if let Ok(Some(bound)) = db::token_hostname(&state.db, &token).await {
                            if !bound.is_empty() && bound != hostname {
                                tracing::warn!(
                                    %hostname, bound = %bound,
                                    "agent token reused under a different hostname — rejecting (identity-binding guard)"
                                );
                                db::record_audit(
                                    &state.db,
                                    now_unix(),
                                    None,
                                    None,
                                    "agent.register",
                                    false,
                                    Some(&format!("token bound to '{bound}', got '{hostname}'")),
                                )
                                .await;
                                break;
                            }
                        }
                    }
                    // Protocol-version skew is advisory (legacy 0 is allowed);
                    // surface it so an incompatible agent isn't silent.
                    if protocol_version != 0 && protocol_version != shared::PROTOCOL_VERSION {
                        tracing::warn!(
                            %hostname,
                            got = protocol_version,
                            expected = shared::PROTOCOL_VERSION,
                            "agent protocol-version skew"
                        );
                    }
                    let id = format!("{}-id", hostname);
                    agent_id_opt = Some(id.clone());

                    state.agents.lock().await.insert(
                        id.clone(),
                        AgentEntry {
                            tx: tx.clone(),
                            capabilities: capabilities.clone(),
                        },
                    );
                    tracing::info!(
                        agent_id = %id,
                        %protocol_version,
                        capabilities = ?capabilities,
                        "agent registered"
                    );

                    // Resolve any pending or confirmed disconnect for
                    // this agent. Three cases:
                    //   1. nothing in the map → first connect (or
                    //      reconnect that already had its reconnect
                    //      webhook delivered earlier) — no webhook.
                    //   2. Pending(handle) → reconnect within the
                    //      grace window. Abort the debounce task and
                    //      suppress both the disconnect AND any
                    //      reconnect webhook. The operator was never
                    //      notified of a disconnect for this blip,
                    //      so they don't need a reconnect either.
                    //   3. Confirmed → the disconnect webhook fired
                    //      already (the agent was offline past the
                    //      grace window). Post an `agent.reconnect`
                    //      webhook so the operator knows it's back.
                    let prior = state.disconnect_states.lock().await.remove(&id);
                    match prior {
                        Some(DisconnectState::Pending(handle)) => {
                            handle.abort();
                            tracing::info!(
                                agent_id = %id,
                                "agent reconnect within grace window — disconnect webhook suppressed"
                            );
                        }
                        Some(DisconnectState::Confirmed { .. }) => {
                            tracing::info!(
                                agent_id = %id,
                                "agent connected after confirmed disconnect — firing connect webhook"
                            );
                            webhook::fire_agent_connect(state.db.clone(), id.clone(), now_unix());
                        }
                        None => {}
                    }

                    // Stamp the token's hostname + last_seen (and seed
                    // created_at on first contact) for the operator UI.
                    if let Err(e) =
                        db::upsert_token_seen(&state.db, &token, &hostname, now_unix()).await
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

                    let _ = tx.send(Message::RegisterAck {
                        agent_id: id.clone(),
                    });

                    broadcast_agent_list(&state).await;

                    // Push the agent's current probe set so it picks
                    // up any rules added while it was offline.
                    health::push_to_agent(&state, &id).await;

                    if ee::ee_active() {
                        let ee_url = ee::ee_sidecar_url().unwrap_or_default();
                        let tag_body = serde_json::json!({
                            "agent_id": id,
                            "hostname": hostname,
                            "capabilities": capabilities,
                            "metadata": metadata,
                        });
                        let url = format!("{}/internal/tag-resolve", ee_url.trim_end_matches('/'));
                        tokio::spawn(async move {
                            let _ = internal_auth::send_json(
                                &reqwest::Client::new(),
                                reqwest::Method::POST,
                                &url,
                                &tag_body,
                                "system",
                                "admin",
                                std::time::Duration::from_secs(5),
                            )
                            .await;
                        });
                    }
                }
                Message::CapabilitiesUpdate { capabilities } => {
                    if let Some(id) = &agent_id_opt {
                        let mut map = state.agents.lock().await;
                        if let Some(entry) = map.get_mut(id) {
                            if entry.capabilities != capabilities {
                                tracing::info!(
                                    agent_id = %id,
                                    capabilities = ?capabilities,
                                    "agent capabilities updated"
                                );
                                entry.capabilities = capabilities;
                                drop(map);
                                broadcast_agent_list(&state).await;
                            }
                        }
                    }
                }
                // Application-level heartbeat (protocol v18+): a v18 agent sends
                // `Ping` every ~20s and resets its reconnect watchdog only on real
                // server messages, so this `Pong` is what proves the end-to-end
                // path is alive through a proxy that may keep the socket warm with
                // its own control frames. Receiving the `Ping` here also resets the
                // server's read timeout, so the symmetric reap can't misfire.
                Message::Ping => {
                    let _ = tx.send(Message::Pong);
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
                                        let level =
                                            if state_str == "red" { "error" } else { "info" };
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
                                        // Outbound webhook fan-out for the
                                        // green↔red transition. Reads
                                        // HEALTH_* env (independent from
                                        // UPDATE_* / BACKUP_* / DISCONNECT_*).
                                        // No-op when nothing's configured.
                                        webhook::fire_health_probe_transition(
                                            state.db.clone(),
                                            agent_id.to_string(),
                                            format!("#{probe_id}"),
                                            r.state,
                                            r.detail.clone(),
                                            now_unix(),
                                        );
                                    }
                                }
                            }
                        }
                        // Backup list-archives / restore: pop oldest
                        // pending oneshot for this agent and resolve.
                        if matches!(other, Message::BackupListArchivesResponse { .. }) {
                            let mut waiters = state.pending_backup_lists.lock().await;
                            if let Some(q) = waiters.get_mut(agent_id) {
                                if let Some(tx) = q.pop_front() {
                                    let _ = tx.send(other.clone());
                                }
                            }
                        }
                        if matches!(other, Message::BackupRestoreResponse { .. }) {
                            let mut waiters = state.pending_backup_restores.lock().await;
                            if let Some(q) = waiters.get_mut(agent_id) {
                                if let Some(tx) = q.pop_front() {
                                    let _ = tx.send(other.clone());
                                }
                            }
                        }
                        // Runbook one-shot exec: resolve the waiter keyed by
                        // request_id (set by /internal/exec-command).
                        if let Message::RunCommandResponse { request_id, .. } = &other {
                            let mut pe = state.pending_exec.lock().await;
                            let should = pe
                                .get(request_id)
                                .map(|(expected, _)| expected == agent_id)
                                .unwrap_or(false);
                            let maybe_tx = if should {
                                pe.remove(request_id).map(|(_, tx)| tx)
                            } else {
                                None
                            };
                            drop(pe);
                            if let Some(tx) = maybe_tx {
                                let _ = tx.send(other.clone());
                            }
                            continue;
                        }

                        // Backup result attribution.
                        if let Message::BackupRunResponse {
                            id,
                            name,
                            success,
                            archive_path,
                            bytes,
                            log,
                            error,
                        } = &other
                        {
                            if let Ok(job_id) = id.parse::<i64>() {
                                let status = if *success { "success" } else { "failed" };
                                let combined = match error {
                                    Some(e) if !e.is_empty() => {
                                        format!("{log}\n[error] {e}")
                                    }
                                    _ => log.clone(),
                                };
                                if let Err(e) = db::record_backup_job_result(
                                    &state.db,
                                    job_id,
                                    now_unix(),
                                    status,
                                    archive_path,
                                    *bytes as i64,
                                    &combined,
                                )
                                .await
                                {
                                    tracing::warn!(error = %e, "persist backup result failed");
                                }
                                db::record_audit(
                                    &state.db,
                                    now_unix(),
                                    Some("scheduler"),
                                    Some(agent_id),
                                    "backup_job.result",
                                    *success,
                                    Some(&format!(
                                        "name={name} bytes={bytes} archive={archive_path}"
                                    )),
                                )
                                .await;
                                let level = if *success { "info" } else { "error" };
                                let title = format!(
                                    "backup '{name}' on {} → {status}",
                                    agent_id.trim_end_matches("-id")
                                );
                                let body_summary = if *success {
                                    format!("{bytes} bytes → {archive_path}")
                                } else {
                                    error.clone().unwrap_or_else(|| "failed".into())
                                };
                                notifications::notify(
                                    &state.db,
                                    "backup_job.result",
                                    Some(agent_id),
                                    level,
                                    &title,
                                    Some(&body_summary),
                                )
                                .await;
                                // Outbound webhook fan-out (BACKUP_* env).
                                // Sends the full combined log so a
                                // chat sink shows the tail of failed
                                // backups; on success the bytes/archive
                                // summary rides in the body.
                                webhook::fire_backup_result(
                                    state.db.clone(),
                                    agent_id.to_string(),
                                    name.clone(),
                                    *success,
                                    if *success {
                                        body_summary.clone()
                                    } else {
                                        combined.clone()
                                    },
                                    error.clone(),
                                    now_unix(),
                                );
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
                        if let Message::DriftSnapshotResponse {
                            ref snapshot_id,
                            ref packages,
                            ref services,
                            ref containers,
                            ref configs,
                            ..
                        } = other
                        {
                            ee::forward_drift_snapshot(
                                agent_id,
                                snapshot_id,
                                packages,
                                services,
                                containers,
                                configs,
                            )
                            .await;
                        }

                        // Recording tap: mirror terminal OUTPUT (agent→user).
                        if let Message::TerminalData { session_id, data } = &other {
                            state.recorder.frame(session_id, "o", data).await;
                        }
                        let operation = operation_routing::agent_operation(&other);
                        let ui_msg = UiMessage::AgentMessage {
                            agent_id: agent_id.clone(),
                            message: other,
                        };
                        if let Some(operation) = operation {
                            let owner = {
                                let mut owners = state.operation_owners.lock().await;
                                let owner = owners.owner(agent_id, &operation.key);
                                if operation.ends {
                                    owners.release_agent_operation(agent_id, &operation.key);
                                }
                                owner
                            };
                            if let Some(owner) = owner {
                                let mut clients = state.ui_clients.lock().await;
                                let remove = match clients.get(&owner) {
                                    Some(ctx)
                                        if agent_allowed_by_access(agent_id, &ctx.agent_access) =>
                                    {
                                        ctx.tx.send(ui_msg).is_err()
                                    }
                                    Some(_) => false,
                                    None => false,
                                };
                                if remove {
                                    clients.remove(&owner);
                                }
                            } else {
                                tracing::warn!(
                                    %agent_id,
                                    operation = ?operation.key,
                                    "dropping operation output without a registered UI owner"
                                );
                            }
                        } else {
                            let mut clients = state.ui_clients.lock().await;
                            clients.retain(|_id, ctx| {
                                if !agent_allowed_by_access(agent_id, &ctx.agent_access) {
                                    return true;
                                }
                                ctx.tx.send(ui_msg.clone()).is_ok()
                            });
                        }
                    }
                }
            }
        }
    }

    send_task.abort();

    if let Some(id) = agent_id_opt {
        // Reconnect-overlap guard: a newer connection for the same agent_id may
        // already have re-registered (k8s pod handover, or a reconnect that beat
        // this socket's teardown). In that case `state.agents` holds the NEW
        // socket's channel, not ours — removing it would evict a live agent and
        // fire a bogus disconnect, leaving it flagged offline forever. Only tear
        // down if the registered entry is still our own channel.
        {
            let mut agents = state.agents.lock().await;
            let still_ours = agents
                .get(&id)
                .map(|e| e.tx.same_channel(&tx))
                .unwrap_or(false);
            if !still_ours {
                tracing::info!(
                    agent_id = %id,
                    "stale agent ws closed; a newer connection already owns this id — skipping disconnect"
                );
                return;
            }
            agents.remove(&id);
        }
        // Cancel any pending exec commands for the disconnected agent.
        {
            let mut pe = state.pending_exec.lock().await;
            pe.retain(|_, (expected, _)| expected != &id);
        }
        state.operation_owners.lock().await.release_agent(&id);
        tracing::info!(agent_id = %id, "agent ws closed; debouncing disconnect webhook");
        broadcast_agent_list(&state).await;

        // Debounced fan-out (DISCONNECT_* env). The agent is gone
        // from `state.agents`, but transient WS blips (Cloudflare
        // idle drop, kernel TCP reset, agent restart from systemd
        // / kubelet) often resolve under the architectural
        // 75s+25s window. Spawn a task that fires the webhook
        // only if the agent is still gone after `DISCONNECT_GRACE`
        // (100s, see the const). On a re-register within that
        // window the register handler aborts this task and
        // suppresses both webhooks. After the window elapses we
        // transition to Confirmed; the next register on a
        // Confirmed entry fires `fire_agent_reconnect` so the
        // operator knows it's back.
        // Independent prefix so this can route to a more urgent
        // sink than the daily apt-update channel.
        let state_for_task = state.clone();
        let id_for_task = id.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(DISCONNECT_GRACE).await;
            // Atomic check-and-transition under the lock: only
            // fire if our Pending entry is still there. If
            // register beat us to it, the entry will have been
            // removed and we stay silent.
            let mut states = state_for_task.disconnect_states.lock().await;
            let still_pending =
                matches!(states.get(&id_for_task), Some(DisconnectState::Pending(_)),);
            if !still_pending {
                return;
            }
            let now_ts = now_unix();
            states.insert(
                id_for_task.clone(),
                DisconnectState::Confirmed {
                    since: now_ts,
                    last_alert: now_ts,
                },
            );
            drop(states);
            tracing::info!(
                agent_id = %id_for_task,
                grace_secs = DISCONNECT_GRACE.as_secs(),
                "agent disconnect grace elapsed — firing webhook"
            );
            webhook::fire_agent_disconnect(state_for_task.db.clone(), id_for_task, now_unix());
        });
        state
            .disconnect_states
            .lock()
            .await
            .insert(id, DisconnectState::Pending(handle));
    }
}

async fn handle_ui_socket(
    socket: WebSocket,
    state: Arc<AppState>,
    login: String,
    initial_role: auth::Role,
    client_ip: String,
) {
    tracing::info!(%login, "new ui websocket connection");
    let (mut sender, mut receiver) = socket.split();

    let (initial_agents, initial_capabilities) = {
        let map = state.agents.lock().await;
        let agents = map.keys().cloned().collect::<Vec<_>>();
        let capabilities = map
            .iter()
            .map(|(id, entry)| (id.clone(), entry.capabilities.clone()))
            .collect::<HashMap<_, _>>();
        (agents, capabilities)
    };
    let mut agent_access = if initial_role == auth::Role::Admin {
        AgentAccess::Unrestricted
    } else {
        ee_fetch_agent_access(&login, &client_ip, &initial_agents).await
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<UiMessage>();
    let client_id = state.ui_id_counter.fetch_add(1, Ordering::Relaxed);
    state.ui_clients.lock().await.insert(
        client_id,
        UiClient {
            tx: tx.clone(),
            login: login.clone(),
            client_ip: client_ip.clone(),
            agent_access: agent_access.clone(),
        },
    );

    // Terminal sessions this client opened, so recordings are stopped if the
    // socket drops without an explicit StopTerminalRequest.
    let mut rec_sessions: std::collections::HashSet<String> = std::collections::HashSet::new();

    let (initial_agents, initial_capabilities) =
        ee_filter_agent_list(initial_agents, initial_capabilities, &agent_access);
    let _ = tx.send(UiMessage::ListAgentsResponse {
        agents: initial_agents,
        capabilities: initial_capabilities,
    });

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
        let parsed = serde_json::from_str::<UiMessage>(&text);
        if let Err(ref e) = parsed {
            tracing::warn!(error = %e, "dropped un-parseable UI message");
        }
        if let Ok(parsed_msg) = parsed {
            match parsed_msg {
                UiMessage::ListAgentsRequest => {
                    let (agents, capabilities) = {
                        let map = state.agents.lock().await;
                        let agents = map.keys().cloned().collect::<Vec<_>>();
                        let capabilities = map
                            .iter()
                            .map(|(id, entry)| (id.clone(), entry.capabilities.clone()))
                            .collect::<HashMap<_, _>>();
                        (agents, capabilities)
                    };
                    agent_access = ui_agent_access(&state, &login, &client_ip, &agents).await;
                    if let Some(client) = state.ui_clients.lock().await.get_mut(&client_id) {
                        client.agent_access = agent_access.clone();
                    }
                    let (agents, capabilities) =
                        ee_filter_agent_list(agents, capabilities, &agent_access);
                    let _ = tx.send(UiMessage::ListAgentsResponse {
                        agents,
                        capabilities,
                    });
                }
                UiMessage::SendToAgent { agent_id, message } => {
                    let target_access = ui_agent_access(
                        &state,
                        &login,
                        &client_ip,
                        std::slice::from_ref(&agent_id),
                    )
                    .await;
                    if let Some(client) = state.ui_clients.lock().await.get_mut(&client_id) {
                        merge_exact_agent_access(
                            &mut client.agent_access,
                            &agent_id,
                            &target_access,
                        );
                    }
                    if !agent_allowed_by_access(&agent_id, &target_access) {
                        let _ = tx.send(UiMessage::PermissionDenied {
                            agent_id: agent_id.clone(),
                            variant_type: "agent_access".to_string(),
                            reason: "not in your allowed agents".to_string(),
                        });
                        continue;
                    }
                    let variant_type = serde_json::to_value(&message)
                        .ok()
                        .and_then(|v| v.get("type").and_then(|t| t.as_str().map(String::from)))
                        .unwrap_or_else(|| "unknown".into());
                    let security = match message.ui_request_security() {
                        Ok(security) => security,
                        Err(error) => {
                            tracing::warn!(
                                %login,
                                %agent_id,
                                variant = %variant_type,
                                ?error,
                                "ui ws: rejected invalid or non-request message"
                            );
                            let _ = tx.send(UiMessage::PermissionDenied {
                                agent_id: agent_id.clone(),
                                variant_type,
                                reason: "message is not an allowed UI request".to_string(),
                            });
                            continue;
                        }
                    };
                    // EE ACL enforcement (skip for admins)
                    if ee::ee_active() && !auth::is_dev_mode() {
                        let is_admin = matches!(
                            crate::db::get_user(&state.db, &login).await,
                            Ok(Some(row)) if row.role == "admin"
                        );
                        if !is_admin {
                            if let Some(action) = security.action {
                                if !ee_check_permission(
                                    &login,
                                    action,
                                    &agent_id,
                                    Some(client_ip.as_str()),
                                )
                                .await
                                {
                                    let _ = tx.send(UiMessage::PermissionDenied {
                                        agent_id: agent_id.clone(),
                                        variant_type: action.to_string(),
                                        reason: format!("denied: {action}"),
                                    });
                                    continue;
                                }
                            }
                        }
                    }
                    // CE RBAC over the WebSocket plane. The HTTP rbac
                    // middleware doesn't run here — without this gate
                    // a viewer with a verified session could
                    // ControlServiceRequest, AptUpgradeRequest,
                    // DockerContainerActionRequest, send terminal
                    // keystrokes, etc., bypassing the entire role
                    // model. Re-resolve the role from the DB on every
                    // mutating message so a freshly-demoted admin is
                    // blocked immediately.
                    if !auth::is_dev_mode() && security.class.requires_admin() {
                        let role_str = match crate::db::get_user(&state.db, &login).await {
                            Ok(Some(row)) => row.role,
                            _ => "viewer".to_string(),
                        };
                        if auth::Role::parse(&role_str) != auth::Role::Admin {
                            tracing::warn!(
                                %login, %agent_id, variant = %variant_type,
                                "ui ws: rejected mutating message from non-admin"
                            );
                            crate::db::record_audit(
                                &state.db,
                                now_unix(),
                                Some(&login),
                                Some(&agent_id),
                                "ws.send_to_agent.denied",
                                false,
                                Some(&format!("role={role_str} variant={variant_type}")),
                            )
                            .await;
                            // Tell the UI the request was rejected so the
                            // calling panel doesn't sit in "waiting for
                            // output…" forever. Best-effort; if the send
                            // fails the client is already gone.
                            let _ = tx.send(UiMessage::PermissionDenied {
                                agent_id: agent_id.clone(),
                                variant_type,
                                reason: "admin only".to_string(),
                            });
                            continue;
                        }
                    }
                    // EE command-approval gate (dual control). A discrete action
                    // that matches an approval rule is HELD: we stash the
                    // serialized message in EE and tell the UI it's pending — a
                    // second admin approves in the Approvals tab, then EE calls
                    // back to /internal/execute-approved to run it. Interactive
                    // Interactive streams and reads are never approval-held.
                    if ee::ee_active() && !auth::is_dev_mode() {
                        if security.class.requires_approval() {
                            let Some(action) = security.action else {
                                let _ = tx.send(UiMessage::PermissionDenied {
                                    agent_id: agent_id.clone(),
                                    variant_type: variant_type.clone(),
                                    reason: "request has no approval action mapping".to_string(),
                                });
                                continue;
                            };
                            let payload = serde_json::to_string(&message).unwrap_or_default();
                            match ee_check_approval(&login, action, &agent_id, &payload).await {
                                Ok(None) => { /* no rule matched — run it now */ }
                                Ok(Some(req_id)) => {
                                    crate::db::record_audit(
                                        &state.db,
                                        now_unix(),
                                        Some(&login),
                                        Some(&agent_id),
                                        "ws.approval.held",
                                        true,
                                        Some(&format!("action={action} request={req_id}")),
                                    )
                                    .await;
                                    let _ = tx.send(UiMessage::PermissionDenied {
                                        agent_id: agent_id.clone(),
                                        variant_type: "approval_pending".to_string(),
                                        reason: format!(
                                            "held for approval — request #{req_id}; a second admin must approve it in the Approvals tab"
                                        ),
                                    });
                                    continue;
                                }
                                Err(()) => {
                                    let _ = tx.send(UiMessage::PermissionDenied {
                                        agent_id: agent_id.clone(),
                                        variant_type: "approval_unavailable".to_string(),
                                        reason: "approval system unavailable — action blocked (fail-closed)".to_string(),
                                    });
                                    continue;
                                }
                            }
                        }
                    }
                    if let Some(operation) = operation_routing::ui_operation(&message) {
                        use operation_routing::UiOperation;
                        let allowed = {
                            let mut owners = state.operation_owners.lock().await;
                            match &operation {
                                UiOperation::Start(key) => {
                                    owners.claim(&agent_id, key.clone(), client_id)
                                }
                                UiOperation::Use(key) => {
                                    owners.owner(&agent_id, key) == Some(client_id)
                                }
                                UiOperation::Stop(key) => owners.release(&agent_id, key, client_id),
                            }
                        };
                        if !allowed {
                            let _ = tx.send(UiMessage::PermissionDenied {
                                agent_id: agent_id.clone(),
                                variant_type: variant_type.clone(),
                                reason: "operation is owned by another client".to_string(),
                            });
                            continue;
                        }
                    }
                    // Recording tap: terminal session lifecycle + INPUT (user→agent).
                    match &message {
                        Message::StartTerminalRequest { session_id } => {
                            state
                                .recorder
                                .start(session_id, &agent_id, &login, "host")
                                .await;
                            rec_sessions.insert(session_id.clone());
                        }
                        Message::TerminalData { session_id, data } => {
                            state.recorder.frame(session_id, "i", data).await;
                        }
                        Message::StopTerminalRequest { session_id } => {
                            state.recorder.stop(session_id).await;
                            rec_sessions.remove(session_id);
                        }
                        _ => {}
                    }
                    if let Some(entry) = state.agents.lock().await.get(&agent_id) {
                        let _ = entry.tx.send(message);
                    }
                }
                _ => {}
            }
        }
    }

    send_task.abort();
    state.ui_clients.lock().await.remove(&client_id);
    state
        .operation_owners
        .lock()
        .await
        .release_client(client_id);
    // Close any recordings still open for this client (abrupt disconnect).
    for sid in rec_sessions {
        state.recorder.stop(&sid).await;
    }
    tracing::info!(client_id, "ui client disconnected");
}

pub(crate) async fn ee_fetch_agent_access(
    login: &str,
    client_ip: &str,
    agents: &[String],
) -> AgentAccess {
    // CE-only deployments have no EE ACL feature to enforce.
    let Some(ee_url) = ee::ee_sidecar_url() else {
        return AgentAccess::Unrestricted;
    };
    let url = format!("{}/api/ee/acl/user-agents", ee_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "login": login,
        "client_ip": client_ip,
        "agents": agents,
    });
    let resp = match internal_auth::send_json(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &url,
        &body,
        login,
        "viewer",
        std::time::Duration::from_secs(5),
    )
    .await
    {
        Ok(r) if r.status.is_success() => r,
        other => {
            tracing::warn!(
                login = %login,
                ok = other.is_ok(),
                "EE agent-access fetch failed; failing closed"
            );
            return AgentAccess::Restricted {
                allowed_agents: HashSet::new(),
            };
        }
    };
    resp.json().unwrap_or_else(|_| AgentAccess::Restricted {
        allowed_agents: HashSet::new(),
    })
}

async fn ui_agent_access(
    state: &AppState,
    login: &str,
    client_ip: &str,
    agents: &[String],
) -> AgentAccess {
    let is_admin = matches!(
        crate::db::get_user(&state.db, login).await,
        Ok(Some(row)) if row.role == "admin"
    );
    if is_admin {
        AgentAccess::Unrestricted
    } else {
        ee_fetch_agent_access(login, client_ip, agents).await
    }
}

pub(crate) fn agent_allowed_by_access(agent: &str, access: &AgentAccess) -> bool {
    match access {
        AgentAccess::Unrestricted => true,
        AgentAccess::Restricted { allowed_agents } => allowed_agents.contains(agent),
    }
}

fn merge_exact_agent_access(cached: &mut AgentAccess, agent: &str, exact: &AgentAccess) {
    let AgentAccess::Restricted { allowed_agents } = exact else {
        *cached = AgentAccess::Unrestricted;
        return;
    };
    let allowed = allowed_agents.contains(agent);
    match cached {
        AgentAccess::Unrestricted => {
            *cached = AgentAccess::Restricted {
                allowed_agents: allowed.then(|| agent.to_string()).into_iter().collect(),
            };
        }
        AgentAccess::Restricted { allowed_agents } => {
            if allowed {
                allowed_agents.insert(agent.to_string());
            } else {
                allowed_agents.remove(agent);
            }
        }
    }
}

/// Ask EE whether this discrete action needs approval, passing the serialized
/// message as the payload to replay on approval. Returns:
///   Ok(None)      — no rule matched; proceed and run the command now.
///   Ok(Some(id))  — a pending approval request was created; HOLD the command.
///   Err(())       — EE configured but unreachable / errored; caller fails closed.
async fn ee_check_approval(
    login: &str,
    action: &str,
    agent_id: &str,
    payload: &str,
) -> Result<Option<i64>, ()> {
    // No EE configured = no approval system = proceed.
    let Some(ee_url) = ee::ee_sidecar_url() else {
        return Ok(None);
    };
    let url = format!("{}/api/ee/approvals/check", ee_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "login": login,
        "action": action,
        "resource": agent_id,
        "agent_id": agent_id,
        "payload": payload,
    });
    match internal_auth::send_json(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &url,
        &body,
        login,
        "",
        std::time::Duration::from_secs(3),
    )
    .await
    {
        Ok(resp) if resp.status.is_success() => {
            let data: serde_json::Value = resp.json().unwrap_or_default();
            if data["requires_approval"].as_bool().unwrap_or(false) {
                Ok(Some(data["request_id"].as_i64().unwrap_or(0)))
            } else {
                Ok(None)
            }
        }
        _ => {
            tracing::warn!(%login, %action, "EE approval check failed; holding (fail closed)");
            Err(())
        }
    }
}

async fn ee_check_permission(
    login: &str,
    action: &str,
    resource: &str,
    client_ip: Option<&str>,
) -> bool {
    let Some(ee_url) = ee::ee_sidecar_url() else {
        return true;
    };
    let url = format!("{}/api/ee/acl/evaluate", ee_url.trim_end_matches('/'));
    // client_ip lets ACL rules with `conditions.ip` actually match (the EE
    // evaluator ignores the condition when no IP is supplied).
    let body = serde_json::json!({
        "login": login,
        "action": action,
        "resource": resource,
        "client_ip": client_ip,
    });
    match internal_auth::send_json(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &url,
        &body,
        login,
        "",
        std::time::Duration::from_secs(3),
    )
    .await
    {
        Ok(resp) if resp.status.is_success() => {
            let data: serde_json::Value = resp.json().unwrap_or_default();
            // Malformed / missing `allowed` => deny (fail closed).
            data["allowed"].as_bool().unwrap_or(false)
        }
        _ => {
            // EE is configured but unreachable / non-2xx: FAIL CLOSED.
            // (CE-only deploys returned early above with allow, so this
            // only denies when the EE policy engine was expected to answer.)
            tracing::warn!(
                login = %login,
                action = %action,
                "EE permission check failed; denying (fail closed)"
            );
            false
        }
    }
}

fn ee_filter_agent_list(
    agents: Vec<String>,
    capabilities: HashMap<String, Vec<String>>,
    access: &AgentAccess,
) -> (Vec<String>, HashMap<String, Vec<String>>) {
    if matches!(access, AgentAccess::Unrestricted) {
        return (agents, capabilities);
    }
    let filtered: Vec<String> = agents
        .into_iter()
        .filter(|a| agent_allowed_by_access(a, access))
        .collect();
    let filtered_caps: HashMap<String, Vec<String>> = capabilities
        .into_iter()
        .filter(|(k, _)| agent_allowed_by_access(k, access))
        .collect();
    (filtered, filtered_caps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_agent_access_has_no_pattern_or_null_ambiguity() {
        let restricted = AgentAccess::Restricted {
            allowed_agents: HashSet::from(["prod-db-1".into(), "agent-exact".into()]),
        };

        assert!(agent_allowed_by_access("prod-db-1", &restricted));
        assert!(!agent_allowed_by_access("prod-db-2", &restricted));
        assert!(!agent_allowed_by_access("dev-db-1", &restricted));
        assert!(agent_allowed_by_access(
            "anything",
            &AgentAccess::Unrestricted
        ));
    }

    #[test]
    fn exact_access_refresh_updates_only_the_checked_agent() {
        let mut cached = AgentAccess::Restricted {
            allowed_agents: HashSet::from(["agent-a".into(), "agent-b".into()]),
        };

        merge_exact_agent_access(
            &mut cached,
            "agent-c",
            &AgentAccess::Restricted {
                allowed_agents: HashSet::from(["agent-c".into()]),
            },
        );
        assert!(agent_allowed_by_access("agent-a", &cached));
        assert!(agent_allowed_by_access("agent-c", &cached));

        merge_exact_agent_access(
            &mut cached,
            "agent-a",
            &AgentAccess::Restricted {
                allowed_agents: HashSet::new(),
            },
        );
        assert!(!agent_allowed_by_access("agent-a", &cached));
        assert!(agent_allowed_by_access("agent-b", &cached));
    }

    #[test]
    fn restricted_exact_refresh_replaces_stale_unrestricted_cache() {
        let mut cached = AgentAccess::Unrestricted;
        merge_exact_agent_access(
            &mut cached,
            "agent-a",
            &AgentAccess::Restricted {
                allowed_agents: HashSet::from(["agent-a".into()]),
            },
        );

        assert!(agent_allowed_by_access("agent-a", &cached));
        assert!(!agent_allowed_by_access("agent-b", &cached));
    }
}
