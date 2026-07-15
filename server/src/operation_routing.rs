use shared::Message;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OperationKey {
    Terminal(String),
    DockerExec,
    K8sLogs(String),
    DockerLogs(String),
    JournalLogs(String),
    JournalStream(String),
    Trusted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiOperation {
    Start(OperationKey),
    Use(OperationKey),
    Stop(OperationKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOperation {
    pub key: OperationKey,
    pub ends: bool,
}

#[derive(Debug, Default)]
pub struct OperationOwners {
    owners: HashMap<(String, OperationKey), u64>,
}

impl OperationOwners {
    pub fn claim(&mut self, agent_id: &str, key: OperationKey, client_id: u64) -> bool {
        let map_key = (agent_id.to_string(), key);
        match self.owners.get(&map_key) {
            Some(owner) => *owner == client_id,
            None => {
                self.owners.insert(map_key, client_id);
                true
            }
        }
    }

    pub fn owner(&self, agent_id: &str, key: &OperationKey) -> Option<u64> {
        self.owners
            .get(&(agent_id.to_string(), key.clone()))
            .copied()
    }

    pub fn release(&mut self, agent_id: &str, key: &OperationKey, client_id: u64) -> bool {
        let map_key = (agent_id.to_string(), key.clone());
        if self.owners.get(&map_key) != Some(&client_id) {
            return false;
        }
        self.owners.remove(&map_key);
        true
    }

    pub fn release_agent_operation(&mut self, agent_id: &str, key: &OperationKey) {
        self.owners.remove(&(agent_id.to_string(), key.clone()));
    }

    pub fn release_client(&mut self, client_id: u64) {
        self.owners.retain(|_, owner| *owner != client_id);
    }

    pub fn release_agent(&mut self, agent_id: &str) {
        self.owners.retain(|(agent, _), _| agent != agent_id);
    }
}

pub fn ui_operation(message: &Message) -> Option<UiOperation> {
    use Message::*;
    let operation = match message {
        StartTerminalRequest { session_id } | K8sExecRequest { session_id, .. } => {
            UiOperation::Start(OperationKey::Terminal(session_id.clone()))
        }
        DockerExecStartRequest { .. } => UiOperation::Start(OperationKey::DockerExec),
        K8sLogsRequest { stream_id, .. } => {
            UiOperation::Start(OperationKey::K8sLogs(stream_id.clone()))
        }
        DockerLogsRequest { container_id, .. } => {
            UiOperation::Start(OperationKey::DockerLogs(container_id.clone()))
        }
        JournalLogsRequest { unit, .. } => {
            UiOperation::Start(OperationKey::JournalLogs(unit.clone()))
        }
        JournalStreamRequest { stream_id, .. } => {
            UiOperation::Start(OperationKey::JournalStream(stream_id.clone()))
        }
        TerminalData { session_id, .. } | TerminalResize { session_id, .. } => {
            UiOperation::Use(terminal_key(session_id))
        }
        StopTerminalRequest { session_id } => UiOperation::Stop(terminal_key(session_id)),
        DockerExecStopRequest => UiOperation::Stop(OperationKey::DockerExec),
        K8sLogsStop { stream_id } => UiOperation::Stop(OperationKey::K8sLogs(stream_id.clone())),
        DockerLogsStop { container_id } => {
            UiOperation::Stop(OperationKey::DockerLogs(container_id.clone()))
        }
        JournalLogsStop { unit } => UiOperation::Stop(OperationKey::JournalLogs(unit.clone())),
        JournalStreamStop { stream_id } => {
            UiOperation::Stop(OperationKey::JournalStream(stream_id.clone()))
        }
        TrustedOperationClient {
            request_id,
            start,
            close,
            ..
        } => {
            let key = OperationKey::Trusted(request_id.clone());
            if *start {
                UiOperation::Start(key)
            } else if *close {
                UiOperation::Stop(key)
            } else {
                UiOperation::Use(key)
            }
        }
        _ => return None,
    };
    Some(operation)
}

pub fn agent_operation(message: &Message) -> Option<AgentOperation> {
    use Message::*;
    let (key, ends) = match message {
        TerminalData { session_id, .. } => (terminal_key(session_id), false),
        K8sExecResponse {
            session_id,
            success,
            ..
        } => (OperationKey::Terminal(session_id.clone()), !success),
        DockerExecStartResponse { success, .. } => (OperationKey::DockerExec, !success),
        K8sLogsChunk { stream_id, .. } => (OperationKey::K8sLogs(stream_id.clone()), false),
        K8sLogsEnd { stream_id, .. } => (OperationKey::K8sLogs(stream_id.clone()), true),
        DockerLogsChunk { container_id, .. } => {
            (OperationKey::DockerLogs(container_id.clone()), false)
        }
        DockerLogsEnd { container_id, .. } => {
            (OperationKey::DockerLogs(container_id.clone()), true)
        }
        JournalLogsChunk { unit, .. } => (OperationKey::JournalLogs(unit.clone()), false),
        JournalLogsEnd { unit, .. } => (OperationKey::JournalLogs(unit.clone()), true),
        JournalStreamChunk { stream_id, .. } => {
            (OperationKey::JournalStream(stream_id.clone()), false)
        }
        JournalStreamEnd { stream_id, .. } => {
            (OperationKey::JournalStream(stream_id.clone()), true)
        }
        TrustedOperationHost {
            request_id,
            complete,
            ..
        } => (OperationKey::Trusted(request_id.clone()), *complete),
        _ => return None,
    };
    Some(AgentOperation { key, ends })
}

fn terminal_key(session_id: &str) -> OperationKey {
    if session_id.is_empty() {
        OperationKey::DockerExec
    } else {
        OperationKey::Terminal(session_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentOperation, OperationKey, OperationOwners, UiOperation, agent_operation, ui_operation,
    };
    use shared::Message;

    #[test]
    fn terminal_and_kubernetes_exec_share_session_ownership() {
        let host = Message::StartTerminalRequest {
            session_id: "host-session".into(),
        };
        assert_eq!(
            ui_operation(&host),
            Some(UiOperation::Start(OperationKey::Terminal(
                "host-session".into()
            )))
        );

        let k8s = Message::K8sExecRequest {
            session_id: "k8s-session".into(),
            namespace: "default".into(),
            pod_name: "api".into(),
            container: None,
            command: vec![],
        };
        assert_eq!(
            ui_operation(&k8s),
            Some(UiOperation::Start(OperationKey::Terminal(
                "k8s-session".into()
            )))
        );
    }

    #[test]
    fn terminal_continuations_and_stop_require_the_same_owner() {
        let data = Message::TerminalData {
            session_id: "session".into(),
            data: b"id\n".to_vec(),
        };
        assert_eq!(
            ui_operation(&data),
            Some(UiOperation::Use(OperationKey::Terminal("session".into())))
        );
        let stop = Message::StopTerminalRequest {
            session_id: "session".into(),
        };
        assert_eq!(
            ui_operation(&stop),
            Some(UiOperation::Stop(OperationKey::Terminal("session".into())))
        );
    }

    #[test]
    fn docker_exec_uses_an_agent_scoped_singleton_key() {
        let start = Message::DockerExecStartRequest {
            container_id: "container".into(),
            shell: "sh".into(),
        };
        assert_eq!(
            ui_operation(&start),
            Some(UiOperation::Start(OperationKey::DockerExec))
        );
        let output = Message::TerminalData {
            session_id: String::new(),
            data: vec![1],
        };
        assert_eq!(
            agent_operation(&output),
            Some(AgentOperation {
                key: OperationKey::DockerExec,
                ends: false,
            })
        );
    }

    #[test]
    fn log_chunks_route_to_requester_and_end_releases_ownership() {
        let request = Message::K8sLogsRequest {
            stream_id: "logs".into(),
            namespace: "default".into(),
            pod_name: "api".into(),
            container: None,
            tail_lines: 100,
            follow: true,
        };
        assert_eq!(
            ui_operation(&request),
            Some(UiOperation::Start(OperationKey::K8sLogs("logs".into())))
        );
        let end = Message::K8sLogsEnd {
            stream_id: "logs".into(),
            error: None,
        };
        assert_eq!(
            agent_operation(&end),
            Some(AgentOperation {
                key: OperationKey::K8sLogs("logs".into()),
                ends: true,
            })
        );
    }

    #[test]
    fn an_operation_has_exactly_one_client_owner() {
        let mut owners = OperationOwners::default();
        let key = OperationKey::Terminal("session".into());

        assert!(owners.claim("agent-a", key.clone(), 10));
        assert!(owners.claim("agent-a", key.clone(), 10));
        assert!(!owners.claim("agent-a", key.clone(), 11));
        assert_eq!(owners.owner("agent-a", &key), Some(10));
        assert!(!owners.release("agent-a", &key, 11));
        assert!(owners.release("agent-a", &key, 10));
        assert_eq!(owners.owner("agent-a", &key), None);
    }

    #[test]
    fn disconnect_releases_only_that_clients_operations() {
        let mut owners = OperationOwners::default();
        owners.claim("agent-a", OperationKey::DockerExec, 10);
        owners.claim("agent-b", OperationKey::DockerExec, 11);

        owners.release_client(10);

        assert_eq!(owners.owner("agent-a", &OperationKey::DockerExec), None);
        assert_eq!(owners.owner("agent-b", &OperationKey::DockerExec), Some(11));
    }

    #[test]
    fn trusted_root_ciphertext_routes_only_to_request_owner() {
        let request = Message::TrustedOperationClient {
            request_id: "trusted-1".into(),
            start: true,
            close: false,
            payload: vec![1, 2, 3],
        };
        assert_eq!(
            ui_operation(&request),
            Some(UiOperation::Start(OperationKey::Trusted(
                "trusted-1".into()
            )))
        );
        let response = Message::TrustedOperationHost {
            request_id: "trusted-1".into(),
            complete: false,
            payload: vec![9, 8, 7],
        };
        assert_eq!(
            agent_operation(&response),
            Some(AgentOperation {
                key: OperationKey::Trusted("trusted-1".into()),
                ends: false,
            })
        );
        let mut owners = OperationOwners::default();
        let key = OperationKey::Trusted("trusted-1".into());
        assert!(owners.claim("host", key.clone(), 10));
        assert_eq!(owners.owner("host", &key), Some(10));
        assert!(!owners.claim("host", key, 11));
    }
}
