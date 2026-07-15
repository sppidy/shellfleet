//! Inter-agent fan-out commands. Operator targets a list of agent_ids
//! and a fan-out kind; server broadcasts the corresponding Message and
//! attributes responses back to the run.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use shared::Message;
use std::sync::Arc;

use crate::{AppState, auth, db};

/// Fan-out kinds supported in v1.
pub const KIND_APT_STATUS: &str = "apt-status";
pub const KIND_APT_UPGRADE: &str = "apt-upgrade";
pub const KIND_DOCKER_LIST: &str = "docker-list";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler).post(create_handler))
        .route("/{id}", get(get_handler))
}

async fn require_auth(jar: &CookieJar, state: &AppState) -> Option<String> {
    auth::current_user(jar, &state.db).await.ok().map(|claims| claims.sub)
}

#[derive(Deserialize)]
struct CreateBody {
    /// One of `apt-status`, `apt-upgrade`, `docker-list`.
    kind: String,
    /// Agent ids to target. Either this OR `label` must be provided.
    /// Agents that are offline get recorded immediately with
    /// status="offline".
    #[serde(default)]
    agent_ids: Vec<String>,
    /// Resolve targets by label instead of by id. When set, the
    /// server expands to all agents tagged with this label.
    #[serde(default)]
    label: Option<String>,
    /// Optional kind-specific payload. For `apt-upgrade`, this is the
    /// package name (None = full upgrade).
    #[serde(default)]
    package: Option<String>,
}

#[derive(Serialize)]
struct RunOut {
    run: db::FanOutRunRow,
    results: Vec<db::FanOutResultRow>,
}

fn message_for(kind: &str, package: Option<String>) -> Option<Message> {
    match kind {
        KIND_APT_STATUS => Some(Message::AptStatusRequest),
        KIND_APT_UPGRADE => Some(Message::AptUpgradeRequest { package }),
        KIND_DOCKER_LIST => Some(Message::DockerListRequest),
        _ => None,
    }
}

async fn list_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::list_fan_out_runs(&state.db, 50).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list fan_out_runs failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn get_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let run = match db::get_fan_out_run(&state.db, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "no run").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get fan_out_run failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let results = db::get_fan_out_results(&state.db, id)
        .await
        .unwrap_or_default();
    Json(RunOut { run, results }).into_response()
}

async fn create_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateBody>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let Some(message_template) = message_for(&body.kind, body.package.clone()) else {
        return (StatusCode::BAD_REQUEST, "unsupported kind").into_response();
    };
    // Resolve targets — either explicit ids or by label.
    let agent_ids: Vec<String> = if let Some(label) = body.label.as_deref() {
        match db::agents_for_label(&state.db, label).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(error = %e, "resolve label failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    } else {
        body.agent_ids.clone()
    };
    if agent_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "no targets — provide agent_ids or a label that resolves to at least one agent",
        )
            .into_response();
    }
    let now = crate::now_unix();
    let payload_json = body
        .package
        .as_ref()
        .map(|p| format!("{{\"package\":\"{p}\"}}"));
    let run_id = match db::create_fan_out_run(
        &state.db,
        &body.kind,
        payload_json.as_deref(),
        now,
        Some(&actor),
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "create fan_out_run failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    db::record_audit(
        &state.db,
        now,
        Some(&actor),
        None,
        "fan_out.create",
        true,
        Some(&format!(
            "kind={} agents={} label={:?}",
            body.kind,
            agent_ids.len(),
            body.label.as_deref().unwrap_or("")
        )),
    )
    .await;

    let agents = state.agents.lock().await;
    for agent_id in &agent_ids {
        if let Some(entry) = agents.get(agent_id) {
            // Mark pending up-front so response routing can find it.
            let _ =
                db::upsert_fan_out_result(&state.db, run_id, agent_id, "pending", None, None).await;
            if entry.tx.send(message_template.clone()).is_err() {
                let _ = db::upsert_fan_out_result(
                    &state.db,
                    run_id,
                    agent_id,
                    "failed",
                    Some("send failed"),
                    Some(now),
                )
                .await;
            }
        } else {
            let _ = db::upsert_fan_out_result(
                &state.db,
                run_id,
                agent_id,
                "offline",
                Some("agent not connected"),
                Some(now),
            )
            .await;
        }
    }
    drop(agents);

    let results = db::get_fan_out_results(&state.db, run_id)
        .await
        .unwrap_or_default();
    let run = db::get_fan_out_run(&state.db, run_id).await.ok().flatten();
    match run {
        Some(r) => Json(RunOut { run: r, results }).into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response(),
    }
}

/// Called from `handle_agent_socket` whenever a fan-out-eligible
/// response message arrives. Looks for the oldest pending row for
/// (agent_id, kind) and stamps it with the result.
pub async fn maybe_attribute_response(state: &AppState, agent_id: &str, msg: &Message) {
    let (kind, status, detail) = match msg {
        Message::AptStatusResponse {
            available,
            upgradable,
            error,
            ..
        } => {
            let det = if let Some(e) = error {
                format!("error: {e}")
            } else if *available {
                format!("{} upgradable", upgradable.len())
            } else {
                "apt unavailable".to_string()
            };
            let s = if error.is_some() || !*available {
                "failed"
            } else {
                "success"
            };
            (KIND_APT_STATUS, s, det)
        }
        Message::AptUpgradeResponse {
            success,
            log,
            error,
            ..
        } => {
            let det = if *success {
                let snippet: String = log.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                if snippet.is_empty() {
                    "ok".to_string()
                } else {
                    snippet
                }
            } else {
                error.clone().unwrap_or_else(|| "failed".to_string())
            };
            let s = if *success { "success" } else { "failed" };
            (KIND_APT_UPGRADE, s, det)
        }
        Message::DockerListResponse {
            available,
            swarm_role,
            containers,
            error,
            ..
        } => {
            let det = if let Some(e) = error {
                format!("error: {e}")
            } else if *available {
                let running = containers.iter().filter(|c| c.state == "running").count();
                format!("{running}/{} running ({swarm_role:?})", containers.len())
            } else {
                "docker unavailable".to_string()
            };
            let s = if error.is_some() || !*available {
                "failed"
            } else {
                "success"
            };
            (KIND_DOCKER_LIST, s, det)
        }
        _ => return,
    };
    let pending = match db::pending_fan_out_for_agent(&state.db, agent_id, kind).await {
        Ok(Some(id)) => id,
        _ => return,
    };
    let _ = db::upsert_fan_out_result(
        &state.db,
        pending,
        agent_id,
        status,
        Some(&detail),
        Some(crate::now_unix()),
    )
    .await;
}
