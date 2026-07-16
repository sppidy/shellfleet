use serde::Deserialize;
use shared::{
    DockerContainer, ServiceInfo, SwarmRole, SwarmService,
    fleet::{FleetHost, SnapshotValue},
};

#[derive(Clone, Debug, Deserialize)]
pub struct SystemSnapshot {
    pub hostname: String,
    pub load_1: f32,
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    pub root_disk_total_kb: u64,
    pub root_disk_used_kb: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DockerSnapshot {
    pub available: bool,
    pub swarm_role: SwarmRole,
    pub containers: Vec<DockerContainer>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SwarmSnapshot {
    pub is_manager: bool,
    pub services: Vec<SwarmService>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(rename = "type")]
    kind: String,
    payload: T,
}

#[derive(Deserialize)]
struct ServicesPayload {
    services: Vec<ServiceInfo>,
}

fn decode<T: for<'de> Deserialize<'de>>(
    snapshot: Option<&SnapshotValue>,
    expected: &str,
) -> Option<T> {
    let envelope: Envelope<T> = serde_json::from_value(snapshot?.value.clone()).ok()?;
    (envelope.kind == expected).then_some(envelope.payload)
}

pub fn system(host: &FleetHost) -> Option<SystemSnapshot> {
    decode(host.system.as_ref(), "SystemStatsResponse")
}

pub fn services(host: &FleetHost) -> Option<Vec<ServiceInfo>> {
    decode::<ServicesPayload>(host.services.as_ref(), "ListServicesResponse")
        .map(|payload| payload.services)
}

pub fn docker(host: &FleetHost) -> Option<DockerSnapshot> {
    decode(host.docker.as_ref(), "DockerListResponse")
}

pub fn swarm(host: &FleetHost) -> Option<SwarmSnapshot> {
    decode(host.swarm.as_ref(), "SwarmListResponse")
}

#[cfg(test)]
mod tests {
    use super::{docker, services, swarm, system};
    use shared::fleet::{ConnectionStatus, FleetHost, SnapshotValue};
    use std::collections::BTreeMap;

    fn host() -> FleetHost {
        FleetHost {
            agent_id: "agent-a".into(),
            hostname: "worker-a".into(),
            status: ConnectionStatus::Online,
            protocol_version: 19,
            capabilities: vec!["systemd".into(), "docker".into(), "swarm".into()],
            metadata: BTreeMap::new(),
            first_seen_at: 1,
            last_seen_at: 9,
            disconnected_at: None,
            system: Some(SnapshotValue {
                observed_at: 9,
                value: serde_json::json!({
                    "type": "SystemStatsResponse",
                    "payload": {
                        "hostname": "worker-a", "kernel": "6.8", "uptime_secs": 90,
                        "cpu_count": 4, "load_1": 0.5, "load_5": 0.4, "load_15": 0.3,
                        "mem_total_kb": 1000, "mem_available_kb": 400,
                        "swap_total_kb": 0, "swap_free_kb": 0,
                        "root_disk_total_kb": 2000, "root_disk_used_kb": 500
                    }
                }),
            }),
            services: Some(SnapshotValue {
                observed_at: 9,
                value: serde_json::json!({
                    "type": "ListServicesResponse",
                    "payload": {"services": [{
                        "name": "docker.service", "description": "Docker",
                        "status": "running", "active_state": "active"
                    }]}
                }),
            }),
            docker: Some(SnapshotValue {
                observed_at: 9,
                value: serde_json::json!({
                    "type": "DockerListResponse",
                    "payload": {"available": true, "swarm_role": "worker", "containers": [], "error": null}
                }),
            }),
            swarm: Some(SnapshotValue {
                observed_at: 9,
                value: serde_json::json!({
                    "type": "SwarmListResponse",
                    "payload": {"available": true, "is_manager": false, "services": [], "nodes": [], "error": null}
                }),
            }),
        }
    }

    #[test]
    fn valid_durable_snapshots_decode_to_typed_values() {
        let host = host();
        assert_eq!(system(&host).unwrap().hostname, "worker-a");
        assert_eq!(services(&host).unwrap()[0].name, "docker.service");
        assert!(docker(&host).unwrap().available);
        assert!(!swarm(&host).unwrap().is_manager);
    }

    #[test]
    fn malformed_or_mistagged_snapshots_fail_soft() {
        let mut host = host();
        host.system.as_mut().unwrap().value = serde_json::json!({
            "type": "DockerListResponse",
            "payload": {"hostname": "wrong"}
        });
        host.services.as_mut().unwrap().value = serde_json::json!({"bad": true});
        assert!(system(&host).is_none());
        assert!(services(&host).is_none());
    }
}
