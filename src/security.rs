use crate::{DockerContainerAction, Message, SwarmAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequestClass {
    Read,
    PrivilegedRead,
    Interactive,
    Mutation,
}

impl UiRequestClass {
    pub const fn requires_admin(self) -> bool {
        matches!(
            self,
            Self::PrivilegedRead | Self::Interactive | Self::Mutation
        )
    }

    pub const fn requires_approval(self) -> bool {
        matches!(self, Self::Mutation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiRequestSecurity {
    pub action: Option<&'static str>,
    pub class: UiRequestClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequestError {
    NotUiRequest,
    InvalidAction,
}

const fn request(action: Option<&'static str>, class: UiRequestClass) -> UiRequestSecurity {
    UiRequestSecurity { action, class }
}

impl Message {
    /// Returns the security contract for every message the UI may send to an
    /// agent. The exhaustive match intentionally has no wildcard: protocol
    /// additions must be classified before the shared crate can compile.
    pub fn ui_request_security(&self) -> Result<UiRequestSecurity, UiRequestError> {
        use Message::*;
        use UiRequestClass::*;

        let security = match self {
            ListServicesRequest => request(Some("service:List"), Read),
            ControlServiceRequest { action, .. } => {
                let action = match action.as_str() {
                    "start" => "service:Start",
                    "stop" => "service:Stop",
                    "restart" => "service:Restart",
                    _ => return Err(UiRequestError::InvalidAction),
                };
                request(Some(action), Mutation)
            }
            StartTerminalRequest { .. }
            | TerminalData { .. }
            | StopTerminalRequest { .. }
            | TerminalResize { .. } => request(Some("agent:Terminal"), Interactive),

            K8sListPodsRequest
            | K8sListDeploymentsRequest
            | K8sListServicesRequest
            | K8sListIngressesRequest
            | K8sListPvcsRequest
            | K8sListEventsRequest => request(Some("k8s:List"), Read),
            K8sDescribeRequest { .. } => request(Some("k8s:Describe"), Read),
            K8sLogsRequest { .. } | K8sLogsStop { .. } => request(Some("k8s:Logs"), PrivilegedRead),
            K8sExecRequest { .. } => request(Some("k8s:Exec"), Interactive),
            K8sApplyRequest { .. } => request(Some("k8s:Apply"), Mutation),
            K8sScaleRequest { .. } => request(Some("k8s:Scale"), Mutation),
            K8sDeletePodRequest { .. } => request(Some("k8s:Delete"), Mutation),

            ReadConfigRequest { .. } => request(Some("config:Read"), PrivilegedRead),
            WriteConfigRequest { .. } => request(Some("config:Write"), Mutation),
            SystemStatsRequest => request(None, Read),
            DockerListRequest => request(Some("container:List"), Read),
            SwarmListRequest => request(Some("swarm:List"), Read),
            SwarmServiceActionRequest { action, .. } => {
                let action = match action {
                    SwarmAction::Scale(_) => "swarm:Scale",
                    SwarmAction::ForceUpdate => "swarm:Deploy",
                    SwarmAction::Remove => "swarm:Remove",
                };
                request(Some(action), Mutation)
            }
            AptStatusRequest => request(Some("apt:Status"), Read),
            AptRefreshRequest => request(Some("apt:Refresh"), Mutation),
            AptUpgradeRequest { .. } => request(Some("apt:Upgrade"), Mutation),
            DockerCreateContainerRequest { .. } => request(Some("container:Create"), Mutation),
            SwarmCreateServiceRequest { .. } => request(Some("swarm:Deploy"), Mutation),
            DockerContainerActionRequest { action, .. } => {
                let action = match action {
                    DockerContainerAction::Start => "container:Start",
                    DockerContainerAction::Stop => "container:Stop",
                    DockerContainerAction::Restart => "container:Restart",
                    DockerContainerAction::Remove => "container:Remove",
                };
                request(Some(action), Mutation)
            }
            DockerLogsRequest { .. } | DockerLogsStop { .. } => {
                request(Some("container:Logs"), Read)
            }
            JournalLogsRequest { .. }
            | JournalLogsStop { .. }
            | JournalStreamRequest { .. }
            | JournalStreamStop { .. } => request(Some("journal:Read"), Read),
            SwarmServiceInspectRequest { .. } => request(Some("swarm:List"), Read),
            SwarmStackDeployRequest { .. } => request(Some("swarm:Deploy"), Mutation),
            BackupRunRequest { .. } => request(Some("backup:Run"), Mutation),
            BackupListArchivesRequest { .. } => request(Some("backup:List"), Read),
            BackupRestoreRequest { .. } => request(Some("backup:Restore"), Mutation),
            DockerImageListRequest => request(Some("docker:ImageList"), Read),
            DockerImageRemoveRequest { .. } => request(Some("docker:ImageRemove"), Mutation),
            DockerImagePullRequest { .. } => request(Some("docker:ImagePull"), Mutation),
            DockerNetworkListRequest | DockerNetworkInspectRequest { .. } => {
                request(Some("network:List"), Read)
            }
            DockerNetworkCreateRequest { .. } => request(Some("network:Create"), Mutation),
            DockerNetworkRemoveRequest { .. } => request(Some("network:Remove"), Mutation),
            DockerVolumeListRequest | DockerVolumeInspectRequest { .. } => {
                request(Some("volume:List"), Read)
            }
            DockerVolumeRemoveRequest { .. } => request(Some("volume:Remove"), Mutation),
            DockerVolumePruneRequest => request(Some("docker:Prune"), Mutation),
            SwarmStackListRequest | SwarmStackInspectRequest { .. } => {
                request(Some("swarm:List"), Read)
            }
            SwarmStackRemoveRequest { .. } => request(Some("swarm:Remove"), Mutation),
            DockerSystemPruneRequest { .. } => request(Some("docker:Prune"), Mutation),
            DockerStatsRequest => request(Some("container:List"), Read),
            DockerExecStartRequest { .. } | DockerExecStopRequest => {
                request(Some("container:Exec"), Interactive)
            }
            DriftSnapshotRequest { .. } => request(Some("drift:Snapshot"), Mutation),

            Register { .. }
            | RegisterAck { .. }
            | CapabilitiesUpdate { .. }
            | Ping
            | Pong
            | ListServicesResponse { .. }
            | ControlServiceResponse { .. }
            | K8sListPodsResponse { .. }
            | K8sListDeploymentsResponse { .. }
            | K8sListServicesResponse { .. }
            | K8sListIngressesResponse { .. }
            | K8sListPvcsResponse { .. }
            | K8sListEventsResponse { .. }
            | K8sDescribeResponse { .. }
            | K8sLogsChunk { .. }
            | K8sLogsEnd { .. }
            | K8sExecResponse { .. }
            | RunCommandRequest { .. }
            | RunCommandResponse { .. }
            | K8sApplyResponse { .. }
            | K8sScaleResponse { .. }
            | K8sDeletePodResponse { .. }
            | ReadConfigResponse { .. }
            | WriteConfigResponse { .. }
            | SystemStatsResponse { .. }
            | DockerListResponse { .. }
            | SwarmListResponse { .. }
            | SwarmServiceActionResponse { .. }
            | AptStatusResponse { .. }
            | AptRefreshResponse { .. }
            | AptUpgradeResponse { .. }
            | DockerCreateContainerResponse { .. }
            | SwarmCreateServiceResponse { .. }
            | DockerContainerActionResponse { .. }
            | DockerLogsChunk { .. }
            | DockerLogsEnd { .. }
            | JournalLogsChunk { .. }
            | JournalLogsEnd { .. }
            | JournalStreamChunk { .. }
            | JournalStreamEnd { .. }
            | SwarmServiceInspectResponse { .. }
            | SwarmStackDeployResponse { .. }
            | HealthProbeSyncRequest { .. }
            | HealthProbeReport { .. }
            | BackupRunResponse { .. }
            | BackupListArchivesResponse { .. }
            | BackupRestoreResponse { .. }
            | DockerImageListResponse { .. }
            | DockerImageRemoveResponse { .. }
            | DockerImagePullResponse { .. }
            | DockerNetworkListResponse { .. }
            | DockerNetworkInspectResponse { .. }
            | DockerNetworkCreateResponse { .. }
            | DockerNetworkRemoveResponse { .. }
            | DockerVolumeListResponse { .. }
            | DockerVolumeInspectResponse { .. }
            | DockerVolumeRemoveResponse { .. }
            | DockerVolumePruneResponse { .. }
            | SwarmStackListResponse { .. }
            | SwarmStackInspectResponse { .. }
            | SwarmStackRemoveResponse { .. }
            | DockerSystemPruneResponse { .. }
            | DockerStatsResponse { .. }
            | DockerExecStartResponse { .. }
            | DriftSnapshotResponse { .. } => return Err(UiRequestError::NotUiRequest),
        };
        Ok(security)
    }
}

#[cfg(test)]
mod tests {
    use crate::{DockerContainerAction, Message, SwarmAction, UiRequestClass, UiRequestError};

    #[test]
    fn service_actions_have_distinct_policy_actions() {
        for (wire_action, policy_action) in [
            ("start", "service:Start"),
            ("stop", "service:Stop"),
            ("restart", "service:Restart"),
        ] {
            let security = Message::ControlServiceRequest {
                name: "nginx".into(),
                action: wire_action.into(),
            }
            .ui_request_security()
            .unwrap();
            assert_eq!(security.action, Some(policy_action));
            assert_eq!(security.class, UiRequestClass::Mutation);
        }
    }

    #[test]
    fn container_and_swarm_actions_preserve_destructive_semantics() {
        let remove = Message::DockerContainerActionRequest {
            id: "container".into(),
            action: DockerContainerAction::Remove,
        }
        .ui_request_security()
        .unwrap();
        assert_eq!(remove.action, Some("container:Remove"));

        let swarm_remove = Message::SwarmServiceActionRequest {
            name: "service".into(),
            action: SwarmAction::Remove,
        }
        .ui_request_security()
        .unwrap();
        assert_eq!(swarm_remove.action, Some("swarm:Remove"));

        let stack_remove = Message::SwarmStackRemoveRequest {
            name: "stack".into(),
        }
        .ui_request_security()
        .unwrap();
        assert_eq!(stack_remove.action, Some("swarm:Remove"));

        let volume_prune = Message::DockerVolumePruneRequest
            .ui_request_security()
            .unwrap();
        assert_eq!(volume_prune.action, Some("docker:Prune"));
    }

    #[test]
    fn invalid_service_action_and_agent_response_fail_closed() {
        let invalid = Message::ControlServiceRequest {
            name: "nginx".into(),
            action: "reload".into(),
        };
        assert_eq!(
            invalid.ui_request_security(),
            Err(UiRequestError::InvalidAction)
        );

        let response = Message::ControlServiceResponse {
            name: "nginx".into(),
            success: true,
            error: None,
        };
        assert_eq!(
            response.ui_request_security(),
            Err(UiRequestError::NotUiRequest)
        );
    }

    #[test]
    fn terminal_stop_remains_privileged_and_is_never_approval_held() {
        let security = Message::StopTerminalRequest {
            session_id: "session".into(),
        }
        .ui_request_security()
        .unwrap();
        assert_eq!(security.class, UiRequestClass::Interactive);
        assert!(security.class.requires_admin());
        assert!(!security.class.requires_approval());
    }
}
