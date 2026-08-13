pub use shared::fleet::{
    ConnectionStatus, CoreEvent, CoreEventKind, FleetHost, FleetResponse, SnapshotValue,
};

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
