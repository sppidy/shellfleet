# ShellFleet Durable Fleet Read Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Fleet overview render durable host, system, service, Docker, and Swarm state from SQLite through REST, with SSE as a non-essential live enhancement and a real browser-plus-agent release journey.

**Architecture:** The existing agent WebSocket remains the collection transport for this slice, but the Rust server becomes the owner of polling and persistence. Agent messages are projected into focused `core_hosts` and `core_snapshots` tables; `/api/core/v1/fleet` reconstructs the UI from SQLite; `/api/core/v1/events` only tells clients when to refetch. The current Next.js application remains temporarily, but its Fleet overview stops depending on `/ui/ws`; later slices can replace the web runtime without migrating the product state model again.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio, SQLx SQLite, serde, Next.js 16/React 19, TypeScript 6, Vitest, Playwright, Docker Compose, Nginx.

## Global Constraints

- Target customer: technically capable operators responsible for 3-100 Linux machines, especially Docker and Docker Swarm hosts.
- Initial product promise: "See what is unhealthy, understand why, take a safe action, and verify recovery from one place."
- Fleet truth is durable; browser reconnects never erase inventory or snapshots.
- Initial screen reads use authenticated REST; SSE is optional enhancement and browser WebSocket is not used by Fleet.
- One canonical HTTPS origin remains the production network model.
- Offline detection must occur within 45 seconds and reconnect must appear within 10 seconds.
- Existing agent messages remain wire-compatible in this slice; do not bump `shared::PROTOCOL_VERSION`.
- SQLite remains the only server-side data dependency.
- Existing non-core routes stay available only for migration compatibility; this slice must not expand them.
- Every commit is independently testable and must preserve the existing CI gates.

---

## Program decomposition

The product-reset design covers several independently reviewable systems. They execute in this order, each with its own plan and acceptance gate:

1. **Durable Fleet read plane — this plan.** Server-owned collection, SQLite projections, REST, SSE, Fleet UI migration, and the first browser-plus-agent journey.
2. **Local bootstrap and agent identity.** First-admin bootstrap, Argon2id credentials, TOTP, Ed25519 device enrollment, one-origin agent connectivity, and migration from tokens/mTLS.
3. **Host read surfaces.** Host overview, services, containers, Swarm topology, updates, and bounded logs on the durable API.
4. **Durable operations.** Operation resource/state machine, typed privileged-helper dispatch, retry/reconnect behavior, Activity UI, and service/container mutations.
5. **Packaging and CLI.** Static Vite assets embedded in the Rust server, single-container deployment, browser-approved CLI login, doctor/read commands, and Debian enrollment flow.
6. **Repository and legacy removal.** Public-submodule absorption, one release version, default-navigation reduction, old UI WebSocket deletion, obsolete route/protocol deletion, and public-documentation rewrite.

Each later plan consumes the stable REST, event, identity, or operation contracts produced by the preceding plan. No later plan may restore browser-owned fleet truth.

## File structure for this slice

### Server submodule

- Create `server/src/core/mod.rs`: module boundary and router composition.
- Create `server/src/core/model.rs`: serialized core domain types and snapshot kinds.
- Create `server/src/core/repository.rs`: only SQL and SQLite row mapping for core hosts/snapshots.
- Create `server/src/core/events.rs`: in-process invalidation bus and SSE response conversion.
- Create `server/src/core/ingest.rs`: projection of existing `shared::Message` values into the repository.
- Create `server/src/core/collector.rs`: server-owned periodic read requests to connected agents.
- Create `server/src/core/http.rs`: authenticated Fleet REST and SSE handlers.
- Modify `server/src/db.rs`: invoke the isolated core schema migration.
- Modify `server/src/main.rs`: own the event bus, start collection, ingest registration/messages/disconnects, and mount core routes.

### Web submodule

- Create `web/src/lib/coreFleet.ts`: API response types and one strict REST decoder boundary.
- Create `web/src/components/providers/CoreFleetProvider.tsx`: REST state, SSE invalidation, stale-state behavior, and refresh.
- Create `web/src/components/providers/__tests__/CoreFleetProvider.test.tsx`: transport-failure behavior.
- Modify `web/src/app/layout.tsx`: mount the core provider without removing legacy providers yet.
- Modify `web/src/components/FleetOverview.tsx`: consume durable hosts and display online/offline/data age correctly.
- Modify `web/src/app/overview/page.tsx`: report REST/SSE status instead of `/ui/ws` status.
- Modify `web/package.json` and `web/package-lock.json`: add React Testing Library and Playwright test dependencies/scripts.

### Root repository

- Create `tests/journey/docker-compose.yml`: production-shaped server/web/ingress plus the released agent image.
- Create `tests/journey/nginx.conf`: one-origin routing for UI, API, SSE, and legacy WS during migration.
- Create `tests/journey/agent-token.txt`: non-secret fixture matching the journey server's legacy development token.
- Create `tests/journey/playwright.config.ts`: build-artifact browser test configuration.
- Create `tests/journey/fleet-read-plane.spec.ts`: visible-host, refresh, disconnect, stale-state, and reconnect assertions.
- Modify `.github/workflows/ci.yml`: add a journey job after Rust and web gates.

---

### Task 1: Core domain types and snapshot contract

**Files:**
- Create: `server/src/core/mod.rs`
- Create: `server/src/core/model.rs`
- Modify: `server/src/main.rs:8-40`

**Interfaces:**
- Consumes: `shared::Message` only in later ingestion code; this task is transport-independent.
- Produces: `SnapshotKind`, `ConnectionStatus`, `FleetHost`, `FleetResponse`, `CoreEvent`, and `CoreEventKind` with stable serde names.

- [ ] **Step 1: Write model serialization tests in `server/src/core/model.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_kind_has_stable_database_names() {
        assert_eq!(SnapshotKind::System.as_str(), "system");
        assert_eq!(SnapshotKind::Services.as_str(), "services");
        assert_eq!(SnapshotKind::Docker.as_str(), "docker");
        assert_eq!(SnapshotKind::Swarm.as_str(), "swarm");
    }

    #[test]
    fn fleet_status_serializes_in_lowercase() {
        assert_eq!(
            serde_json::to_string(&ConnectionStatus::Offline).unwrap(),
            "\"offline\""
        );
    }
}
```

- [ ] **Step 2: Run the focused test and verify the missing module failure**

Run: `cargo test -p server core::model::tests -- --nocapture`

Expected: FAIL because `core` and its model types do not exist.

- [ ] **Step 3: Define the core domain types**

Create `server/src/core/mod.rs`:

```rust
pub mod collector;
pub mod events;
pub mod http;
pub mod ingest;
pub mod model;
pub mod repository;

pub use events::CoreEventBus;

pub fn routes() -> axum::Router<std::sync::Arc<crate::AppState>> {
    http::routes()
}
```

Create `server/src/core/model.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    System,
    Services,
    Docker,
    Swarm,
}

impl SnapshotKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Services => "services",
            Self::Docker => "docker",
            Self::Swarm => "swarm",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotValue {
    pub observed_at: i64,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetHost {
    pub agent_id: String,
    pub hostname: String,
    pub status: ConnectionStatus,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub disconnected_at: Option<i64>,
    pub system: Option<SnapshotValue>,
    pub services: Option<SnapshotValue>,
    pub docker: Option<SnapshotValue>,
    pub swarm: Option<SnapshotValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetResponse {
    pub generated_at: i64,
    pub offline_after_seconds: i64,
    pub hosts: Vec<FleetHost>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreEventKind {
    HostConnected,
    HostDisconnected,
    HostUpdated,
    ResyncRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreEvent {
    pub id: u64,
    pub kind: CoreEventKind,
    pub agent_id: Option<String>,
    pub observed_at: i64,
}
```

Add `mod core;` beside the other module declarations in `server/src/main.rs`.

- [ ] **Step 4: Run model tests**

Run: `cargo test -p server core::model::tests -- --nocapture`

Expected: PASS with 2 tests.

- [ ] **Step 5: Commit the model boundary**

```bash
git -C server add src/core/mod.rs src/core/model.rs src/main.rs
git -C server commit -S -m "feat: define durable fleet domain"
```

---

### Task 2: SQLite host and snapshot repository

**Files:**
- Create: `server/src/core/repository.rs`
- Modify: `server/src/db.rs:350-365`

**Interfaces:**
- Consumes: `SnapshotKind`, agent identity fields, JSON payloads, and Unix timestamps.
- Produces:
  - `migrate(pool: &SqlitePool) -> Result<(), sqlx::Error>`
  - `upsert_connected(pool, agent_id, hostname, protocol_version, capabilities, metadata, now)`
  - `touch(pool, agent_id, now)`
  - `update_capabilities(pool, agent_id, capabilities, now)`
  - `mark_disconnected(pool, agent_id, now)`
  - `mark_all_disconnected(pool, now)` as the server-startup live-state fence
  - `put_snapshot(pool, agent_id, kind, payload, observed_at)`
  - `list_fleet(pool, now) -> Result<FleetResponse, sqlx::Error>`

- [ ] **Step 1: Write repository tests using an in-memory single-connection pool**

```rust
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
```

- [ ] **Step 2: Run repository tests and verify failure**

Run: `cargo test -p server core::repository::tests -- --nocapture`

Expected: FAIL because repository functions are undefined.

- [ ] **Step 3: Implement the schema and write methods**

Use these exact tables in `migrate`:

```sql
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
```

Use one transaction in `upsert_connected`; preserve `first_seen_at`, update identity fields, set `last_seen_at`, and clear `disconnected_at`. Use `INSERT ... ON CONFLICT(agent_id) DO UPDATE`.

`put_snapshot` must serialize before opening the query, then upsert `(agent_id, kind)` only when the new `observed_at` is greater than or equal to the existing timestamp:

```sql
INSERT INTO core_snapshots(agent_id, kind, observed_at, payload_json)
VALUES (?, ?, ?, ?)
ON CONFLICT(agent_id, kind) DO UPDATE SET
    observed_at = excluded.observed_at,
    payload_json = excluded.payload_json
WHERE excluded.observed_at >= core_snapshots.observed_at
```

`list_fleet` must load hosts ordered by `hostname COLLATE NOCASE`, load all matching snapshots in one second query, group them by agent ID, and derive `ConnectionStatus::Online` only when `disconnected_at IS NULL AND now - last_seen_at <= 45`. Invalid persisted JSON must return `sqlx::Error::Decode` rather than silently manufacturing empty state.

Use these exact repository signatures and row mappings so later tasks do not invent a second persistence API:

```rust
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

pub async fn upsert_connected(
    pool: &SqlitePool,
    agent_id: &str,
    hostname: &str,
    protocol_version: u32,
    capabilities: &[String],
    metadata: &HashMap<String, String>,
    now: i64,
) -> Result<(), sqlx::Error> {
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
    .bind(encode(capabilities)?)
    .bind(encode(metadata)?)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch(pool: &SqlitePool, agent_id: &str, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE core_hosts SET last_seen_at = ?, disconnected_at = NULL WHERE agent_id = ?")
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
    .bind(encode(payload)?)
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
            SnapshotValue { observed_at: row.observed_at, value: decode(&row.payload_json)? },
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
            status: if online { ConnectionStatus::Online } else { ConnectionStatus::Offline },
            protocol_version: u32::try_from(row.protocol_version).unwrap_or_default(),
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
```

- [ ] **Step 4: Invoke the core migration from `db::init`**

Insert before `migrate_legacy_tokens(&pool).await?`:

```rust
crate::core::repository::migrate(&pool).await?;
```

- [ ] **Step 5: Run repository and existing server tests**

Run: `cargo test -p server core::repository::tests -- --nocapture && cargo test -p server`

Expected: the 3 focused tests and all existing server tests PASS.

- [ ] **Step 6: Commit persistence**

```bash
git -C server add src/core/repository.rs src/db.rs
git -C server commit -S -m "feat: persist fleet state in sqlite"
```

---

### Task 3: Event bus and authenticated Fleet endpoints

**Files:**
- Create: `server/src/core/events.rs`
- Create: `server/src/core/http.rs`
- Modify: `server/src/core/mod.rs`
- Modify: `server/src/main.rs:143-190,521-620`

**Interfaces:**
- Consumes: `repository::list_fleet`, `CoreEvent`, existing `/api` session/RBAC middleware.
- Produces:
  - `CoreEventBus::new(capacity: usize)`
  - `CoreEventBus::publish(kind, agent_id, observed_at)`
  - `CoreEventBus::subscribe() -> broadcast::Receiver<CoreEvent>`
  - `core::routes() -> Router<Arc<AppState>>`
  - `GET /api/core/v1/fleet`
  - `GET /api/core/v1/events` as `text/event-stream`.

- [ ] **Step 1: Write event-bus tests**

```rust
#[tokio::test]
async fn published_events_have_monotonic_ids() {
    let bus = CoreEventBus::new(8);
    let mut rx = bus.subscribe();
    bus.publish(CoreEventKind::HostUpdated, Some("node-a-id"), 100);
    bus.publish(CoreEventKind::HostUpdated, Some("node-a-id"), 101);
    assert_eq!(rx.recv().await.unwrap().id, 1);
    assert_eq!(rx.recv().await.unwrap().id, 2);
}
```

- [ ] **Step 2: Run the event test and verify failure**

Run: `cargo test -p server core::events::tests -- --nocapture`

Expected: FAIL because `CoreEventBus` is undefined.

- [ ] **Step 3: Implement `CoreEventBus`**

Use this implementation in `server/src/core/events.rs`:

```rust
use super::model::{CoreEvent, CoreEventKind};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::broadcast;

struct Inner {
    tx: broadcast::Sender<CoreEvent>,
    next_id: AtomicU64,
}

#[derive(Clone)]
pub struct CoreEventBus {
    inner: Arc<Inner>,
}

impl CoreEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { inner: Arc::new(Inner { tx, next_id: AtomicU64::new(0) }) }
    }

    pub fn publish(&self, kind: CoreEventKind, agent_id: Option<&str>, observed_at: i64) {
        let event = CoreEvent {
            id: self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1,
            kind,
            agent_id: agent_id.map(str::to_owned),
            observed_at,
        };
        let _ = self.inner.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.inner.tx.subscribe()
    }
}
```

- [ ] **Step 4: Implement Fleet REST and SSE handlers**

`fleet` calls `repository::list_fleet(&state.db, crate::now_unix())` and returns JSON or `(500, "fleet repository unavailable")` while logging the SQL error.

`events` subscribes to `state.core_events` and uses `futures_util::stream::unfold`. A normal event becomes:

```rust
Event::default()
    .id(event.id.to_string())
    .event("fleet")
    .json_data(event)
```

A lagged receiver emits `CoreEventKind::ResyncRequired`; a closed sender ends the stream. Configure `KeepAlive::new().interval(Duration::from_secs(15)).text("keepalive")`.

Use this handler implementation in `server/src/core/http.rs`:

```rust
use super::{
    model::{CoreEvent, CoreEventKind},
    repository,
};
use crate::AppState;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Sse, sse::{Event, KeepAlive}},
    routing::get,
};
use futures_util::stream;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/fleet", get(fleet)).route("/events", get(events))
}

async fn fleet(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match repository::list_fleet(&state.db, crate::now_unix()).await {
        Ok(fleet) => Json(fleet).into_response(),
        Err(error) => {
            tracing::error!(%error, "durable fleet query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "fleet repository unavailable").into_response()
        }
    }
}

async fn events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.core_events.subscribe();
    let output = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(event) => Some((Ok(to_sse(event)), receiver)),
            Err(RecvError::Lagged(_)) => Some((
                Ok(to_sse(CoreEvent {
                    id: 0,
                    kind: CoreEventKind::ResyncRequired,
                    agent_id: None,
                    observed_at: crate::now_unix(),
                })),
                receiver,
            )),
            Err(RecvError::Closed) => None,
        }
    });
    Sse::new(output).keep_alive(
        KeepAlive::new().interval(Duration::from_secs(15)).text("keepalive"),
    )
}

fn to_sse(event: CoreEvent) -> Event {
    Event::default()
        .id(event.id.to_string())
        .event("fleet")
        .json_data(event)
        .expect("CoreEvent contains only serializable fields")
}
```

Create the router:

```rust
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/fleet", get(fleet))
        .route("/events", get(events))
}
```

- [ ] **Step 5: Add the event bus to `AppState` and mount the router**

Add `pub core_events: core::CoreEventBus` to `AppState`, initialize it with `core::CoreEventBus::new(256)`, and add this to the authenticated `api_routes` before middleware is applied:

```rust
.nest("/core/v1", core::routes())
```

Do not mount core routes directly on the top-level router; they must inherit existing session, MFA, RBAC, CSRF, body-limit, IP-allow-list, and security-header behavior.

- [ ] **Step 6: Run server tests and clippy**

Run: `cargo test -p server && cargo clippy -p server --all-targets -- -D warnings`

Expected: PASS with no warnings.

- [ ] **Step 7: Commit the read API**

```bash
git -C server add src/core/events.rs src/core/http.rs src/core/mod.rs src/main.rs
git -C server commit -S -m "feat: expose durable fleet api"
```

---

### Task 4: Server-owned collection and message projection

**Files:**
- Create: `server/src/core/collector.rs`
- Create: `server/src/core/ingest.rs`
- Modify: `server/src/main.rs:919-1185,1540-1605`

**Interfaces:**
- Consumes: live `AppState::agents`, existing `shared::Message` response variants, repository functions, and `CoreEventBus`.
- Produces:
  - `collector::spawn(state: Arc<AppState>)`
  - `ingest::connected(...)`
  - `ingest::capabilities_updated(...)`
  - `ingest::touch(...)`
  - `ingest::message(...)`
  - `ingest::disconnected(...)`.

- [ ] **Step 1: Write projection tests**

```rust
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
    let mut rx = bus.subscribe();
    message(
        &pool,
        &bus,
        "node-a-id",
        &shared::Message::SystemStatsResponse {
            hostname: "node-a".into(), kernel: "6.12".into(), uptime_secs: 9,
            cpu_count: 4, load_1: 0.1, load_5: 0.2, load_15: 0.3,
            mem_total_kb: 1000, mem_available_kb: 700,
            swap_total_kb: 0, swap_free_kb: 0,
            root_disk_total_kb: 2000, root_disk_used_kb: 500,
        },
        101,
    )
    .await
    .unwrap();
    assert_eq!(rx.recv().await.unwrap().kind, CoreEventKind::HostUpdated);
    let fleet = repository::list_fleet(&pool, 101).await.unwrap();
    assert_eq!(fleet.hosts[0].system.as_ref().unwrap().observed_at, 101);
}
```

- [ ] **Step 2: Run projection tests and verify failure**

Run: `cargo test -p server core::ingest::tests -- --nocapture`

Expected: FAIL because ingestion functions are undefined.

- [ ] **Step 3: Implement typed projection**

`message` must call `touch` for every authenticated agent message. It persists only these existing response variants:

```rust
match message {
    Message::SystemStatsResponse { .. } => SnapshotKind::System,
    Message::ListServicesResponse { .. } => SnapshotKind::Services,
    Message::DockerListResponse { .. } => SnapshotKind::Docker,
    Message::SwarmListResponse { .. } => SnapshotKind::Swarm,
    _ => return Ok(()),
}
```

Serialize the complete tagged `shared::Message`, not a manually duplicated field list. This preserves exact current payload semantics and lets the HTTP decoder reject unexpected message types at one boundary. After `put_snapshot` succeeds, publish `HostUpdated`.

`connected`, `capabilities_updated`, and `disconnected` write the repository first and publish only after a successful commit. A failed persistence write is logged and returned; it must never broadcast state that REST cannot load.

Use these function bodies in `server/src/core/ingest.rs`:

```rust
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
        pool, agent_id, hostname, protocol_version, capabilities, metadata, now,
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
    let payload = serde_json::to_value(message)
        .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
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
```

- [ ] **Step 4: Implement the collector**

Start one Tokio task with a 10-second interval. On each tick, clone the connected entries under the mutex, release the mutex, then send:

```rust
Message::SystemStatsRequest
Message::ListServicesRequest       // only with systemd or legacy empty caps
Message::DockerListRequest         // only with docker or legacy empty caps
Message::SwarmListRequest          // only with swarm or legacy empty caps
```

Never await while holding `state.agents`. A closed channel is ignored because socket teardown owns removal. The immediate interval tick provides the first snapshot without any browser connection.

Use this implementation in `server/src/core/collector.rs`:

```rust
use crate::AppState;
use shared::Message;
use std::{sync::Arc, time::Duration};

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let agents = state
                .agents
                .lock()
                .await
                .values()
                .map(|entry| (entry.tx.clone(), entry.capabilities.clone()))
                .collect::<Vec<_>>();
            for (tx, capabilities) in agents {
                let legacy = capabilities.is_empty();
                let has = |value: &str| capabilities.iter().any(|item| item == value);
                let _ = tx.send(Message::SystemStatsRequest);
                if legacy || has("systemd") {
                    let _ = tx.send(Message::ListServicesRequest);
                }
                if legacy || has("docker") {
                    let _ = tx.send(Message::DockerListRequest);
                }
                if legacy || has("swarm") {
                    let _ = tx.send(Message::SwarmListRequest);
                }
            }
        }
    });
}
```

- [ ] **Step 5: Wire ingestion into the agent lifecycle**

After successful Register insertion, call `ingest::connected`. After capability mutation, call `ingest::capabilities_updated`. Before the existing specialized handling of every `other` message, call `ingest::message`. On `Message::Ping`, call `ingest::touch` before sending `Pong`. When the current connection owns teardown, call `ingest::disconnected` immediately after removing the live entry.

Change the agent read timeout from 75 seconds to 45 seconds. The agent sends application `Ping` every approximately 20 seconds, so two missed pings now satisfy the product's offline threshold.

Start `core::collector::spawn(state.clone())` once after `AppState` construction.

Immediately after `db::init()` succeeds and before `AppState` is created, call:

```rust
if let Err(error) = core::repository::mark_all_disconnected(&pool, now_unix()).await {
    tracing::error!(%error, "failed to fence restored fleet connections");
    std::process::exit(1);
}
```

This startup fence is mandatory: persisted inventory and snapshots survive restart, but no host is reported online until its current agent connection registers.

- [ ] **Step 6: Run all server tests and clippy**

Run: `cargo test -p server && cargo clippy -p server --all-targets -- -D warnings`

Expected: PASS; projection tests prove the server has state before any UI connection.

- [ ] **Step 7: Commit collection ownership**

```bash
git -C server add src/core/collector.rs src/core/ingest.rs src/main.rs
git -C server commit -S -m "feat: collect fleet snapshots on server"
```

---

### Task 5: Strict web API boundary and resilient provider

**Files:**
- Create: `web/src/lib/coreFleet.ts`
- Create: `web/src/components/providers/CoreFleetProvider.tsx`
- Create: `web/src/components/providers/__tests__/CoreFleetProvider.test.tsx`
- Modify: `web/package.json`
- Modify: `web/package-lock.json`

**Interfaces:**
- Consumes: `GET /api/core/v1/fleet`, `GET /api/core/v1/events`, existing `apiFetch`, and `SessionProvider`.
- Produces: `useCoreFleet()` with `{hosts, snapshots, liveStatus, loading, error, refresh}`. `liveStatus` is `connecting | live | degraded` and never represents agent status.

- [ ] **Step 1: Add React test dependencies**

Run from `web/`:

```bash
npm install --save-dev @testing-library/react @testing-library/jest-dom
```

Expected: `package.json` and `package-lock.json` update with exact resolved versions.

- [ ] **Step 2: Write a provider test proving SSE failure preserves REST state**

```tsx
it('keeps durable hosts visible when the event stream fails', async () => {
  mockFleetFetch([offlineHost('node-a-id')]);
  const stream = installMockEventSource();
  render(
    <SessionFixture status="authed">
      <CoreFleetProvider><Probe /></CoreFleetProvider>
    </SessionFixture>,
  );
  expect(await screen.findByText('node-a-id')).toBeInTheDocument();
  act(() => stream.fail());
  expect(screen.getByText('node-a-id')).toBeInTheDocument();
  expect(screen.getByText('degraded')).toBeInTheDocument();
});
```

Add companion tests that a `fleet` SSE event refetches REST and that session logout closes EventSource and clears user-scoped state.

- [ ] **Step 3: Run the focused web test and verify failure**

Run: `cd web && npm test -- CoreFleetProvider.test.tsx`

Expected: FAIL because the provider and test fixtures do not exist.

- [ ] **Step 4: Define strict response types in `coreFleet.ts`**

Define `FleetHost`, `FleetResponse`, `SnapshotValue`, and `CoreLiveStatus`. Implement `fetchFleet(signal?: AbortSignal): Promise<FleetResponse>` using `apiFetch`. Reject non-2xx responses with `FleetApiError(status, code)` and reject malformed top-level payloads when `generated_at`, `offline_after_seconds`, or `hosts` have the wrong primitive types.

Decode tagged snapshot messages into existing `SystemStatsPayload`, `ServiceInfo[]`, `DockerListPayload`, and `SwarmListPayload` only when the message `type` exactly matches the snapshot slot. Expose malformed snapshots as `undefined` and record a console error containing `agent_id`, slot, and observed timestamp.

Use this public boundary in `web/src/lib/coreFleet.ts`:

```ts
import { apiFetch } from './api';
import type {
  DockerListPayload,
  ServiceInfo,
  SwarmListPayload,
  SystemStatsPayload,
} from './types';

export type CoreLiveStatus = 'connecting' | 'live' | 'degraded';
export type ConnectionStatus = 'online' | 'offline';

export type SnapshotValue = { observed_at: number; value: unknown };
export type FleetHost = {
  agent_id: string;
  hostname: string;
  status: ConnectionStatus;
  protocol_version: number;
  capabilities: string[];
  metadata: Record<string, string>;
  first_seen_at: number;
  last_seen_at: number;
  disconnected_at: number | null;
  system: SnapshotValue | null;
  services: SnapshotValue | null;
  docker: SnapshotValue | null;
  swarm: SnapshotValue | null;
};
export type FleetResponse = {
  generated_at: number;
  offline_after_seconds: number;
  hosts: FleetHost[];
};
export type CoreAgentSnapshot = {
  agentId: string;
  hostname: string;
  status: ConnectionStatus;
  lastSeenAt: number;
  stats?: SystemStatsPayload;
  services?: ServiceInfo[];
  docker?: DockerListPayload;
  swarm?: SwarmListPayload;
};

export class FleetApiError extends Error {
  constructor(public readonly status: number, public readonly code: string) {
    super(`fleet request failed: ${status} ${code}`);
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function assertFleet(value: unknown): asserts value is FleetResponse {
  if (!isObject(value) || typeof value.generated_at !== 'number' ||
      typeof value.offline_after_seconds !== 'number' || !Array.isArray(value.hosts)) {
    throw new FleetApiError(502, 'invalid_fleet_payload');
  }
  for (const host of value.hosts) {
    if (!isObject(host) || typeof host.agent_id !== 'string' ||
        typeof host.hostname !== 'string' ||
        (host.status !== 'online' && host.status !== 'offline')) {
      throw new FleetApiError(502, 'invalid_host_payload');
    }
  }
}

export async function fetchFleet(signal?: AbortSignal): Promise<FleetResponse> {
  const response = await apiFetch('/api/core/v1/fleet', { signal });
  if (!response.ok) throw new FleetApiError(response.status, 'fleet_unavailable');
  const value: unknown = await response.json();
  assertFleet(value);
  return value;
}

function payload<T>(
  host: FleetHost,
  slot: keyof Pick<FleetHost, 'system' | 'services' | 'docker' | 'swarm'>,
  expected: string,
): T | undefined {
  const snapshot = host[slot];
  if (!snapshot || !isObject(snapshot.value) || snapshot.value.type !== expected ||
      !isObject(snapshot.value.payload)) {
    if (snapshot) console.error('[shellfleet] malformed durable snapshot', {
      agent_id: host.agent_id, slot, observed_at: snapshot.observed_at,
    });
    return undefined;
  }
  return snapshot.value.payload as T;
}

export function snapshotsByAgent(hosts: FleetHost[]): Record<string, CoreAgentSnapshot> {
  return Object.fromEntries(hosts.map((host) => [host.agent_id, {
    agentId: host.agent_id,
    hostname: host.hostname,
    status: host.status,
    lastSeenAt: host.last_seen_at,
    stats: payload<SystemStatsPayload>(host, 'system', 'SystemStatsResponse'),
    services: payload<{services: ServiceInfo[]}>(
      host, 'services', 'ListServicesResponse',
    )?.services,
    docker: payload<DockerListPayload>(host, 'docker', 'DockerListResponse'),
    swarm: payload<SwarmListPayload>(host, 'swarm', 'SwarmListResponse'),
  }]));
}
```

- [ ] **Step 5: Implement `CoreFleetProvider`**

On `status === 'authed'`, perform one abortable REST load and open `new EventSource('/api/core/v1/events', { withCredentials: true })`. On each `fleet` event, coalesce refetches so only one request is in flight and one trailing refresh is retained. `onopen` sets `live`; `onerror` sets `degraded` without changing `hosts` or `snapshots`. Native EventSource reconnect behavior remains enabled.

On `guest` or `pending_mfa`, close EventSource, abort fetch, clear hosts, and set `connecting`. `refresh()` invokes the same coalesced loader.

The context value must have this exact shape:

```tsx
type CoreFleetContextValue = {
  hosts: FleetHost[];
  snapshots: Record<string, CoreAgentSnapshot>;
  liveStatus: CoreLiveStatus;
  loading: boolean;
  error: string | null;
  refresh: () => void;
};
```

Use refs named `inFlightRef`, `trailingRefreshRef`, `abortRef`, and `eventSourceRef`. The loader's `finally` block must clear `inFlightRef` and immediately call the loader once when `trailingRefreshRef` was set. The effect cleanup must abort `abortRef.current`, close `eventSourceRef.current`, and null both references. Memoize `snapshotsByAgent(hosts)` and the context value so consumers do not rerender on unrelated session state.

- [ ] **Step 6: Run provider tests, typecheck, and lint**

Run: `cd web && npm test -- CoreFleetProvider.test.tsx && npm run typecheck && npm run lint`

Expected: PASS.

- [ ] **Step 7: Commit the web data boundary**

```bash
git -C web add package.json package-lock.json src/lib/coreFleet.ts src/components/providers/CoreFleetProvider.tsx src/components/providers/__tests__/CoreFleetProvider.test.tsx
git -C web commit -S -m "feat: load durable fleet state"
```

---

### Task 6: Migrate Fleet overview away from browser WebSocket truth

**Files:**
- Modify: `web/src/app/layout.tsx`
- Modify: `web/src/components/FleetOverview.tsx`
- Modify: `web/src/app/overview/page.tsx`
- Create: `web/src/components/__tests__/FleetOverview.test.tsx`

**Interfaces:**
- Consumes: `useCoreFleet()` from Task 5.
- Produces: a Fleet overview that renders online and offline hosts after refresh and during SSE failure without using `useWebSocket` or `useFleetSnapshots`.

- [ ] **Step 1: Write Fleet rendering tests**

Create fixtures with one online Docker Swarm manager and one offline worker whose system/service/container snapshots are still present. Assert:

```tsx
expect(screen.getByText('2 agents · 1 online')).toBeInTheDocument();
expect(screen.getByText('swarm-master')).toBeInTheDocument();
expect(screen.getByText('swarm-worker-1')).toBeInTheDocument();
expect(screen.getByText('offline')).toBeInTheDocument();
expect(screen.getByText(/live updates disconnected/i)).toBeInTheDocument();
```

- [ ] **Step 2: Run Fleet tests and verify failure**

Run: `cd web && npm test -- FleetOverview.test.tsx`

Expected: FAIL because Fleet still consumes WebSocket-owned arrays and cannot render an offline host.

- [ ] **Step 3: Mount the provider and migrate FleetOverview**

Mount `CoreFleetProvider` inside `SessionProvider` and outside the legacy `WebSocketProvider`. Keep `WebSocketProvider` and `FleetSnapshotsProvider` only for not-yet-migrated Host tools.

Replace FleetOverview's WebSocket imports with `useCoreFleet`. Derive totals from decoded durable snapshots. Use `hosts.length` for inventory count and `hosts.filter(host => host.status === 'online').length` for online count. Host rows receive explicit status, `last_seen_at`, and snapshot timestamps. Offline rows keep their last values but render muted and show `last seen <relative time>`.

When `liveStatus === 'degraded'`, render one non-blocking banner:

```text
Live updates disconnected. Showing durable state from the server; refresh remains available.
```

When REST itself fails and no prior fleet exists, render the error with a Retry button. When a refresh fails after prior data exists, preserve data and show the failure in the banner.

- [ ] **Step 4: Migrate the standalone overview status pill**

In `web/src/app/overview/page.tsx`, use `liveStatus` from `useCoreFleet`. Label `live` as `LIVE`, `connecting` as `SYNCING`, and `degraded` as `STALE`. Do not label the whole fleet `OFFLINE` because an SSE transport failed.

- [ ] **Step 5: Prove Fleet has no WebSocket dependency**

Run:

```bash
rg -n 'useWebSocket|useFleetSnapshots|WebSocketProvider' web/src/components/FleetOverview.tsx web/src/app/overview/page.tsx
```

Expected: no matches.

- [ ] **Step 6: Run the complete web gate**

Run: `cd web && npm run lint && npm run typecheck && npm test && npm run build`

Expected: PASS.

- [ ] **Step 7: Commit the Fleet migration**

```bash
git -C web add src/app/layout.tsx src/app/overview/page.tsx src/components/FleetOverview.tsx src/components/__tests__/FleetOverview.test.tsx
git -C web commit -S -m "feat: render fleet from durable state"
```

---

### Task 7: Containerized browser-plus-agent journey

**Files:**
- Create: `tests/journey/docker-compose.yml`
- Create: `tests/journey/nginx.conf`
- Create: `tests/journey/agent-token.txt`
- Create: `tests/journey/playwright.config.ts`
- Create: `tests/journey/fleet-read-plane.spec.ts`
- Modify: `web/package.json`
- Modify: `web/package-lock.json`

**Interfaces:**
- Consumes: released-shape server, web, and agent images; single origin `http://127.0.0.1:18080`.
- Produces: `npm run test:journey` and deterministic Docker Compose lifecycle.

- [ ] **Step 1: Add Playwright**

Run from `web/`:

```bash
npm install --save-dev @playwright/test
```

Add script:

```json
"test:journey": "playwright test -c ../tests/journey/playwright.config.ts"
```

- [ ] **Step 2: Create the one-origin journey topology**

The Compose file builds `server`, `web`, and `agent` using the root Dockerfiles. Configure server with `JWT_SECRET=dev`, `SHELLFLEET_DEV=true`, `COOKIE_SECURE=false`, `AGENT_SECRET=journey-agent-token`, `DB_PATH=/data/shellfleet.db`, and syntactically valid non-secret fixture values for startup-validated OAuth/proxy environment fields. Mount `agent-token.txt` read-only at `/var/lib/shellfleet-agent/agent-token.txt` and set agent `SERVER_WS_URL=ws://server:8080/agent/ws` plus hostname `journey-agent`.

Nginx listens on 80; routes `/api/`, `/auth/`, `/ui/ws`, and `/agent/ws` to server, and all other paths to web. Preserve SSE with `proxy_buffering off`, `proxy_cache off`, and `proxy_read_timeout 1h` on `/api/core/v1/events`.

Add health checks and `depends_on: condition: service_healthy` so the test starts only after ingress, server, and web respond.

- [ ] **Step 3: Write the journey test**

The Playwright test must:

1. Open `/overview` and wait for `journey-agent` to appear as online.
2. Reload the page and prove the same host and non-zero system data return from REST.
3. Stop the agent with `docker compose -f tests/journey/docker-compose.yml stop agent`.
4. Wait at most 50 seconds for the row to become offline while its hostname and last system snapshot remain visible.
5. Block `/api/core/v1/events` in the browser context, reload, and prove Fleet still renders from REST with the stale banner.
6. Unblock SSE, start the agent, and wait at most 15 seconds for online state.
7. Query `/api/core/v1/fleet` directly through Playwright request context and assert exactly one durable host row.

Use `test.afterAll` to run `docker compose down -v --remove-orphans` even after assertion failure.

- [ ] **Step 4: Run the journey locally**

Run:

```bash
docker compose -f tests/journey/docker-compose.yml up -d --build
cd web && npx playwright install chromium && npm run test:journey
```

Expected: PASS; the test observes online → offline-with-data → online.

- [ ] **Step 5: Commit the journey harness**

Commit web dependency changes in the web submodule:

```bash
git -C web add package.json package-lock.json
git -C web commit -S -m "test: add browser journey tooling"
```

Commit root-owned journey files and updated submodule pointers:

```bash
git add tests/journey server web
git commit -S -m "test: gate durable fleet journey"
```

---

### Task 8: Make the journey a required CI gate

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `web` dependency lock, journey Compose topology, Playwright test.
- Produces: required `journey (browser + server + agent)` job with uploaded diagnostics on failure.

- [ ] **Step 1: Add the CI job**

Add a job with `needs: [rust, web]`, checkout with recursive submodules, Node 22/npm cache, `npm ci` in `web`, `npx playwright install --with-deps chromium`, Compose build/up, and `npm run test:journey`.

Always upload:

- `web/test-results/`
- `web/playwright-report/`
- `tests/journey/logs/`

Before artifact upload, always run:

```bash
mkdir -p tests/journey/logs
docker compose -f tests/journey/docker-compose.yml ps > tests/journey/logs/compose-ps.txt
docker compose -f tests/journey/docker-compose.yml logs --no-color > tests/journey/logs/compose.log
docker compose -f tests/journey/docker-compose.yml down -v --remove-orphans
```

- [ ] **Step 2: Validate the workflow and repeat all local gates**

Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd web && npm run lint && npm run typecheck && npm test && npm run build
docker compose -f ../tests/journey/docker-compose.yml up -d --build
npm run test:journey
```

Expected: every command PASS.

- [ ] **Step 3: Commit the CI gate**

```bash
git add .github/workflows/ci.yml server web
git commit -S -m "ci: require durable fleet journey"
```

---

### Task 9: Slice acceptance and architecture audit

**Files:**
- Modify: `docs/superpowers/specs/2026-07-15-shellfleet-product-reset-design.md`
- Modify: this plan's checkboxes as tasks complete.

**Interfaces:**
- Consumes: all slice commits and verification output.
- Produces: evidence that the original blank-dashboard failure mode is structurally removed from Fleet.

- [ ] **Step 1: Verify requirement-to-evidence mapping**

Record exact command output or CI run links for:

- server collects with zero connected UI clients;
- SQLite retains the host and snapshots after agent disconnect and server restart;
- `/api/core/v1/fleet` returns online and offline inventory;
- blocking SSE does not remove Fleet data;
- Fleet source has no `useWebSocket` or `useFleetSnapshots` dependency;
- offline appears within 45 seconds from the last authenticated agent message;
- reconnect appears within 10 seconds after agent registration;
- current Rust, web, image-build, and journey gates pass.

- [ ] **Step 2: Run the live-shape restart check**

With the journey stack running and the host visible, restart only the server:

```bash
docker compose -f tests/journey/docker-compose.yml restart server
```

Reload `/overview` before the agent reconnects. Expected: `journey-agent` remains listed with its last durable snapshot and transitions back online after reconnect.

- [ ] **Step 3: Mark only proven design clauses complete**

In the product-reset design's migration section, add a dated evidence note under Stage B stating that the durable Fleet read plane and journey gate landed. Do not mark Host read surfaces, enrollment, operations, packaging, monorepo consolidation, or legacy deletion complete.

- [ ] **Step 4: Commit acceptance evidence**

```bash
git add docs/superpowers/specs/2026-07-15-shellfleet-product-reset-design.md docs/superpowers/plans/2026-07-15-shellfleet-durable-fleet-read-plane.md server web
git commit -S -m "docs: verify durable fleet slice"
```

## Slice definition of done

This slice is complete only when all nine tasks are checked, every listed command passes, the containerized journey passes from an empty volume, and the Fleet overview remains useful under both agent disconnect and SSE failure. Green unit tests without the browser-plus-agent journey do not satisfy this plan.
