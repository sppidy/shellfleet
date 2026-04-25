use serde::{Deserialize, Serialize};

/// Protocol version sent by the agent in the Register handshake. Bump when
/// the wire format changes in a way the server needs to reject older agents
/// for. Value `0` means "legacy agent that predates this field" — those
/// still connect, just without the version-aware fast paths.
pub const PROTOCOL_VERSION: u32 = 3;

fn default_protocol_version() -> u32 {
    0
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SwarmRole {
    NotInSwarm,
    Worker,
    Manager,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DockerContainer {
    pub id: String,
    pub names: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwarmService {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub replicas: String,
    pub image: String,
    pub ports: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwarmNode {
    pub id: String,
    pub hostname: String,
    pub status: String,
    pub availability: String,
    pub manager_status: String,
    pub engine_version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    /// Agent registering with the server
    Register {
        hostname: String,
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
    },

    /// Server acknowledging registration
    RegisterAck { agent_id: String },

    /// Ping / Pong for heartbeat (application-level; the WebSocket Ping/Pong
    /// frames are also used by the server to keep proxies from idling out).
    Ping,
    Pong,

    /// Request to list systemd services
    ListServicesRequest,

    /// Response containing systemd services
    ListServicesResponse { services: Vec<ServiceInfo> },

    /// Request to control a service (start, stop, restart)
    ControlServiceRequest { name: String, action: String },

    /// Response to control a service
    ControlServiceResponse {
        name: String,
        success: bool,
        error: Option<String>,
    },

    /// Request to start a terminal session
    StartTerminalRequest,

    /// Terminal data
    TerminalData { data: Vec<u8> },

    /// Request to resize terminal
    TerminalResize { cols: u16, rows: u16 },

    /// Request to read a configuration file
    ReadConfigRequest { path: String },

    /// Response containing file content
    ReadConfigResponse {
        path: String,
        content: String,
        error: Option<String>,
    },

    /// Request to write a configuration file
    WriteConfigRequest { path: String, content: String },

    /// Response to write config
    WriteConfigResponse {
        path: String,
        success: bool,
        error: Option<String>,
    },

    /// Request a snapshot of system stats (uptime, load, memory, disk, …).
    /// Introduced in protocol_version 2; older agents simply ignore it
    /// because they don't recognise the variant when deserialising.
    SystemStatsRequest,

    /// Snapshot of system-wide resource usage. All sizes in kilobytes
    /// (KiB, 1024 bytes) to match /proc/meminfo and `df -P`.
    SystemStatsResponse {
        hostname: String,
        kernel: String,
        uptime_secs: u64,
        cpu_count: u32,
        load_1: f32,
        load_5: f32,
        load_15: f32,
        mem_total_kb: u64,
        mem_available_kb: u64,
        swap_total_kb: u64,
        swap_free_kb: u64,
        root_disk_total_kb: u64,
        root_disk_used_kb: u64,
    },

    /// Request a list of Docker containers + the agent's swarm role.
    /// Introduced in protocol_version 3.
    DockerListRequest,

    /// Container list (running + stopped) for the agent's local engine.
    /// `available = false` when the agent can't reach `docker`.
    DockerListResponse {
        available: bool,
        swarm_role: SwarmRole,
        containers: Vec<DockerContainer>,
        error: Option<String>,
    },

    /// Request swarm-wide info. Only meaningful on a manager node.
    /// Introduced in protocol_version 3.
    SwarmListRequest,

    /// Swarm-wide services + node list. Empty (with `available=false` /
    /// `is_manager=false`) if the agent isn't a manager.
    SwarmListResponse {
        available: bool,
        is_manager: bool,
        services: Vec<SwarmService>,
        nodes: Vec<SwarmNode>,
        error: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub description: String,
    /// SUB state from systemctl: running, exited, failed, dead, …
    pub status: String,
    /// ACTIVE state from systemctl: active, inactive, failed, activating, …
    pub active_state: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum UiMessage {
    /// UI asking for online agents
    ListAgentsRequest,

    /// Server telling UI about online agents
    ListAgentsResponse { agents: Vec<String> },

    /// UI sending a message to a specific agent
    SendToAgent { agent_id: String, message: Message },

    /// Server forwarding a message from an agent to the UI
    AgentMessage { agent_id: String, message: Message },
}
