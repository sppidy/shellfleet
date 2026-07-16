use serde::{Deserialize, Serialize};

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
    fn connection_and_event_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&ConnectionStatus::Offline).unwrap(),
            "\"offline\""
        );
        assert_eq!(
            serde_json::to_string(&CoreEventKind::HostUpdated).unwrap(),
            "\"host_updated\""
        );
    }
}
