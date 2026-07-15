//! Per-agent backup jobs: list of paths + destination + optional cron.
//! Server schedules them, dispatches `BackupRunRequest` to the agent,
//! and attributes the resulting `BackupRunResponse` back to the job
//! so the UI can show last_status/last_archive_path/last_bytes/last_log.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use shared::{BackupMode, Message};
use std::str::FromStr;
use std::sync::Arc;

use crate::{AppState, auth, db};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler).post(upsert_handler))
        .route("/{id}", delete(delete_handler))
        .route("/{id}/run", post(run_now_handler))
        .route("/{id}/archives", post(list_archives_handler))
        .route("/{id}/restore", post(restore_handler))
}

#[derive(Serialize)]
struct JobOut {
    id: i64,
    agent_id: String,
    name: String,
    paths: Vec<String>,
    dest: String,
    cron_expr: Option<String>,
    enabled: bool,
    mode: String,
    last_run_at: i64,
    last_status: Option<String>,
    last_archive_path: Option<String>,
    last_bytes: Option<i64>,
    last_log: Option<String>,
    updated_at: i64,
    next_run_at: Option<i64>,
}

fn next_run(cron_expr: &Option<String>) -> Option<i64> {
    let expr = cron_expr.as_ref()?;
    let schedule = cron::Schedule::from_str(expr).ok()?;
    let now = chrono::Utc::now();
    schedule
        .upcoming(chrono::Utc)
        .next()
        .map(|t| t.timestamp())
        .or(Some(now.timestamp()))
}

fn to_out(row: db::BackupJobRow) -> JobOut {
    let paths: Vec<String> = serde_json::from_str(&row.paths_json).unwrap_or_default();
    let next = next_run(&row.cron_expr);
    JobOut {
        id: row.id,
        agent_id: row.agent_id,
        name: row.name,
        paths,
        dest: row.dest,
        cron_expr: row.cron_expr,
        enabled: row.enabled != 0,
        mode: row.mode,
        last_run_at: row.last_run_at,
        last_status: row.last_status,
        last_archive_path: row.last_archive_path,
        last_bytes: row.last_bytes,
        last_log: row.last_log,
        updated_at: row.updated_at,
        next_run_at: next,
    }
}

fn parse_mode(s: &str) -> BackupMode {
    match s {
        "restic" => BackupMode::Restic,
        _ => BackupMode::Tar,
    }
}

#[derive(Deserialize)]
struct UpsertBody {
    agent_id: String,
    name: String,
    paths: Vec<String>,
    /// Destination URI:
    ///   - bare path or `file:///...` — local filesystem on the agent
    ///   - `s3://bucket/prefix` — uploaded via the agent's `aws` CLI
    dest: String,
    /// Optional cron expression (7-field, UTC). When set, the scheduler
    /// fires the job. When absent, the job only runs via run-now.
    #[serde(default)]
    cron_expr: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    /// "tar" (default) or "restic". Restic mode is field-shaped for
    /// the future and currently rejected by the agent with a clear
    /// "not implemented" message.
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "tar".to_string()
}

fn default_true() -> bool {
    true
}

async fn require_auth(jar: &CookieJar, state: &AppState) -> Option<String> {
    auth::current_user(jar, &state.db).await.ok().map(|claims| claims.sub)
}

async fn list_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::list_backup_jobs(&state.db).await {
        Ok(rows) => {
            let out: Vec<JobOut> = rows.into_iter().map(to_out).collect();
            Json(out).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list backup_jobs failed");
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
    if body.name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name required").into_response();
    }
    if body.paths.is_empty() {
        return (StatusCode::BAD_REQUEST, "at least one path required").into_response();
    }
    if body.dest.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "dest required").into_response();
    }
    if let Some(cron) = body.cron_expr.as_deref().filter(|s| !s.is_empty()) {
        if cron::Schedule::from_str(cron).is_err() {
            return (StatusCode::BAD_REQUEST, "invalid cron expression").into_response();
        }
    }
    let paths_json = match serde_json::to_string(&body.paths) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("encode paths: {e}"),
            )
                .into_response();
        }
    };
    let now = crate::now_unix();
    let cron_for_db: Option<&str> = body.cron_expr.as_deref().filter(|s| !s.is_empty());
    let id = match db::upsert_backup_job(
        &state.db,
        &body.agent_id,
        &body.name,
        &paths_json,
        &body.dest,
        cron_for_db,
        body.enabled,
        if body.mode == "restic" {
            "restic"
        } else {
            "tar"
        },
        now,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "upsert backup_job failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    db::record_audit(
        &state.db,
        now,
        Some(&actor),
        Some(&body.agent_id),
        "backup_job.upsert",
        true,
        Some(&format!(
            "name={} paths={} dest={} cron={:?}",
            body.name,
            body.paths.len(),
            body.dest,
            cron_for_db.unwrap_or("")
        )),
    )
    .await;
    match db::get_backup_job(&state.db, id).await {
        Ok(Some(r)) => Json(to_out(r)).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response(),
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
    let prev = db::get_backup_job(&state.db, id).await.ok().flatten();
    match db::delete_backup_job(&state.db, id).await {
        Ok(true) => {
            if let Some(row) = prev {
                db::record_audit(
                    &state.db,
                    crate::now_unix(),
                    Some(&actor),
                    Some(&row.agent_id),
                    "backup_job.delete",
                    true,
                    Some(&format!("name={}", row.name)),
                )
                .await;
            }
            (StatusCode::OK, "deleted").into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "no such job").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete backup_job failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn run_now_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let row = match db::get_backup_job(&state.db, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such job").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get backup_job failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let paths: Vec<String> = serde_json::from_str(&row.paths_json).unwrap_or_default();
    if paths.is_empty() {
        return (StatusCode::BAD_REQUEST, "paths is empty").into_response();
    }
    let agents = state.agents.lock().await;
    let Some(tx) = agents.get(&row.agent_id).map(|e| e.tx.clone()) else {
        return (StatusCode::NOT_FOUND, "agent offline").into_response();
    };
    drop(agents);
    let req = Message::BackupRunRequest {
        id: row.id.to_string(),
        name: row.name.clone(),
        paths,
        dest: row.dest.clone(),
        mode: parse_mode(&row.mode),
    };
    if tx.send(req).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "send failed").into_response();
    }
    let now = crate::now_unix();
    let _ = db::record_backup_job_result(&state.db, id, now, "running", "", 0, "").await;
    db::record_audit(
        &state.db,
        now,
        Some(&actor),
        Some(&row.agent_id),
        "backup_job.run_now",
        true,
        Some(&row.name),
    )
    .await;
    (StatusCode::OK, "Triggered").into_response()
}

async fn list_archives_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let row = match db::get_backup_job(&state.db, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such job").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get backup_job failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let agents = state.agents.lock().await;
    let Some(tx) = agents.get(&row.agent_id).map(|e| e.tx.clone()) else {
        return (StatusCode::NOT_FOUND, "agent offline").into_response();
    };
    drop(agents);

    let (otx, orx) = tokio::sync::oneshot::channel();
    {
        const MAX_PENDING: usize = 32;
        let mut waiters = state.pending_backup_lists.lock().await;
        let q = waiters.entry(row.agent_id.clone()).or_default();
        if q.len() >= MAX_PENDING {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "too many in-flight backup-list requests for this agent",
            )
                .into_response();
        }
        q.push_back(otx);
    }
    let req = Message::BackupListArchivesRequest {
        id: row.id.to_string(),
        name: row.name.clone(),
        dest: row.dest.clone(),
    };
    if tx.send(req).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "send failed").into_response();
    }
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(20), orx).await {
        Ok(Ok(m)) => m,
        Ok(Err(_)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "agent dropped").into_response();
        }
        Err(_) => {
            return (StatusCode::REQUEST_TIMEOUT, "timeout waiting for agent").into_response();
        }
    };
    if let Message::BackupListArchivesResponse {
        success,
        archives,
        error,
        ..
    } = resp
    {
        if !success {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                error.unwrap_or_else(|| "unknown".into()),
            )
                .into_response();
        }
        Json(archives).into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "unexpected message").into_response()
    }
}

#[derive(Deserialize)]
struct RestoreBody {
    archive_uri: String,
    dest_root: String,
}

#[derive(Serialize)]
struct RestoreOut {
    success: bool,
    log: String,
    error: Option<String>,
}

async fn restore_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<RestoreBody>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    if body.archive_uri.is_empty() || body.dest_root.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "archive_uri and dest_root required",
        )
            .into_response();
    }
    let row = match db::get_backup_job(&state.db, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such job").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get backup_job failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let agents = state.agents.lock().await;
    let Some(tx) = agents.get(&row.agent_id).map(|e| e.tx.clone()) else {
        return (StatusCode::NOT_FOUND, "agent offline").into_response();
    };
    drop(agents);

    let (otx, orx) = tokio::sync::oneshot::channel();
    {
        const MAX_PENDING: usize = 32;
        let mut waiters = state.pending_backup_restores.lock().await;
        let q = waiters.entry(row.agent_id.clone()).or_default();
        if q.len() >= MAX_PENDING {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "too many in-flight backup-restore requests for this agent",
            )
                .into_response();
        }
        q.push_back(otx);
    }
    let req = Message::BackupRestoreRequest {
        id: row.id.to_string(),
        archive_uri: body.archive_uri.clone(),
        dest_root: body.dest_root.clone(),
    };
    if tx.send(req).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "send failed").into_response();
    }
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(120), orx).await {
        Ok(Ok(m)) => m,
        Ok(Err(_)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "agent dropped").into_response();
        }
        Err(_) => {
            return (StatusCode::REQUEST_TIMEOUT, "timeout waiting for agent").into_response();
        }
    };
    if let Message::BackupRestoreResponse {
        success,
        log,
        error,
        ..
    } = resp
    {
        db::record_audit(
            &state.db,
            crate::now_unix(),
            Some(&actor),
            Some(&row.agent_id),
            "backup_job.restore",
            success,
            Some(&format!(
                "name={} archive={} dest={}",
                row.name, body.archive_uri, body.dest_root
            )),
        )
        .await;
        Json(RestoreOut {
            success,
            log,
            error,
        })
        .into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "unexpected message").into_response()
    }
}

/// Background loop: every 60s, scan enabled jobs with a cron set; if
/// the next tick has passed since `last_run_at`, dispatch the request.
pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = scheduler_tick(&state).await {
                tracing::warn!(error = %e, "backup scheduler tick failed");
            }
        }
    });
}

async fn scheduler_tick(state: &AppState) -> Result<(), sqlx::Error> {
    let rows = db::list_backup_jobs(&state.db).await?;
    let now = chrono::Utc::now();
    for row in rows {
        if row.enabled == 0 {
            continue;
        }
        let Some(cron_expr) = row.cron_expr.as_deref() else {
            continue;
        };
        let Ok(schedule) = cron::Schedule::from_str(cron_expr) else {
            continue;
        };
        let last = if row.last_run_at > 0 {
            chrono::DateTime::<chrono::Utc>::from_timestamp(row.last_run_at, 0)
                .unwrap_or_else(|| now - chrono::Duration::days(365))
        } else {
            chrono::DateTime::<chrono::Utc>::from_timestamp(row.updated_at, 0).unwrap_or(now)
        };
        let due = schedule
            .after(&last)
            .next()
            .map(|t| t <= now)
            .unwrap_or(false);
        if !due {
            continue;
        }
        let agents = state.agents.lock().await;
        let Some(tx) = agents.get(&row.agent_id).map(|e| e.tx.clone()) else {
            tracing::info!(agent_id = %row.agent_id, name = %row.name, "backup job due but agent offline");
            continue;
        };
        drop(agents);
        let now_unix = crate::now_unix();
        let _ =
            db::record_backup_job_result(&state.db, row.id, now_unix, "running", "", 0, "").await;
        let paths: Vec<String> = serde_json::from_str(&row.paths_json).unwrap_or_default();
        let req = Message::BackupRunRequest {
            id: row.id.to_string(),
            name: row.name.clone(),
            paths,
            dest: row.dest.clone(),
            mode: parse_mode(&row.mode),
        };
        if tx.send(req).is_err() {
            tracing::warn!(agent_id = %row.agent_id, "failed to send scheduled BackupRunRequest");
            continue;
        }
        db::record_audit(
            &state.db,
            now_unix,
            Some("scheduler"),
            Some(&row.agent_id),
            "backup_job.fire",
            true,
            Some(&format!("name={} cron={cron_expr}", row.name)),
        )
        .await;
    }
    Ok(())
}
