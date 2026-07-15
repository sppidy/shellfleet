//! Auto-update windows: per-agent cron schedules that fire
//! `AptUpgradeRequest` automatically. Results land in `update_windows`
//! plus `audit`.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use shared::Message;
use std::{str::FromStr, sync::Arc};

use crate::{AppState, auth, db};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler).post(upsert_handler))
        .route("/{agent_id}", delete(delete_handler))
        .route("/{agent_id}/run", post(run_now_handler))
}

#[derive(Serialize)]
struct WindowOut {
    agent_id: String,
    cron_expr: String,
    enabled: bool,
    last_run_at: i64,
    last_status: Option<String>,
    last_log: Option<String>,
    updated_at: i64,
    next_run_at: Option<i64>,
}

#[derive(Deserialize)]
struct UpsertBody {
    agent_id: String,
    cron_expr: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

async fn require_auth(jar: &CookieJar, state: &AppState) -> Option<String> {
    auth::current_user(jar, &state.db).await.ok().map(|claims| claims.sub)
}

fn next_run(cron_expr: &str) -> Option<i64> {
    let schedule = cron::Schedule::from_str(cron_expr).ok()?;
    let now = chrono::Utc::now();
    schedule
        .upcoming(chrono::Utc)
        .next()
        .map(|t| t.timestamp())
        .or(Some(now.timestamp()))
}

fn validate_cron(expr: &str) -> Result<(), String> {
    cron::Schedule::from_str(expr)
        .map(|_| ())
        .map_err(|e| format!("invalid cron: {e}"))
}

fn to_out(row: db::UpdateWindowRow) -> WindowOut {
    let next_run_at = next_run(&row.cron_expr);
    WindowOut {
        agent_id: row.agent_id,
        cron_expr: row.cron_expr,
        enabled: row.enabled != 0,
        last_run_at: row.last_run_at,
        last_status: row.last_status,
        last_log: row.last_log,
        updated_at: row.updated_at,
        next_run_at,
    }
}

async fn list_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    match db::list_update_windows(&state.db).await {
        Ok(rows) => {
            let out: Vec<WindowOut> = rows.into_iter().map(to_out).collect();
            Json(out).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list update_windows failed");
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
    if let Err(e) = validate_cron(&body.cron_expr) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    let now = crate::now_unix();
    if let Err(e) = db::upsert_update_window(
        &state.db,
        &body.agent_id,
        &body.cron_expr,
        body.enabled,
        now,
    )
    .await
    {
        tracing::error!(error = %e, "upsert update_window failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }
    db::record_audit(
        &state.db,
        now,
        Some(&actor),
        Some(&body.agent_id),
        "update_window.upsert",
        true,
        Some(&format!("cron={} enabled={}", body.cron_expr, body.enabled)),
    )
    .await;
    match db::get_update_window(&state.db, &body.agent_id).await {
        Ok(Some(row)) => Json(to_out(row)).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response(),
    }
}

async fn delete_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    match db::delete_update_window(&state.db, &agent_id).await {
        Ok(true) => {
            db::record_audit(
                &state.db,
                crate::now_unix(),
                Some(&actor),
                Some(&agent_id),
                "update_window.delete",
                true,
                None,
            )
            .await;
            (StatusCode::OK, "Deleted").into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "No window").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete update_window failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

#[derive(Deserialize)]
struct RunNowQuery {
    /// Optional single package to upgrade (`apt-get install --only-upgrade`).
    /// When omitted the agent runs `apt-get -y upgrade` (full window).
    #[serde(default)]
    package: Option<String>,
}

async fn run_now_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(q): Query<RunNowQuery>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let agents = state.agents.lock().await;
    let Some(tx) = agents.get(&agent_id).map(|e| e.tx.clone()) else {
        return (StatusCode::NOT_FOUND, "Agent offline").into_response();
    };
    drop(agents);
    state
        .scheduled_apt_runs
        .lock()
        .await
        .insert(agent_id.clone());
    let pkg = q.package.clone();
    if tx
        .send(Message::AptUpgradeRequest {
            package: pkg.clone(),
        })
        .is_err()
    {
        state.scheduled_apt_runs.lock().await.remove(&agent_id);
        return (StatusCode::INTERNAL_SERVER_ERROR, "send failed").into_response();
    }
    db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&actor),
        Some(&agent_id),
        "update_window.run_now",
        true,
        pkg.as_deref(),
    )
    .await;
    (StatusCode::OK, "Triggered").into_response()
}

/// Background loop: every 60s, check each enabled window. If `next` from
/// its cron has elapsed since `last_run_at`, fire `AptUpgradeRequest` to
/// the connected agent.
pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        // Skip the immediate first tick — gives the server a moment to
        // accept agent connections before we try to fire.
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = scheduler_tick(&state).await {
                tracing::warn!(error = %e, "update_windows scheduler tick failed");
            }
        }
    });
}

async fn scheduler_tick(state: &AppState) -> Result<(), sqlx::Error> {
    let rows = db::list_update_windows(&state.db).await?;
    let now = chrono::Utc::now();
    for row in rows {
        if row.enabled == 0 {
            continue;
        }
        let Ok(schedule) = cron::Schedule::from_str(&row.cron_expr) else {
            continue;
        };
        let last = if row.last_run_at > 0 {
            chrono::DateTime::<chrono::Utc>::from_timestamp(row.last_run_at, 0)
                .unwrap_or_else(|| now - chrono::Duration::days(365))
        } else {
            // First-ever schedule: only fire on the next strictly-future tick,
            // not retroactively. Use updated_at as the floor.
            chrono::DateTime::<chrono::Utc>::from_timestamp(row.updated_at, 0).unwrap_or(now)
        };
        // Has any cron tick occurred between `last` and `now`?
        let due = schedule
            .after(&last)
            .next()
            .map(|t| t <= now)
            .unwrap_or(false);
        if !due {
            continue;
        }
        let agent_id = row.agent_id.clone();
        let agents = state.agents.lock().await;
        let Some(tx) = agents.get(&agent_id).map(|e| e.tx.clone()) else {
            tracing::info!(agent_id = %agent_id, "update_window due but agent offline; deferring");
            continue;
        };
        drop(agents);
        // Bookkeep before sending — guarantees we record `last_run_at`
        // even if the agent never replies.
        let now_unix = crate::now_unix();
        let _ =
            db::record_update_window_result(&state.db, &agent_id, now_unix, "running", "").await;
        state
            .scheduled_apt_runs
            .lock()
            .await
            .insert(agent_id.clone());
        if tx
            .send(Message::AptUpgradeRequest { package: None })
            .is_err()
        {
            state.scheduled_apt_runs.lock().await.remove(&agent_id);
            tracing::warn!(agent_id = %agent_id, "failed to send scheduled AptUpgradeRequest");
            continue;
        }
        db::record_audit(
            &state.db,
            now_unix,
            Some("scheduler"),
            Some(&agent_id),
            "update_window.fire",
            true,
            Some(&format!("cron={}", row.cron_expr)),
        )
        .await;
    }
    Ok(())
}
