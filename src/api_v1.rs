use axum::{
    Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use crate::{AppState, db, ee};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/{name}", get(get_agent))
        .route("/agents/{name}/exec", post(exec_command))
        .route(
            "/agents/{name}/service/{svc}/{action}",
            post(control_service),
        )
        .route("/users", get(list_users))
        .route("/policies/acl", get(get_acl).put(put_acl))
        .route("/audit", get(export_audit))
        .route("/metrics/query", post(query_metrics))
}

fn caller_login(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-api-key-login")
        .and_then(|h| h.to_str().ok())
        .map(String::from)
}

fn caller_role(headers: &HeaderMap) -> String {
    headers
        .get("x-api-key-role")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("viewer")
        .to_string()
}

fn require_api_key(headers: &HeaderMap) -> Result<(String, String), StatusCode> {
    let login = caller_login(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let role = caller_role(headers);
    Ok((login, role))
}

#[derive(Serialize)]
struct AgentSummary {
    agent_id: String,
    capabilities: Vec<String>,
}

async fn list_agents(
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "agent:View").await
    {
        return (code, msg).into_response();
    }
    let summaries: Vec<AgentSummary> = state
        .agents
        .lock()
        .await
        .iter()
        .map(|(id, entry)| AgentSummary {
            agent_id: id.clone(),
            capabilities: entry.capabilities.clone(),
        })
        .collect();
    let agent_ids = summaries
        .iter()
        .map(|a| a.agent_id.clone())
        .collect::<Vec<_>>();
    let client_ip = crate::throttle::real_client_ip(&headers, Some(peer.ip()));
    let access = if role == "admin" {
        crate::AgentAccess::Unrestricted
    } else {
        crate::ee_fetch_agent_access(&login, &client_ip, &agent_ids).await
    };
    let visible = summaries
        .into_iter()
        .filter(|agent| crate::agent_allowed_by_access(&agent.agent_id, &access))
        .collect::<Vec<_>>();
    axum::Json(visible).into_response()
}

async fn get_agent(
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "agent:View").await
    {
        return (code, msg).into_response();
    }
    let agent_id = format!("{name}-id");
    let client_ip = crate::throttle::real_client_ip(&headers, Some(peer.ip()));
    let access = if role == "admin" {
        crate::AgentAccess::Unrestricted
    } else {
        crate::ee_fetch_agent_access(&login, &client_ip, std::slice::from_ref(&agent_id)).await
    };
    if !crate::agent_allowed_by_access(&agent_id, &access) {
        return (StatusCode::NOT_FOUND, "agent not found").into_response();
    }
    let map = state.agents.lock().await;
    match map.get(&agent_id) {
        Some(entry) => axum::Json(AgentSummary {
            agent_id,
            capabilities: entry.capabilities.clone(),
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "agent not found").into_response(),
    }
}

#[derive(Deserialize)]
struct ExecRequest {
    command: String,
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

async fn exec_command(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<ExecRequest>,
) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if role != "admin" {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "agent:Exec").await
    {
        return (code, msg).into_response();
    }

    let agent_id = format!("{name}-id");
    let timer = Instant::now();
    let timeout_secs = body.timeout_secs.clamp(1, 300);

    let pending = match crate::agent_dispatch::dispatch_run_command(
        &state,
        &agent_id,
        body.command.clone(),
        timeout_secs,
    )
    .await
    {
        Ok(p) => p,
        Err(crate::agent_dispatch::DispatchError::AgentNotFound) => {
            api_exec_audit(
                &state,
                &login,
                &agent_id,
                &body.command,
                false,
                None,
                Some("agent not found"),
                false,
                false,
                &timer,
            )
            .await;
            return (StatusCode::NOT_FOUND, "agent not connected").into_response();
        }
        Err(crate::agent_dispatch::DispatchError::AgentDisconnected) => {
            api_exec_audit(
                &state,
                &login,
                &agent_id,
                &body.command,
                false,
                None,
                Some("agent disconnected"),
                false,
                false,
                &timer,
            )
            .await;
            return (StatusCode::BAD_GATEWAY, "agent disconnected").into_response();
        }
    };

    let outer = std::time::Duration::from_secs(
        timeout_secs + shared::EXEC_READER_GRACE_SECS + shared::EXEC_TRANSPORT_MARGIN_SECS,
    );
    let resp = match tokio::time::timeout(outer, pending.rx).await {
        Ok(Ok(msg)) => msg,
        Ok(Err(_)) => {
            api_exec_audit(
                &state,
                &login,
                &agent_id,
                &body.command,
                false,
                None,
                Some("agent disconnected mid-exec"),
                false,
                false,
                &timer,
            )
            .await;
            return (StatusCode::BAD_GATEWAY, "agent disconnected").into_response();
        }
        Err(_) => {
            api_exec_audit(
                &state,
                &login,
                &agent_id,
                &body.command,
                false,
                None,
                None,
                false,
                true,
                &timer,
            )
            .await;
            return (StatusCode::GATEWAY_TIMEOUT, "agent did not respond in time").into_response();
        }
    };

    match &resp {
        shared::Message::RunCommandResponse {
            exit_code,
            stdout,
            stderr,
            error,
            truncated,
            timed_out,
            duration_ms,
            ..
        } => {
            let is_legacy =
                *exit_code == -1 && error.as_deref().unwrap_or("").contains("timed out");
            let effective_timed_out = *timed_out || is_legacy;

            api_exec_audit(
                &state,
                &login,
                &agent_id,
                &body.command,
                *exit_code == 0 && !effective_timed_out,
                Some(*exit_code),
                error.as_deref(),
                *truncated,
                effective_timed_out,
                &timer,
            )
            .await;

            let dur = if *duration_ms > 0 {
                *duration_ms
            } else {
                timer.elapsed().as_millis() as u64
            };
            let http_status = if effective_timed_out {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::OK
            };
            let ec = if *exit_code == -1 {
                serde_json::Value::Null
            } else {
                serde_json::json!(*exit_code)
            };

            let json = serde_json::json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": ec,
                "error": error,
                "truncated": truncated,
                "timed_out": effective_timed_out,
                "duration_ms": dur,
            });
            (http_status, axum::Json(json)).into_response()
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "unexpected response").into_response(),
    }
}

async fn api_exec_audit(
    state: &Arc<AppState>,
    login: &str,
    agent_id: &str,
    cmd: &str,
    ok: bool,
    exit_code: Option<i32>,
    err_detail: Option<&str>,
    truncated: bool,
    timed_out: bool,
    elapsed: &Instant,
) {
    let dur = elapsed.elapsed().as_millis() as u64;
    let ec = exit_code
        .map(|c| format!(" exit_code={c}"))
        .unwrap_or_default();
    let err = err_detail.map(|e| format!(" err={e}")).unwrap_or_default();
    // Sanitize command for audit — strip control characters and truncate
    // to prevent log injection and unbounded audit rows.
    let safe_cmd: String = cmd.chars().filter(|c| !c.is_control()).take(512).collect();
    let detail =
        format!("cmd={safe_cmd}{ec} truncated={truncated} timed_out={timed_out} dur={dur}ms{err}");
    crate::db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(login),
        Some(agent_id),
        "api.exec",
        ok,
        Some(&detail),
    )
    .await;
}

async fn control_service(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path((name, svc, action)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if role != "admin" {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }

    let valid_actions = ["start", "stop", "restart"];
    if !valid_actions.contains(&action.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            "action must be start, stop, or restart",
        )
            .into_response();
    }

    let service_action = match action.as_str() {
        "start" => "service:Start",
        "stop" => "service:Stop",
        "restart" => "service:Restart",
        _ => unreachable!(),
    };
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, service_action).await
    {
        return (code, msg).into_response();
    }

    let agent_id = format!("{name}-id");
    let entry = {
        let map = state.agents.lock().await;
        map.get(&agent_id).cloned()
    };
    let Some(entry) = entry else {
        return (StatusCode::NOT_FOUND, "agent not connected").into_response();
    };

    let _ = entry.tx.send(shared::Message::ControlServiceRequest {
        name: svc.clone(),
        action: action.clone(),
    });

    db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&login),
        Some(&agent_id),
        &format!("api.service.{action}"),
        true,
        Some(&format!("service={svc}")),
    )
    .await;

    (StatusCode::ACCEPTED, format!("service {action} sent")).into_response()
}

async fn list_users(headers: HeaderMap, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let Ok((_login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if role != "admin" {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "user:List").await
    {
        return (code, msg).into_response();
    }
    match db::list_users(&state.db).await {
        Ok(users) => axum::Json(users).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

async fn get_acl(headers: HeaderMap, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "acl:View").await
    {
        return (code, msg).into_response();
    }
    if !ee::ee_active() {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    }
    let url = format!("{}/api/ee/acl/document", ee::ee_sidecar_url().unwrap());
    match crate::internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::GET,
        &url,
        Vec::new(),
        "",
        &login,
        &role,
        std::time::Duration::from_secs(30),
    )
    .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text();
            (status, body).into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "EE unavailable").into_response(),
    }
}

async fn put_acl(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    body: String,
) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if role != "admin" {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "acl:Write").await
    {
        return (code, msg).into_response();
    }
    if !ee::ee_active() {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    }
    let url = format!("{}/api/ee/acl/document", ee::ee_sidecar_url().unwrap());
    match crate::internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::PUT,
        &url,
        body.into_bytes(),
        "application/json",
        &login,
        &role,
        std::time::Duration::from_secs(30),
    )
    .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text();
            (status, body).into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "EE unavailable").into_response(),
    }
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<i64>,
}

async fn export_audit(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    let Ok((_login, _role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "audit:Read").await
    {
        return (code, msg).into_response();
    }
    let limit = q.limit.unwrap_or(200).clamp(1, 10000);
    match db::recent_audit(&state.db, limit).await {
        Ok(rows) => axum::Json(rows).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

async fn query_metrics(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    body: String,
) -> impl IntoResponse {
    let Ok((login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) =
        crate::api_auth::require_key_action(&headers, &state.db, "metric:Query").await
    {
        return (code, msg).into_response();
    }
    if !ee::ee_active() {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    }
    let url = format!("{}/api/ee/metrics/query", ee::ee_sidecar_url().unwrap());
    match crate::internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &url,
        body.into_bytes(),
        "application/json",
        &login,
        &role,
        std::time::Duration::from_secs(30),
    )
    .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = resp.text();
            (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                text,
            )
                .into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "EE unavailable").into_response(),
    }
}
