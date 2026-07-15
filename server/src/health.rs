//! Health probes — per-agent HTTP/TCP probes the agent runs on a fixed
//! interval. Mutations push the agent's full probe set; the agent
//! reports state changes back.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use shared::{HealthProbeKind, HealthProbeSpec, Message};
use std::sync::Arc;

use crate::{AppState, auth, db};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler).post(upsert_handler))
        .route("/{id}", delete(delete_handler))
        .route("/snapshot", get(snapshot_handler))
}

#[derive(Serialize)]
struct ProbeOut {
    id: i64,
    agent_id: String,
    name: String,
    kind: String,
    target: String,
    interval_secs: i64,
    timeout_secs: i64,
    expect_status: Option<i64>,
    expect_body: Option<String>,
    enabled: bool,
    last_run_at: i64,
    last_state: Option<String>,
    last_latency_ms: Option<i64>,
    last_detail: Option<String>,
    updated_at: i64,
    env: Vec<String>,
}

fn to_out(row: db::HealthProbeRow) -> ProbeOut {
    let env: Vec<String> = serde_json::from_str(&row.env_json).unwrap_or_default();
    ProbeOut {
        id: row.id,
        agent_id: row.agent_id,
        name: row.name,
        kind: row.kind,
        target: row.target,
        interval_secs: row.interval_secs,
        timeout_secs: row.timeout_secs,
        expect_status: row.expect_status,
        expect_body: row.expect_body,
        enabled: row.enabled != 0,
        last_run_at: row.last_run_at,
        last_state: row.last_state,
        last_latency_ms: row.last_latency_ms,
        last_detail: row.last_detail,
        updated_at: row.updated_at,
        env,
    }
}

#[derive(Deserialize)]
struct UpsertBody {
    agent_id: String,
    name: String,
    /// "http" | "tcp"
    kind: String,
    target: String,
    #[serde(default = "default_interval")]
    interval_secs: i64,
    #[serde(default = "default_timeout")]
    timeout_secs: i64,
    #[serde(default)]
    expect_status: Option<i64>,
    #[serde(default)]
    expect_body: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    /// Optional `KEY=VALUE` strings passed to exec probes.
    #[serde(default)]
    env: Vec<String>,
}

fn default_interval() -> i64 {
    30
}
fn default_timeout() -> i64 {
    5
}
fn default_true() -> bool {
    true
}

async fn require_auth(jar: &CookieJar, state: &AppState) -> Option<String> {
    auth::current_user(jar, &state.db).await.ok().map(|claims| claims.sub)
}

fn parse_kind(s: &str) -> Option<HealthProbeKind> {
    match s {
        "http" => Some(HealthProbeKind::Http),
        "tcp" => Some(HealthProbeKind::Tcp),
        "exec" => Some(HealthProbeKind::Exec),
        _ => None,
    }
}

fn row_to_spec(row: &db::HealthProbeRow) -> Option<HealthProbeSpec> {
    let env: Vec<String> = serde_json::from_str(&row.env_json).unwrap_or_default();
    Some(HealthProbeSpec {
        id: row.id.to_string(),
        name: row.name.clone(),
        kind: parse_kind(&row.kind)?,
        target: row.target.clone(),
        interval_secs: row.interval_secs.max(1) as u32,
        timeout_secs: row.timeout_secs.max(1) as u32,
        expect_status: row.expect_status.map(|n| n as u16),
        expect_body: row.expect_body.clone(),
        env,
    })
}

/// Push the agent's current enabled probe set to the agent (no-op if
/// the agent isn't connected — they'll be pushed on next register).
pub async fn push_to_agent(state: &AppState, agent_id: &str) {
    let rows = match db::list_health_probes_for(&state.db, agent_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, %agent_id, "health: failed to read probes for push");
            return;
        }
    };
    let probes: Vec<HealthProbeSpec> = rows
        .iter()
        .filter(|r| r.enabled != 0)
        .filter_map(row_to_spec)
        .collect();
    let agents = state.agents.lock().await;
    if let Some(entry) = agents.get(agent_id) {
        let _ = entry.tx.send(Message::HealthProbeSyncRequest { probes });
    }
}

async fn list_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::list_health_probes(&state.db).await {
        Ok(rows) => {
            let out: Vec<ProbeOut> = rows.into_iter().map(to_out).collect();
            Json(out).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list health_probes failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn upsert_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpsertBody>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    if parse_kind(&body.kind).is_none() {
        return (StatusCode::BAD_REQUEST, "kind must be http, tcp, or exec").into_response();
    }
    if body.name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name required").into_response();
    }
    if body.target.is_empty() {
        return (StatusCode::BAD_REQUEST, "target required").into_response();
    }
    let now = crate::now_unix();
    let env_json = serde_json::to_string(&body.env).unwrap_or_else(|_| "[]".to_string());
    let id = match db::upsert_health_probe(
        &state.db,
        &body.agent_id,
        &body.name,
        &body.kind,
        &body.target,
        body.interval_secs.max(1),
        body.timeout_secs.max(1),
        body.expect_status,
        body.expect_body.as_deref(),
        body.enabled,
        &env_json,
        now,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "upsert health_probe failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    db::record_audit(
        &state.db,
        now,
        Some(&actor),
        Some(&body.agent_id),
        "health_probe.upsert",
        true,
        Some(&format!(
            "name={} kind={} target={}",
            body.name, body.kind, body.target
        )),
    )
    .await;
    push_to_agent(&state, &body.agent_id).await;
    let row = match db::list_health_probes_for(&state.db, &body.agent_id).await {
        Ok(rows) => rows.into_iter().find(|r| r.id == id),
        Err(_) => None,
    };
    match row {
        Some(r) => Json(to_out(r)).into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response(),
    }
}

async fn delete_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let removed = match db::delete_health_probe(&state.db, id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "delete health_probe failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let Some((agent_id, name)) = removed else {
        return (StatusCode::NOT_FOUND, "No probe").into_response();
    };
    db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&actor),
        Some(&agent_id),
        "health_probe.delete",
        true,
        Some(&format!("name={name}")),
    )
    .await;
    push_to_agent(&state, &agent_id).await;
    (StatusCode::OK, "Deleted").into_response()
}

#[derive(Serialize)]
struct HostSnapshot {
    agent_id: String,
    total: u32,
    green: u32,
    red: u32,
    unknown: u32,
}

async fn snapshot_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let rows = match db::list_health_probes(&state.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "snapshot health_probes failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let mut by_agent: std::collections::BTreeMap<String, HostSnapshot> =
        std::collections::BTreeMap::new();
    for r in rows {
        if r.enabled == 0 {
            continue;
        }
        let s = by_agent.entry(r.agent_id.clone()).or_insert(HostSnapshot {
            agent_id: r.agent_id.clone(),
            total: 0,
            green: 0,
            red: 0,
            unknown: 0,
        });
        s.total += 1;
        match r.last_state.as_deref() {
            Some("green") => s.green += 1,
            Some("red") => s.red += 1,
            _ => s.unknown += 1,
        }
    }
    let out: Vec<HostSnapshot> = by_agent.into_values().collect();
    Json(out).into_response()
}
