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
    std::env::var("DB_PATH").unwrap_or_else(|_| "/data/sys-manager.db".to_string())
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

pub async fn recent_audit(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditRow>, sqlx::Error> {
    sqlx::query_as::<_, AuditRow>(
        "SELECT id, ts, actor, agent_id, kind, ok, detail FROM audit ORDER BY ts DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

