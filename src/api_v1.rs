use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{db, ee, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/{name}", get(get_agent))
        .route("/agents/{name}/exec", post(exec_command))
        .route("/agents/{name}/service/{svc}/{action}", post(control_service))
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
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Ok((_login, _role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "agent:View").await {
        return (code, msg).into_response();
    }
    let map = state.agents.lock().await;
    let agents: Vec<AgentSummary> = map
        .iter()
        .map(|(id, entry)| AgentSummary {
            agent_id: id.clone(),
            capabilities: entry.capabilities.clone(),
        })
        .collect();
    axum::Json(agents).into_response()
}

async fn get_agent(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Ok((_login, _role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "agent:View").await {
        return (code, msg).into_response();
    }
    let agent_id = format!("{name}-id");
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

#[derive(Serialize)]
struct ExecResponse {
    stdout: String,
    duration_ms: u64,
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
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "agent:Exec").await {
        return (code, msg).into_response();
    }

    let agent_id = format!("{name}-id");
    let timeout = body.timeout_secs.clamp(1, 300);

    let entry = {
        let map = state.agents.lock().await;
        map.get(&agent_id).cloned()
    };
    let Some(entry) = entry else {
        return (StatusCode::NOT_FOUND, "agent not connected").into_response();
    };

    let session_id = format!("api-exec-{}", crate::now_unix());

    let _ = entry.tx.send(shared::Message::StartTerminalRequest {
        session_id: session_id.clone(),
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmd_bytes = format!("{}\nexit\n", body.command).into_bytes();
    let _ = entry.tx.send(shared::Message::TerminalData {
        session_id: session_id.clone(),
        data: cmd_bytes,
    });

    let start = std::time::Instant::now();
    tokio::time::sleep(std::time::Duration::from_secs(timeout)).await;
    let _ = entry.tx.send(shared::Message::StopTerminalRequest {
        session_id,
    });

    let duration_ms = start.elapsed().as_millis() as u64;

    db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&login),
        Some(&agent_id),
        "api.exec",
        true,
        Some(&format!("cmd={}", body.command)),
    )
    .await;

    axum::Json(ExecResponse {
        stdout: "(exec via API — full output collection requires pending_exec infrastructure)"
            .to_string(),
        duration_ms,
    })
    .into_response()
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
        return (StatusCode::BAD_REQUEST, "action must be start, stop, or restart").into_response();
    }

    let service_action = match action.as_str() {
        "start" => "service:Start",
        "stop" => "service:Stop",
        "restart" => "service:Restart",
        _ => unreachable!(),
    };
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, service_action).await {
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

async fn list_users(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Ok((_login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if role != "admin" {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "user:List").await {
        return (code, msg).into_response();
    }
    match db::list_users(&state.db).await {
        Ok(users) => axum::Json(users).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

async fn get_acl(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Ok((_login, _role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "acl:View").await {
        return (code, msg).into_response();
    }
    if !ee::ee_active() {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    }
    let url = format!(
        "{}/api/ee/acl/document",
        ee::ee_sidecar_url().unwrap()
    );
    let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
    match reqwest::Client::new()
        .get(&url)
        .bearer_auth(&secret)
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
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
    let Ok((_login, role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if role != "admin" {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "acl:Write").await {
        return (code, msg).into_response();
    }
    if !ee::ee_active() {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    }
    let url = format!(
        "{}/api/ee/acl/document",
        ee::ee_sidecar_url().unwrap()
    );
    let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
    match reqwest::Client::new()
        .put(&url)
        .bearer_auth(&secret)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
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
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "audit:Read").await {
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
    let Ok((_login, _role)) = require_api_key(&headers) else {
        return (StatusCode::UNAUTHORIZED, "API key required").into_response();
    };
    if let Err((code, msg)) = crate::api_auth::require_key_action(&headers, &state.db, "metric:Query").await {
        return (code, msg).into_response();
    }
    if !ee::ee_active() {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    }
    let url = format!(
        "{}/api/ee/metrics/query",
        ee::ee_sidecar_url().unwrap()
    );
    let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
    match reqwest::Client::new()
        .post(&url)
        .bearer_auth(&secret)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = resp.text().await.unwrap_or_default();
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
