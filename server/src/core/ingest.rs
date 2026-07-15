use super::{
    CoreEventBus,
    model::{CoreEventKind, SnapshotKind},
    repository,
};
use shared::Message;
use sqlx::SqlitePool;
use std::collections::HashMap;

pub async fn connected(
    pool: &SqlitePool,
    events: &CoreEventBus,
    agent_id: &str,
    hostname: &str,
    protocol_version: u32,
    capabilities: &[String],
    metadata: &HashMap<String, String>,
    now: i64,
) -> Result<(), sqlx::Error> {
    repository::upsert_connected(
        pool,
        agent_id,
        hostname,
        protocol_version,
        capabilities,
        metadata,
        now,
    )
    .await?;
    events.publish(CoreEventKind::HostConnected, Some(agent_id), now);
    Ok(())
}

pub async fn capabilities_updated(
    pool: &SqlitePool,
    events: &CoreEventBus,
    agent_id: &str,
    capabilities: &[String],
    now: i64,
) -> Result<(), sqlx::Error> {
    repository::update_capabilities(pool, agent_id, capabilities, now).await?;
    events.publish(CoreEventKind::HostUpdated, Some(agent_id), now);
    Ok(())
}

pub async fn touch(
    pool: &SqlitePool,
    events: &CoreEventBus,
    agent_id: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    repository::touch(pool, agent_id, now).await?;
    events.publish(CoreEventKind::HostUpdated, Some(agent_id), now);
    Ok(())
}

pub async fn message(
    pool: &SqlitePool,
    events: &CoreEventBus,
    agent_id: &str,
    message: &Message,
    now: i64,
) -> Result<(), sqlx::Error> {
    repository::touch(pool, agent_id, now).await?;
    let kind = match message {
        Message::SystemStatsResponse { .. } => SnapshotKind::System,
        Message::ListServicesResponse { .. } => SnapshotKind::Services,
        Message::DockerListResponse { .. } => SnapshotKind::Docker,
        Message::SwarmListResponse { .. } => SnapshotKind::Swarm,
        _ => return Ok(()),
    };
    let payload =
        serde_json::to_value(message).map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
    repository::put_snapshot(pool, agent_id, kind, &payload, now).await?;
    events.publish(CoreEventKind::HostUpdated, Some(agent_id), now);
    Ok(())
}

pub async fn disconnected(
    pool: &SqlitePool,
    events: &CoreEventBus,
    agent_id: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    repository::mark_disconnected(pool, agent_id, now).await?;
    events.publish(CoreEventKind::HostDisconnected, Some(agent_id), now);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        repository::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn system_response_becomes_a_durable_snapshot_and_event() {
        let pool = test_pool().await;
        let bus = CoreEventBus::new(8);
        connected(
            &pool,
            &bus,
            "node-a-id",
            "node-a",
            19,
            &["systemd".into()],
            &std::collections::HashMap::new(),
            100,
        )
        .await
        .unwrap();
        let mut receiver = bus.subscribe();
        message(
            &pool,
            &bus,
            "node-a-id",
            &shared::Message::SystemStatsResponse {
                hostname: "node-a".into(),
                kernel: "6.12".into(),
                uptime_secs: 9,
                cpu_count: 4,
                load_1: 0.1,
                load_5: 0.2,
                load_15: 0.3,
                mem_total_kb: 1000,
                mem_available_kb: 700,
                swap_total_kb: 0,
                swap_free_kb: 0,
                root_disk_total_kb: 2000,
                root_disk_used_kb: 500,
            },
            101,
        )
        .await
        .unwrap();
        assert_eq!(
            receiver.recv().await.unwrap().kind,
            CoreEventKind::HostUpdated
        );
        let fleet = repository::list_fleet(&pool, 101).await.unwrap();
        assert_eq!(fleet.hosts[0].system.as_ref().unwrap().observed_at, 101);
    }
}
