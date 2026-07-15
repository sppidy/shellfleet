use super::model::{ConnectionStatus, FleetHost, FleetResponse, SnapshotKind, SnapshotValue};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use std::collections::{BTreeMap, HashMap};

const OFFLINE_AFTER_SECONDS: i64 = 45;

#[derive(FromRow)]
struct HostRow {
    agent_id: String,
    hostname: String,
    protocol_version: i64,
    capabilities_json: String,
    metadata_json: String,
    first_seen_at: i64,
    last_seen_at: i64,
    disconnected_at: Option<i64>,
}

#[derive(FromRow)]
struct SnapshotRow {
    agent_id: String,
    kind: String,
    observed_at: i64,
    payload_json: String,
}

fn encode<T: Serialize + ?Sized>(value: &T) -> Result<String, sqlx::Error> {
    serde_json::to_string(value).map_err(|error| sqlx::Error::Encode(Box::new(error)))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, sqlx::Error> {
    serde_json::from_str(value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn decode_protocol_version(value: i64) -> Result<u32, sqlx::Error> {
    u32::try_from(value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(
        r#"
        CREATE TABLE IF NOT EXISTS core_hosts (
            agent_id TEXT PRIMARY KEY,
            hostname TEXT NOT NULL,
            protocol_version INTEGER NOT NULL,
            capabilities_json TEXT NOT NULL,
            metadata_json TEXT NOT NULL,
            first_seen_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            disconnected_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS core_hosts_last_seen
            ON core_hosts(last_seen_at DESC);
        CREATE TABLE IF NOT EXISTS core_snapshots (
            agent_id TEXT NOT NULL REFERENCES core_hosts(agent_id) ON DELETE CASCADE,
            kind TEXT NOT NULL CHECK(kind IN ('system','services','docker','swarm')),
            observed_at INTEGER NOT NULL,
            payload_json TEXT NOT NULL,
            PRIMARY KEY(agent_id, kind)
        );
        CREATE INDEX IF NOT EXISTS core_snapshots_observed
            ON core_snapshots(observed_at DESC);
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_connected(
    pool: &SqlitePool,
    agent_id: &str,
    hostname: &str,
    protocol_version: u32,
    capabilities: &[String],
    metadata: &HashMap<String, String>,
    now: i64,
) -> Result<(), sqlx::Error> {
    let capabilities_json = encode(capabilities)?;
    let metadata_json = encode(metadata)?;
    let mut transaction = pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO core_hosts(
               agent_id, hostname, protocol_version, capabilities_json,
               metadata_json, first_seen_at, last_seen_at, disconnected_at
           ) VALUES (?, ?, ?, ?, ?, ?, ?, NULL)
           ON CONFLICT(agent_id) DO UPDATE SET
               hostname = excluded.hostname,
               protocol_version = excluded.protocol_version,
               capabilities_json = excluded.capabilities_json,
               metadata_json = excluded.metadata_json,
               last_seen_at = excluded.last_seen_at,
               disconnected_at = NULL"#,
    )
    .bind(agent_id)
    .bind(hostname)
    .bind(i64::from(protocol_version))
    .bind(capabilities_json)
    .bind(metadata_json)
    .bind(now)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn touch(pool: &SqlitePool, agent_id: &str, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE core_hosts SET last_seen_at = ?, disconnected_at = NULL WHERE agent_id = ?",
    )
    .bind(now)
    .bind(agent_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_capabilities(
    pool: &SqlitePool,
    agent_id: &str,
    capabilities: &[String],
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE core_hosts SET capabilities_json = ?, last_seen_at = ? WHERE agent_id = ?")
        .bind(encode(capabilities)?)
        .bind(now)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_disconnected(
    pool: &SqlitePool,
    agent_id: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE core_hosts SET disconnected_at = ? WHERE agent_id = ?")
        .bind(now)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_all_disconnected(pool: &SqlitePool, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE core_hosts SET disconnected_at = ? WHERE disconnected_at IS NULL")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn put_snapshot(
    pool: &SqlitePool,
    agent_id: &str,
    kind: SnapshotKind,
    payload: &serde_json::Value,
    observed_at: i64,
) -> Result<(), sqlx::Error> {
    let payload_json = encode(payload)?;
    sqlx::query(
        r#"INSERT INTO core_snapshots(agent_id, kind, observed_at, payload_json)
           VALUES (?, ?, ?, ?)
           ON CONFLICT(agent_id, kind) DO UPDATE SET
               observed_at = excluded.observed_at,
               payload_json = excluded.payload_json
           WHERE excluded.observed_at >= core_snapshots.observed_at"#,
    )
    .bind(agent_id)
    .bind(kind.as_str())
    .bind(observed_at)
    .bind(payload_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_fleet(pool: &SqlitePool, now: i64) -> Result<FleetResponse, sqlx::Error> {
    let hosts = sqlx::query_as::<_, HostRow>(
        "SELECT agent_id, hostname, protocol_version, capabilities_json, metadata_json, \
         first_seen_at, last_seen_at, disconnected_at \
         FROM core_hosts ORDER BY hostname COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    let snapshots = sqlx::query_as::<_, SnapshotRow>(
        "SELECT agent_id, kind, observed_at, payload_json FROM core_snapshots",
    )
    .fetch_all(pool)
    .await?;

    let mut by_host: HashMap<String, HashMap<String, SnapshotValue>> = HashMap::new();
    for row in snapshots {
        by_host.entry(row.agent_id).or_default().insert(
            row.kind,
            SnapshotValue {
                observed_at: row.observed_at,
                value: decode(&row.payload_json)?,
            },
        );
    }

    let mut output = Vec::with_capacity(hosts.len());
    for row in hosts {
        let mut values = by_host.remove(&row.agent_id).unwrap_or_default();
        let online = row.disconnected_at.is_none()
            && now.saturating_sub(row.last_seen_at) <= OFFLINE_AFTER_SECONDS;
        output.push(FleetHost {
            agent_id: row.agent_id,
            hostname: row.hostname,
            status: if online {
                ConnectionStatus::Online
            } else {
                ConnectionStatus::Offline
            },
            protocol_version: decode_protocol_version(row.protocol_version)?,
            capabilities: decode(&row.capabilities_json)?,
            metadata: decode::<BTreeMap<String, String>>(&row.metadata_json)?,
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
            disconnected_at: row.disconnected_at,
            system: values.remove("system"),
            services: values.remove("services"),
            docker: values.remove("docker"),
            swarm: values.remove("swarm"),
        });
    }

    Ok(FleetResponse {
        generated_at: now,
        offline_after_seconds: OFFLINE_AFTER_SECONDS,
        hosts: output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn fleet_survives_disconnect_and_keeps_last_snapshot() {
        let pool = pool().await;
        upsert_connected(
            &pool,
            "node-a-id",
            "node-a",
            19,
            &["systemd".into()],
            &std::collections::HashMap::new(),
            100,
        )
        .await
        .unwrap();
        put_snapshot(
            &pool,
            "node-a-id",
            SnapshotKind::System,
            &serde_json::json!({"hostname":"node-a","cpu_count":4}),
            101,
        )
        .await
        .unwrap();
        mark_disconnected(&pool, "node-a-id", 110).await.unwrap();

        let fleet = list_fleet(&pool, 111).await.unwrap();
        assert_eq!(fleet.hosts.len(), 1);
        assert_eq!(fleet.hosts[0].status, ConnectionStatus::Offline);
        assert_eq!(fleet.hosts[0].system.as_ref().unwrap().observed_at, 101);
    }

    #[tokio::test]
    async fn last_seen_older_than_threshold_is_offline() {
        let pool = pool().await;
        upsert_connected(
            &pool,
            "node-a-id",
            "node-a",
            19,
            &[],
            &std::collections::HashMap::new(),
            100,
        )
        .await
        .unwrap();
        let fleet = list_fleet(&pool, 146).await.unwrap();
        assert_eq!(fleet.hosts[0].status, ConnectionStatus::Offline);
    }

    #[tokio::test]
    async fn startup_fence_requires_a_fresh_agent_registration() {
        let pool = pool().await;
        upsert_connected(
            &pool,
            "node-a-id",
            "node-a",
            19,
            &[],
            &std::collections::HashMap::new(),
            100,
        )
        .await
        .unwrap();
        mark_all_disconnected(&pool, 101).await.unwrap();
        assert_eq!(
            list_fleet(&pool, 101).await.unwrap().hosts[0].status,
            ConnectionStatus::Offline,
        );
    }
}
