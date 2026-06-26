//! In-app notifications inbox. Events emitted via `notify(...)` are
//! persisted to SQLite and surfaced through the dashboard. Independent
//! of the optional outbound `webhook.rs` forwarder.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::{AppState, auth::verify_token, db};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler))
        .route("/unread-count", get(unread_count_handler))
        .route("/mark-all-read", post(mark_all_read_handler))
        .route("/{id}/read", post(mark_read_handler))
        .route("/{id}", delete(delete_handler))
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

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    unread: Option<bool>,
}

async fn list_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    if require_auth(&jar).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    match db::list_notifications(&state.db, limit, q.unread.unwrap_or(false)).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list notifications failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

#[derive(Serialize)]
struct UnreadCount {
    unread: i64,
}

async fn unread_count_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if require_auth(&jar).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::unread_notification_count(&state.db).await {
        Ok(n) => Json(UnreadCount { unread: n }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "unread count failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn mark_read_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if require_auth(&jar).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::mark_notification_read(&state.db, id, crate::now_unix()).await {
        Ok(true) => (StatusCode::OK, "ok").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "not found or already read").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "mark read failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn mark_all_read_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if require_auth(&jar).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::mark_all_notifications_read(&state.db, crate::now_unix()).await {
        Ok(n) => Json(serde_json::json!({ "updated": n })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "mark all read failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn delete_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if require_auth(&jar).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::delete_notification(&state.db, id).await {
        Ok(true) => (StatusCode::OK, "deleted").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete notification failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

/// Emit an in-app notification. Levels: "info" | "warn" | "error".
/// Failures are logged but don't bubble up — the caller's primary work
/// (recording the event itself) shouldn't fail because the inbox is.
pub async fn notify(
    db: &SqlitePool,
    kind: &str,
    agent_id: Option<&str>,
    level: &str,
    title: &str,
    body: Option<&str>,
) {
    if let Err(e) =
        db::insert_notification(db, kind, agent_id, level, title, body, crate::now_unix()).await
    {
        tracing::warn!(error = %e, "failed to insert notification");
    }
}
