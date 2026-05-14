//! SQLite-backed persistence for tokens, pending device-auth requests, and
//! an audit ring. Replaces the prior `approved_tokens.json` + in-memory
//! HashMaps for `pending_devices` and `user_codes`.
//!
//! On first boot we transparently migrate the existing
//! `approved_tokens.json` file into the `tokens` table and rename the
//! file to `.migrated` so it isn't re-imported on subsequent boots.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub token: String,
    pub hostname: Option<String>,
    pub created_at: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingDeviceRow {
    pub device_code: String,
    pub user_code: String,
    pub expires_at: i64,
    pub approved: i64,
}

pub fn db_path_str() -> String {
    std::env::var("DB_PATH").unwrap_or_else(|_| "/data/shellfleet.db".to_string())
}

/// Initialize a SQLite pool, run schema migrations, and import the legacy
/// approved_tokens.json file if it exists.
pub async fn init() -> Result<SqlitePool, sqlx::Error> {
    let path = db_path_str();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tokens (
            token TEXT PRIMARY KEY,
            hostname TEXT,
            created_at INTEGER NOT NULL DEFAULT 0,
            last_seen INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pending_devices (
            device_code TEXT PRIMARY KEY,
            user_code TEXT NOT NULL UNIQUE,
            expires_at INTEGER NOT NULL,
            approved INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            actor TEXT,
            agent_id TEXT,
            kind TEXT NOT NULL,
            ok INTEGER NOT NULL,
            detail TEXT
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS audit_ts ON audit(ts DESC);")
        .execute(&pool)
        .await?;

    // CE: per-user state for RBAC + 2FA. Created lazily on first OAuth
    // sign-in (see auth::callback_handler). Roles are restricted to the
    // CE-supported pair (admin / viewer); EE will introduce richer roles
    // through the extension API rather than by widening this column.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            login TEXT PRIMARY KEY,
            role TEXT NOT NULL DEFAULT 'viewer'
                CHECK (role IN ('admin', 'viewer')),
            totp_enabled INTEGER NOT NULL DEFAULT 0,
            totp_secret TEXT,
            totp_recovery_hashes TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL DEFAULT 0,
            last_login_at INTEGER NOT NULL DEFAULT 0,
            session_epoch INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;
    // Idempotent column add for installs created before session_epoch
    // landed. `ALTER TABLE ADD COLUMN` errors if the column already
    // exists; ignore that case.
    let _ = sqlx::query(
        "ALTER TABLE users ADD COLUMN session_epoch INTEGER NOT NULL DEFAULT 0",
    )
    .execute(&pool)
    .await;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS update_windows (
            agent_id TEXT PRIMARY KEY,
            cron_expr TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run_at INTEGER NOT NULL DEFAULT 0,
            last_status TEXT,
            last_log TEXT,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS health_probes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            target TEXT NOT NULL,
            interval_secs INTEGER NOT NULL DEFAULT 30,
            timeout_secs INTEGER NOT NULL DEFAULT 5,
            expect_status INTEGER,
            expect_body TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run_at INTEGER NOT NULL DEFAULT 0,
            last_state TEXT,
            last_latency_ms INTEGER,
            last_detail TEXT,
            updated_at INTEGER NOT NULL DEFAULT 0,
            env_json TEXT NOT NULL DEFAULT '[]',
            UNIQUE(agent_id, name)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    let _ = sqlx::query("ALTER TABLE health_probes ADD COLUMN env_json TEXT NOT NULL DEFAULT '[]'")
        .execute(&pool)
        .await;

    sqlx::query("CREATE INDEX IF NOT EXISTS health_probes_agent ON health_probes(agent_id);")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS fan_out_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            payload TEXT,
            started_at INTEGER NOT NULL,
            actor TEXT
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS fan_out_results (
            run_id INTEGER NOT NULL,
            agent_id TEXT NOT NULL,
            status TEXT NOT NULL,
            detail TEXT,
            finished_at INTEGER,
            PRIMARY KEY (run_id, agent_id)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS fan_out_runs_started ON fan_out_runs(started_at DESC);")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            agent_id TEXT,
            level TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT,
            created_at INTEGER NOT NULL,
            read_at INTEGER
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS notifications_created ON notifications(created_at DESC);")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS notifications_unread ON notifications(read_at, created_at DESC);")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS agent_labels (
            agent_id TEXT NOT NULL,
            label TEXT NOT NULL,
            PRIMARY KEY (agent_id, label)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS agent_labels_label ON agent_labels(label);")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS backup_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            name TEXT NOT NULL,
            paths_json TEXT NOT NULL,
            dest TEXT NOT NULL,
            cron_expr TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run_at INTEGER NOT NULL DEFAULT 0,
            last_status TEXT,
            last_archive_path TEXT,
            last_bytes INTEGER,
            last_log TEXT,
            updated_at INTEGER NOT NULL DEFAULT 0,
            mode TEXT NOT NULL DEFAULT 'tar',
            UNIQUE(agent_id, name)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS backup_jobs_agent ON backup_jobs(agent_id);")
        .execute(&pool)
        .await?;
    // Idempotent column add for installs that already had the table
    // before mode landed. SQLite errors if the column exists; ignore.
    let _ = sqlx::query("ALTER TABLE backup_jobs ADD COLUMN mode TEXT NOT NULL DEFAULT 'tar'")
        .execute(&pool)
        .await;

    migrate_legacy_tokens(&pool).await?;

    Ok(pool)
}

async fn migrate_legacy_tokens(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Look in the same directory the volume mounts at, plus the legacy
    // CWD path used by the very first version of the server.
    let candidates = [
        std::env::var("TOKENS_PATH").unwrap_or_else(|_| "/data/approved_tokens.json".to_string()),
        "approved_tokens.json".to_string(),
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read_to_string(&path) {
            #[derive(serde::Deserialize)]
            struct LegacyInfo {
                #[serde(default)]
                created_at: i64,
                #[serde(default)]
                hostname: Option<String>,
                #[serde(default)]
                last_seen: i64,
            }
            // Try the metadata schema first; fall back to the bool schema
            // that the very first version shipped with.
            let map: HashMap<String, LegacyInfo> = if let Ok(m) = serde_json::from_str(&data) {
                m
            } else if let Ok(m) = serde_json::from_str::<HashMap<String, bool>>(&data) {
                m.into_keys()
                    .map(|k| {
                        (
                            k,
                            LegacyInfo {
                                created_at: 0,
                                hostname: None,
                                last_seen: 0,
                            },
                        )
                    })
                    .collect()
            } else {
                continue;
            };

            tracing::info!(
                count = map.len(),
                path = %path,
                "migrating legacy tokens.json into sqlite"
            );

            for (token, info) in map {
                let _ = sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO tokens (token, hostname, created_at, last_seen)
                    VALUES (?1, ?2, ?3, ?4)
                    "#,
                )
                .bind(&token)
                .bind(info.hostname.as_deref())
                .bind(info.created_at)
                .bind(info.last_seen)
                .execute(pool)
                .await;
            }

            // Rename so we don't re-import on every restart.
            let migrated = format!("{path}.migrated");
            if let Err(e) = std::fs::rename(&path, &migrated) {
                tracing::warn!(error = %e, "could not rename legacy file; leaving in place");
            }
            return Ok(());
        }
    }
    Ok(())
}

pub async fn token_exists(pool: &SqlitePool, token: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT 1 FROM tokens WHERE token = ?")
        .bind(token)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

pub async fn upsert_token_seen(
    pool: &SqlitePool,
    token: &str,
    hostname: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO tokens (token, hostname, created_at, last_seen)
        VALUES (?1, ?2, ?3, ?3)
        ON CONFLICT(token) DO UPDATE SET
            hostname = excluded.hostname,
            last_seen = excluded.last_seen,
            created_at = CASE
                WHEN tokens.created_at = 0 THEN excluded.last_seen
                ELSE tokens.created_at
            END
        "#,
    )
    .bind(token)
    .bind(hostname)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_token(
    pool: &SqlitePool,
    token: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO tokens (token, hostname, created_at, last_seen)
        VALUES (?1, NULL, ?2, 0)
        "#,
    )
    .bind(token)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_tokens(pool: &SqlitePool) -> Result<Vec<TokenRow>, sqlx::Error> {
    sqlx::query_as::<_, TokenRow>(
        "SELECT token, hostname, created_at, last_seen FROM tokens ORDER BY last_seen DESC, created_at DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn revoke_token(pool: &SqlitePool, token: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM tokens WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn revoke_by_hostname(pool: &SqlitePool, hostname: &str) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("DELETE FROM tokens WHERE hostname = ?")
        .bind(hostname)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn insert_pending_device(
    pool: &SqlitePool,
    device_code: &str,
    user_code: &str,
    expires_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO pending_devices (device_code, user_code, expires_at, approved)
        VALUES (?1, ?2, ?3, 0)
        "#,
    )
    .bind(device_code)
    .bind(user_code)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn pending_device(
    pool: &SqlitePool,
    device_code: &str,
) -> Result<Option<PendingDeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, PendingDeviceRow>(
        "SELECT device_code, user_code, expires_at, approved FROM pending_devices WHERE device_code = ?",
    )
    .bind(device_code)
    .fetch_optional(pool)
    .await
}

pub async fn approve_user_code(
    pool: &SqlitePool,
    user_code: &str,
    now: i64,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE pending_devices SET approved = 1 WHERE user_code = ? AND expires_at >= ?",
    )
    .bind(user_code)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn delete_pending_device(
    pool: &SqlitePool,
    device_code: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM pending_devices WHERE device_code = ?")
        .bind(device_code)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn purge_expired_devices(pool: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("DELETE FROM pending_devices WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub ts: i64,
    pub actor: Option<String>,
    pub agent_id: Option<String>,
    pub kind: String,
    pub ok: i64,
    pub detail: Option<String>,
}

pub async fn record_audit(
    pool: &SqlitePool,
    ts: i64,
    actor: Option<&str>,
    agent_id: Option<&str>,
    kind: &str,
    ok: bool,
    detail: Option<&str>,
) {
    let _ = sqlx::query(
        r#"
        INSERT INTO audit (ts, actor, agent_id, kind, ok, detail)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(ts)
    .bind(actor)
    .bind(agent_id)
    .bind(kind)
    .bind(if ok { 1 } else { 0 })
    .bind(detail)
    .execute(pool)
    .await;
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UpdateWindowRow {
    pub agent_id: String,
    pub cron_expr: String,
    pub enabled: i64,
    pub last_run_at: i64,
    pub last_status: Option<String>,
    pub last_log: Option<String>,
    pub updated_at: i64,
}

pub async fn list_update_windows(
    pool: &SqlitePool,
) -> Result<Vec<UpdateWindowRow>, sqlx::Error> {
    sqlx::query_as::<_, UpdateWindowRow>(
        "SELECT agent_id, cron_expr, enabled, last_run_at, last_status, last_log, updated_at \
         FROM update_windows ORDER BY agent_id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_update_window(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<Option<UpdateWindowRow>, sqlx::Error> {
    sqlx::query_as::<_, UpdateWindowRow>(
        "SELECT agent_id, cron_expr, enabled, last_run_at, last_status, last_log, updated_at \
         FROM update_windows WHERE agent_id = ?",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_update_window(
    pool: &SqlitePool,
    agent_id: &str,
    cron_expr: &str,
    enabled: bool,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO update_windows (agent_id, cron_expr, enabled, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(agent_id) DO UPDATE SET
            cron_expr = excluded.cron_expr,
            enabled = excluded.enabled,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(agent_id)
    .bind(cron_expr)
    .bind(if enabled { 1 } else { 0 })
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_update_window(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM update_windows WHERE agent_id = ?")
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn record_update_window_result(
    pool: &SqlitePool,
    agent_id: &str,
    last_run_at: i64,
    last_status: &str,
    last_log: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE update_windows
           SET last_run_at = ?2,
               last_status = ?3,
               last_log = ?4
         WHERE agent_id = ?1
        "#,
    )
    .bind(agent_id)
    .bind(last_run_at)
    .bind(last_status)
    .bind(last_log)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct HealthProbeRow {
    pub id: i64,
    pub agent_id: String,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub interval_secs: i64,
    pub timeout_secs: i64,
    pub expect_status: Option<i64>,
    pub expect_body: Option<String>,
    pub enabled: i64,
    pub last_run_at: i64,
    pub last_state: Option<String>,
    pub last_latency_ms: Option<i64>,
    pub last_detail: Option<String>,
    pub updated_at: i64,
    pub env_json: String,
}

pub async fn list_health_probes(
    pool: &SqlitePool,
) -> Result<Vec<HealthProbeRow>, sqlx::Error> {
    sqlx::query_as::<_, HealthProbeRow>(
        "SELECT id, agent_id, name, kind, target, interval_secs, timeout_secs, \
         expect_status, expect_body, enabled, last_run_at, last_state, \
         last_latency_ms, last_detail, updated_at, env_json \
         FROM health_probes ORDER BY agent_id, name",
    )
    .fetch_all(pool)
    .await
}

pub async fn list_health_probes_for(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<Vec<HealthProbeRow>, sqlx::Error> {
    sqlx::query_as::<_, HealthProbeRow>(
        "SELECT id, agent_id, name, kind, target, interval_secs, timeout_secs, \
         expect_status, expect_body, enabled, last_run_at, last_state, \
         last_latency_ms, last_detail, updated_at, env_json \
         FROM health_probes WHERE agent_id = ? ORDER BY name",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
}

pub async fn upsert_health_probe(
    pool: &SqlitePool,
    agent_id: &str,
    name: &str,
    kind: &str,
    target: &str,
    interval_secs: i64,
    timeout_secs: i64,
    expect_status: Option<i64>,
    expect_body: Option<&str>,
    enabled: bool,
    env_json: &str,
    now: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO health_probes (
            agent_id, name, kind, target, interval_secs, timeout_secs,
            expect_status, expect_body, enabled, env_json, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(agent_id, name) DO UPDATE SET
            kind          = excluded.kind,
            target        = excluded.target,
            interval_secs = excluded.interval_secs,
            timeout_secs  = excluded.timeout_secs,
            expect_status = excluded.expect_status,
            expect_body   = excluded.expect_body,
            enabled       = excluded.enabled,
            env_json      = excluded.env_json,
            updated_at    = excluded.updated_at
        "#,
    )
    .bind(agent_id)
    .bind(name)
    .bind(kind)
    .bind(target)
    .bind(interval_secs)
    .bind(timeout_secs)
    .bind(expect_status)
    .bind(expect_body)
    .bind(if enabled { 1 } else { 0 })
    .bind(env_json)
    .bind(now)
    .execute(pool)
    .await?;
    let id: i64 = sqlx::query_scalar(
        "SELECT id FROM health_probes WHERE agent_id = ?1 AND name = ?2",
    )
    .bind(agent_id)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn delete_health_probe(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<(String, String)>, sqlx::Error> {
    // Return (agent_id, name) of the deleted row so the caller can
    // re-push the new probe set to the affected agent.
    let row: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT agent_id, name FROM health_probes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    if row.is_some() {
        sqlx::query("DELETE FROM health_probes WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(row)
}

pub async fn record_health_probe_result(
    pool: &SqlitePool,
    id: i64,
    last_run_at: i64,
    last_state: &str,
    last_latency_ms: i64,
    last_detail: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE health_probes
           SET last_run_at = ?2,
               last_state = ?3,
               last_latency_ms = ?4,
               last_detail = ?5
         WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(last_run_at)
    .bind(last_state)
    .bind(last_latency_ms)
    .bind(last_detail)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FanOutRunRow {
    pub id: i64,
    pub kind: String,
    pub payload: Option<String>,
    pub started_at: i64,
    pub actor: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FanOutResultRow {
    pub run_id: i64,
    pub agent_id: String,
    pub status: String,
    pub detail: Option<String>,
    pub finished_at: Option<i64>,
}

pub async fn create_fan_out_run(
    pool: &SqlitePool,
    kind: &str,
    payload: Option<&str>,
    started_at: i64,
    actor: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO fan_out_runs (kind, payload, started_at, actor) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(kind)
    .bind(payload)
    .bind(started_at)
    .bind(actor)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub async fn upsert_fan_out_result(
    pool: &SqlitePool,
    run_id: i64,
    agent_id: &str,
    status: &str,
    detail: Option<&str>,
    finished_at: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO fan_out_results (run_id, agent_id, status, detail, finished_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(run_id, agent_id) DO UPDATE SET
            status = excluded.status,
            detail = excluded.detail,
            finished_at = excluded.finished_at
        "#,
    )
    .bind(run_id)
    .bind(agent_id)
    .bind(status)
    .bind(detail)
    .bind(finished_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_fan_out_runs(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<FanOutRunRow>, sqlx::Error> {
    sqlx::query_as::<_, FanOutRunRow>(
        "SELECT id, kind, payload, started_at, actor FROM fan_out_runs \
         ORDER BY started_at DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_fan_out_run(
    pool: &SqlitePool,
    run_id: i64,
) -> Result<Option<FanOutRunRow>, sqlx::Error> {
    sqlx::query_as::<_, FanOutRunRow>(
        "SELECT id, kind, payload, started_at, actor FROM fan_out_runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_fan_out_results(
    pool: &SqlitePool,
    run_id: i64,
) -> Result<Vec<FanOutResultRow>, sqlx::Error> {
    sqlx::query_as::<_, FanOutResultRow>(
        "SELECT run_id, agent_id, status, detail, finished_at \
         FROM fan_out_results WHERE run_id = ? ORDER BY agent_id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

pub async fn pending_fan_out_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
    kind: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT r.run_id FROM fan_out_results r \
         JOIN fan_out_runs f ON f.id = r.run_id \
         WHERE r.agent_id = ?1 AND f.kind = ?2 AND r.status = 'pending' \
         ORDER BY f.started_at ASC LIMIT 1",
    )
    .bind(agent_id)
    .bind(kind)
    .fetch_optional(pool)
    .await
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct NotificationRow {
    pub id: i64,
    pub kind: String,
    pub agent_id: Option<String>,
    pub level: String,
    pub title: String,
    pub body: Option<String>,
    pub created_at: i64,
    pub read_at: Option<i64>,
}

pub async fn insert_notification(
    pool: &SqlitePool,
    kind: &str,
    agent_id: Option<&str>,
    level: &str,
    title: &str,
    body: Option<&str>,
    created_at: i64,
) -> Result<i64, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO notifications (kind, agent_id, level, title, body, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(kind)
    .bind(agent_id)
    .bind(level)
    .bind(title)
    .bind(body)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub async fn list_notifications(
    pool: &SqlitePool,
    limit: i64,
    only_unread: bool,
) -> Result<Vec<NotificationRow>, sqlx::Error> {
    let q = if only_unread {
        "SELECT id, kind, agent_id, level, title, body, created_at, read_at \
         FROM notifications WHERE read_at IS NULL \
         ORDER BY created_at DESC, id DESC LIMIT ?"
    } else {
        "SELECT id, kind, agent_id, level, title, body, created_at, read_at \
         FROM notifications ORDER BY created_at DESC, id DESC LIMIT ?"
    };
    sqlx::query_as::<_, NotificationRow>(q)
        .bind(limit)
        .fetch_all(pool)
        .await
}

pub async fn unread_notification_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM notifications WHERE read_at IS NULL")
        .fetch_one(pool)
        .await
}

pub async fn mark_notification_read(
    pool: &SqlitePool,
    id: i64,
    now: i64,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("UPDATE notifications SET read_at = ?2 WHERE id = ?1 AND read_at IS NULL")
        .bind(id)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn mark_all_notifications_read(
    pool: &SqlitePool,
    now: i64,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("UPDATE notifications SET read_at = ? WHERE read_at IS NULL")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn delete_notification(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM notifications WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---------- agent_labels ----------

pub async fn list_all_labels(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT agent_id, label FROM agent_labels ORDER BY agent_id, label",
    )
    .fetch_all(pool)
    .await
}

pub async fn agents_for_label(pool: &SqlitePool, label: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT agent_id FROM agent_labels WHERE label = ? ORDER BY agent_id",
    )
    .bind(label)
    .fetch_all(pool)
    .await
}

pub async fn add_label(pool: &SqlitePool, agent_id: &str, label: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO agent_labels (agent_id, label) VALUES (?1, ?2)")
        .bind(agent_id)
        .bind(label)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn remove_label(
    pool: &SqlitePool,
    agent_id: &str,
    label: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM agent_labels WHERE agent_id = ?1 AND label = ?2")
        .bind(agent_id)
        .bind(label)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---------- backup_jobs ----------

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BackupJobRow {
    pub id: i64,
    pub agent_id: String,
    pub name: String,
    pub paths_json: String,
    pub dest: String,
    pub cron_expr: Option<String>,
    pub enabled: i64,
    pub last_run_at: i64,
    pub last_status: Option<String>,
    pub last_archive_path: Option<String>,
    pub last_bytes: Option<i64>,
    pub last_log: Option<String>,
    pub updated_at: i64,
    pub mode: String,
}

pub async fn list_backup_jobs(pool: &SqlitePool) -> Result<Vec<BackupJobRow>, sqlx::Error> {
    sqlx::query_as::<_, BackupJobRow>(
        "SELECT id, agent_id, name, paths_json, dest, cron_expr, enabled, \
         last_run_at, last_status, last_archive_path, last_bytes, last_log, updated_at, mode \
         FROM backup_jobs ORDER BY agent_id, name",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_backup_job(pool: &SqlitePool, id: i64) -> Result<Option<BackupJobRow>, sqlx::Error> {
    sqlx::query_as::<_, BackupJobRow>(
        "SELECT id, agent_id, name, paths_json, dest, cron_expr, enabled, \
         last_run_at, last_status, last_archive_path, last_bytes, last_log, updated_at, mode \
         FROM backup_jobs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_backup_job(
    pool: &SqlitePool,
    agent_id: &str,
    name: &str,
    paths_json: &str,
    dest: &str,
    cron_expr: Option<&str>,
    enabled: bool,
    mode: &str,
    now: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO backup_jobs (agent_id, name, paths_json, dest, cron_expr, enabled, mode, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(agent_id, name) DO UPDATE SET
            paths_json = excluded.paths_json,
            dest       = excluded.dest,
            cron_expr  = excluded.cron_expr,
            enabled    = excluded.enabled,
            mode       = excluded.mode,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(agent_id)
    .bind(name)
    .bind(paths_json)
    .bind(dest)
    .bind(cron_expr)
    .bind(if enabled { 1 } else { 0 })
    .bind(mode)
    .bind(now)
    .execute(pool)
    .await?;
    let id: i64 = sqlx::query_scalar(
        "SELECT id FROM backup_jobs WHERE agent_id = ?1 AND name = ?2",
    )
    .bind(agent_id)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn delete_backup_job(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM backup_jobs WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn record_backup_job_result(
    pool: &SqlitePool,
    id: i64,
    last_run_at: i64,
    last_status: &str,
    last_archive_path: &str,
    last_bytes: i64,
    last_log: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE backup_jobs
           SET last_run_at = ?2,
               last_status = ?3,
               last_archive_path = ?4,
               last_bytes = ?5,
               last_log = ?6
         WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(last_run_at)
    .bind(last_status)
    .bind(last_archive_path)
    .bind(last_bytes)
    .bind(last_log)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn recent_audit(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditRow>, sqlx::Error> {
    sqlx::query_as::<_, AuditRow>(
        "SELECT id, ts, actor, agent_id, kind, ok, detail FROM audit ORDER BY ts DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// CE: drop audit rows older than `cutoff_ts`. Called from a daily
/// task in `main` to honor the 7-day local retention promise.
pub async fn purge_audit_before(pool: &SqlitePool, cutoff_ts: i64) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("DELETE FROM audit WHERE ts < ?")
        .bind(cutoff_ts)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ---------- users ----------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    /// Required for sqlx::FromRow mapping of the SELECT * row even
    /// though the struct's consumers only read `role` / `totp_*` /
    /// timestamp fields. Dead-code analysis can't see the FromRow
    /// derive's reflection-style usage.
    #[allow(dead_code)]
    pub login: String,
    pub role: String,
    pub totp_enabled: i64,
    pub totp_secret: Option<String>,
    pub totp_recovery_hashes: String,
    #[allow(dead_code)]
    pub created_at: i64,
    #[allow(dead_code)]
    pub last_login_at: i64,
    pub session_epoch: i64,
}

pub async fn get_user(pool: &SqlitePool, login: &str) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT login, role, totp_enabled, totp_secret, totp_recovery_hashes, \
         created_at, last_login_at, session_epoch FROM users WHERE login = ?",
    )
    .bind(login)
    .fetch_optional(pool)
    .await
}

/// Bumps the user's `session_epoch` to `now`. JWTs whose `iat` is
/// older than the new epoch are treated as logged-out by
/// `claims_from_token`. Called on explicit logout, role change, and
/// MFA disable so a leaked-but-still-unexpired cookie can't keep
/// authenticating.
pub async fn bump_session_epoch(
    pool: &SqlitePool,
    login: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET session_epoch = ? WHERE login = ?")
        .bind(now)
        .bind(login)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count_users(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
}

pub async fn set_user_role(
    pool: &SqlitePool,
    login: &str,
    role: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("UPDATE users SET role = ? WHERE login = ?")
        .bind(role)
        .bind(login)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn set_user_totp(
    pool: &SqlitePool,
    login: &str,
    secret: &str,
    recovery_hashes_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET totp_enabled = 1, totp_secret = ?, totp_recovery_hashes = ? \
         WHERE login = ?",
    )
    .bind(secret)
    .bind(recovery_hashes_json)
    .bind(login)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_user_totp(pool: &SqlitePool, login: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET totp_enabled = 0, totp_secret = NULL, totp_recovery_hashes = '[]' \
         WHERE login = ?",
    )
    .bind(login)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_user_recovery_hashes(
    pool: &SqlitePool,
    login: &str,
    recovery_hashes_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET totp_recovery_hashes = ? WHERE login = ?")
        .bind(recovery_hashes_json)
        .bind(login)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UserListRow {
    pub login: String,
    pub role: String,
    pub totp_enabled: i64,
    pub created_at: i64,
    pub last_login_at: i64,
}

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<UserListRow>, sqlx::Error> {
    sqlx::query_as::<_, UserListRow>(
        "SELECT login, role, totp_enabled, created_at, last_login_at \
         FROM users ORDER BY login",
    )
    .fetch_all(pool)
    .await
}

/// Outcome of `upsert_login_with_seat_check`. The OAuth callback uses
/// this to distinguish "new seat consumed" (audit + welcome flow) from
/// "existing user signing in" (just a `last_login_at` bump).
pub enum SeatedUpsert {
    /// The login already had a row; `last_login_at` was bumped.
    Existing(UserRow),
    /// A new row was created; this seat is now occupied.
    Created(UserRow),
    /// No row existed and the seat cap was at capacity.
    SeatCapReached,
}

/// Transactional version of `upsert_login` that enforces the CE seat
/// cap atomically. The previous "count then insert" sequence had a
/// race where two concurrent first-time sign-ins could both pass the
/// COUNT check before either committed. Wrapping the whole sequence
/// in `BEGIN ... COMMIT` (with sqlite's default serialised-write
/// model) closes that window.
pub async fn upsert_login_with_seat_check(
    pool: &SqlitePool,
    login: &str,
    default_role_if_new: &str,
    now: i64,
    seat_limit: i64,
) -> Result<SeatedUpsert, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let existing: Option<UserRow> = sqlx::query_as::<_, UserRow>(
        "SELECT login, role, totp_enabled, totp_secret, totp_recovery_hashes, \
         created_at, last_login_at, session_epoch FROM users WHERE login = ?",
    )
    .bind(login)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(_) = existing.as_ref() {
        sqlx::query("UPDATE users SET last_login_at = ?2 WHERE login = ?1")
            .bind(login)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        let row: UserRow = sqlx::query_as::<_, UserRow>(
            "SELECT login, role, totp_enabled, totp_secret, totp_recovery_hashes, \
             created_at, last_login_at, session_epoch FROM users WHERE login = ?",
        )
        .bind(login)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(SeatedUpsert::Existing(row));
    }

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if count >= seat_limit {
        // Roll back; nothing was written.
        tx.rollback().await?;
        return Ok(SeatedUpsert::SeatCapReached);
    }

    sqlx::query(
        r#"
        INSERT INTO users (login, role, created_at, last_login_at)
        VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(login)
    .bind(default_role_if_new)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let row: UserRow = sqlx::query_as::<_, UserRow>(
        "SELECT login, role, totp_enabled, totp_secret, totp_recovery_hashes, \
         created_at, last_login_at, session_epoch FROM users WHERE login = ?",
    )
    .bind(login)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(SeatedUpsert::Created(row))
}

