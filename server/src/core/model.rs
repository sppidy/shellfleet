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
