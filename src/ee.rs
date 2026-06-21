use axum::{
    extract::{OriginalUri, Request, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{auth, db, AppState};

pub fn ee_sidecar_url() -> Option<String> {
    std::env::var("EE_SIDECAR_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn ee_active() -> bool {
    ee_sidecar_url().is_some()
}

fn internal_secret() -> Option<String> {
    std::env::var("EE_INTERNAL_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

fn ct_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn verify_internal_auth(headers: &HeaderMap) -> bool {
    let Some(expected) = internal_secret() else {
        return false;
    };
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|t| ct_eq(t.trim(), &expected))
        .unwrap_or(false)
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/resolve", post(auth_resolve_handler))
        .route("/seat-limit", post(seat_limit_handler))
        .route("/agents", get(agents_handler))
        .route("/send-to-agent", post(send_to_agent_handler))
        .route("/execute-approved", post(execute_approved_handler))
        .route("/exec-command", post(exec_command_handler))
        .route("/audit", post(internal_audit_handler))
}

fn default_audit_ok() -> bool {
    true
}

#[derive(Deserialize)]
struct InternalAuditRequest {
    actor: Option<String>,
    agent_id: Option<String>,
    kind: String,
    #[serde(default = "default_audit_ok")]
    ok: bool,
    detail: Option<String>,
}

/// Let the EE sidecar record an entry in the CE audit log (e.g. a break-glass
/// request/approval), so it surfaces at `/activity` and streams to any SIEM
/// via the EE audit archiver. Internal-secret gated.
async fn internal_audit_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<InternalAuditRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    db::record_audit(
        &state.db,
        crate::now_unix(),
        body.actor.as_deref(),
        body.agent_id.as_deref(),
        &body.kind,
        body.ok,
        body.detail.as_deref(),
    )
    .await;
    (StatusCode::OK, "ok").into_response()
}

fn default_exec_timeout() -> u64 {
    60
}

#[derive(Deserialize)]
struct ExecCommandRequest {
    agent_id: String,
    command: String,
    #[serde(default = "default_exec_timeout")]
    timeout_secs: u64,
}

/// Run a one-shot command on a connected agent and return its captured output.
/// This is what makes EE runbooks actually execute: EE POSTs the step's command
/// here, CE dispatches a `RunCommandRequest` to the agent and awaits the
/// correlated `RunCommandResponse`. Internal-secret gated; *which* commands run
/// is gated upstream by EE's runbook allow-list + the ACL. A non-zero exit is a
/// normal 200 (the runbook treats it as a failed step); only infrastructure
/// failures (agent offline, timeout) are non-2xx.
async fn exec_command_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<ExecCommandRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let timeout_secs = body.timeout_secs.clamp(1, 3600);
    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx_os, rx_os) = tokio::sync::oneshot::channel();
    state.pending_exec.lock().await.insert(request_id.clone(), tx_os);

    let dispatched = {
        let agents = state.agents.lock().await;
        match agents.get(&body.agent_id) {
            Some(entry) => entry
                .tx
                .send(shared::Message::RunCommandRequest {
                    request_id: request_id.clone(),
                    command: body.command.clone(),
                    timeout_secs,
                })
                .is_ok(),
            None => false,
        }
    };
    if !dispatched {
        state.pending_exec.lock().await.remove(&request_id);
        return (StatusCode::NOT_FOUND, "agent not connected").into_response();
    }

    // Wait a little past the agent-side timeout for the round-trip.
    let wait = std::time::Duration::from_secs(timeout_secs + 10);
    match tokio::time::timeout(wait, rx_os).await {
        Ok(Ok(shared::Message::RunCommandResponse { exit_code, stdout, stderr, error, .. })) => {
            axum::Json(serde_json::json!({
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "error": error,
            }))
            .into_response()
        }
        Ok(_) => (StatusCode::BAD_GATEWAY, "unexpected exec response").into_response(),
        Err(_) => {
            state.pending_exec.lock().await.remove(&request_id);
            (StatusCode::GATEWAY_TIMEOUT, "agent did not respond in time").into_response()
        }
    }
}

#[derive(Deserialize)]
struct ExecuteApprovedRequest {
    agent_id: String,
    /// The original agent Message, serialized to JSON, that EE stashed when the
    /// command was held for approval. We deserialize and deliver it verbatim.
    payload: String,
}

/// Run a command that an EE approval workflow just approved. When CE held a
/// discrete action for approval it serialized the agent Message into the
/// approval request's `payload`; on approval EE calls back here to actually
/// execute it. Internal-secret gated. The original ACL/RBAC checks already
/// passed at request time — approval is the *additional* dual-control gate.
async fn execute_approved_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<ExecuteApprovedRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let message: shared::Message = match serde_json::from_str(&body.payload) {
        Ok(m) => m,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("bad approved payload: {e}")).into_response()
        }
    };
    let agents = state.agents.lock().await;
    match agents.get(&body.agent_id) {
        Some(entry) if entry.tx.send(message).is_ok() => (StatusCode::OK, "executed").into_response(),
        Some(_) => (StatusCode::BAD_GATEWAY, "agent send channel closed").into_response(),
        None => (StatusCode::NOT_FOUND, "agent not connected").into_response(),
    }
}

#[derive(Deserialize)]
struct SendToAgentRequest {
    agent_id: String,
    message: shared::Message,
}

/// Forward an EE-originated message (e.g. a DriftSnapshotRequest from the drift
/// scheduler/trigger) to a connected agent's WebSocket. EE has no direct agent
/// link, so it round-trips through CE. 404 when the agent isn't connected so the
/// caller can surface "agent offline".
async fn send_to_agent_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<SendToAgentRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let agents = state.agents.lock().await;
    match agents.get(&body.agent_id) {
        Some(entry) if entry.tx.send(body.message).is_ok() => {
            (StatusCode::OK, "sent").into_response()
        }
        Some(_) => (StatusCode::BAD_GATEWAY, "agent send channel closed").into_response(),
        None => (StatusCode::NOT_FOUND, "agent not connected").into_response(),
    }
}

#[derive(Deserialize)]
struct AuthResolveRequest {
    login: String,
    #[serde(default = "default_role")]
    role: String,
    #[serde(default)]
    mfa: bool,
}

fn default_role() -> String {
    "viewer".into()
}

#[derive(Serialize)]
struct AuthResolveResponse {
    token: String,
}

async fn auth_resolve_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<AuthResolveRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let role = match body.role.as_str() {
        "admin" | "viewer" => body.role.as_str(),
        _ => "viewer",
    };

    let now = crate::now_unix();
    let seat_limit = db::seat_limit(&state.db).await;
    match db::upsert_login_with_seat_check(&state.db, &body.login, role, now, seat_limit).await {
        Ok(db::SeatedUpsert::SeatCapReached) => {
            return (StatusCode::FORBIDDEN, "seat cap reached").into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "internal auth resolve: db error");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
        Ok(_) => {}
    }

    let token = auth::issue_internal_jwt(&body.login, role, body.mfa);
    (StatusCode::OK, axum::Json(AuthResolveResponse { token })).into_response()
}

#[derive(Deserialize)]
struct SeatLimitRequest {
    seats: i64,
}

async fn seat_limit_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<SeatLimitRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    match db::set_ee_seat_limit(&state.db, body.seats).await {
        Ok(_) => {
            tracing::info!(seats = body.seats, "EE seat limit updated");
            (StatusCode::OK, "ok").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to set EE seat limit");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

#[derive(Serialize)]
struct AgentInfo {
    agent_id: String,
    capabilities: Vec<String>,
}

async fn agents_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let map = state.agents.lock().await;
    let agents: Vec<AgentInfo> = map
        .iter()
        .map(|(id, entry)| AgentInfo {
            agent_id: id.clone(),
            capabilities: entry.capabilities.clone(),
        })
        .collect();
    (StatusCode::OK, axum::Json(agents)).into_response()
}

pub async fn forward_drift_snapshot(
    agent_id: &str,
    snapshot_id: &str,
    packages: &[shared::DriftPackage],
    services: &[shared::DriftService],
    containers: &[shared::DriftContainer],
    configs: &[shared::DriftConfigFile],
) {
    let Some(ee_url) = ee_sidecar_url() else {
        return;
    };
    let secret = internal_secret().unwrap_or_default();
    let url = format!(
        "{}/internal/drift-snapshot",
        ee_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "agent_id": agent_id,
        "snapshot_id": snapshot_id,
        "packages": packages,
        "services": services,
        "containers": containers,
        "configs": configs,
        "triggered_by": "agent",
    });

    let client = reqwest::Client::new();
    if let Err(e) = client
        .post(&url)
        .bearer_auth(&secret)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        tracing::warn!(error = %e, "failed to forward drift snapshot to EE");
    }
}

pub async fn ee_proxy_handler(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    OriginalUri(orig_uri): OriginalUri,
    req: Request,
) -> impl IntoResponse {
    let Some(ee_url) = ee_sidecar_url() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    };
    // Resolve the authenticated principal. RBAC already ran in the /api
    // layer; this also gives us the DB-resolved role. EE trusts the
    // identity we forward precisely because this path is only reachable
    // behind CE auth + the EE-side internal-secret gate.
    let claims = match auth::current_user(&jar, &state.db).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let secret = internal_secret().unwrap_or_default();

    // The proxy route is nested under `/api`, so `req.uri()` is
    // prefix-stripped — forward the FULL original path (`/api/ee/...`)
    // which is what the EE sidecar serves.
    let path = orig_uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| orig_uri.path());
    let url = format!("{}{}", ee_url.trim_end_matches('/'), path);
    let method = req.method().clone();

    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "body too large").into_response(),
    };

    let client = reqwest::Client::new();
    // Build a FRESH upstream request: client-supplied headers (including
    // any spoofed x-shellfleet-* / Authorization) are deliberately NOT
    // copied. We attach the internal bearer plus the CE-verified identity.
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    )
    .bearer_auth(&secret)
    .header("x-shellfleet-login", claims.sub.as_str())
    .header("x-shellfleet-role", claims.role.as_str())
    .timeout(std::time::Duration::from_secs(30));

    if !body_bytes.is_empty() {
        builder = builder
            .header("content-type", "application/json")
            .body(body_bytes.to_vec());
    }

    match builder.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "EE proxy failed");
            (StatusCode::BAD_GATEWAY, "EE sidecar unavailable").into_response()
        }
    }
}
