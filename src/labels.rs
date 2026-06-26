//! Per-agent labels. Used by fan-out to target a group of agents
//! ("everything tagged `worker`") without listing each id.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{AppState, auth::verify_token, db};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler).post(add_handler))
        .route("/{agent_id}/{label}", delete(delete_handler))
}

fn require_auth(jar: &CookieJar) -> Option<String> {
    if std::env::var("JWT_SECRET").unwrap_or_default() == "dev" {
        return Some("dev".into());
    }
    let cookie = jar.get("auth_token")?;
    if verify_token(cookie.value()) {
        crate::auth::user_from_token(cookie.value())
    } else {
        None
    }
}

#[derive(Serialize)]
struct ListOut {
    /// agent_id → list of labels
    by_agent: BTreeMap<String, Vec<String>>,
    /// label → list of agent_ids
    by_label: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct ListQuery {
    /// If set, return only labels for that agent (still under the
    /// same `by_agent`/`by_label` shape so the SPA doesn't branch).
    #[serde(default)]
    agent_id: Option<String>,
}

async fn list_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    if require_auth(&jar).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let rows = match db::list_all_labels(&state.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "list labels failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let mut by_agent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_label: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (agent_id, label) in rows {
        if q.agent_id
            .as_deref()
            .is_some_and(|filter| filter != agent_id)
        {
            continue;
        }
        by_agent
            .entry(agent_id.clone())
            .or_default()
            .push(label.clone());
        by_label.entry(label).or_default().push(agent_id);
    }
    Json(ListOut { by_agent, by_label }).into_response()
}

#[derive(Deserialize)]
struct AddBody {
    agent_id: String,
    label: String,
}

async fn add_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<AddBody>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar) else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let label = body.label.trim();
    if label.is_empty() || label.contains(',') || label.contains(' ') {
        return (
            StatusCode::BAD_REQUEST,
            "label must be non-empty and contain no commas or spaces",
        )
            .into_response();
    }
    if let Err(e) = db::add_label(&state.db, &body.agent_id, label).await {
        tracing::error!(error = %e, "add label failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }
    db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&actor),
        Some(&body.agent_id),
        "agent_label.add",
        true,
        Some(label),
    )
    .await;
    (StatusCode::OK, "ok").into_response()
}

async fn delete_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path((agent_id, label)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar) else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    match db::remove_label(&state.db, &agent_id, &label).await {
        Ok(true) => {
            db::record_audit(
                &state.db,
                crate::now_unix(),
                Some(&actor),
                Some(&agent_id),
                "agent_label.delete",
                true,
                Some(&label),
            )
            .await;
            (StatusCode::OK, "deleted").into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "no such label").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "remove label failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}
