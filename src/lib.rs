use serde::{Deserialize, Serialize};

/// Protocol version sent by the agent in the Register handshake. Bump when
/// the wire format changes in a way the server needs to reject older agents
/// for. Value `0` means "legacy agent that predates this field" — those
/// still connect, just without the version-aware fast paths.
pub const PROTOCOL_VERSION: u32 = 1;

fn default_protocol_version() -> u32 {
    0
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

    /// Ping / Pong for heartbeat
    Ping,
    Pong,

    /// Request to list systemd services
    ListServicesRequest,

    /// Response containing systemd services
    ListServicesResponse { services: Vec<ServiceInfo> },

    /// Request to control a service (start, stop, restart)
    ControlServiceRequest { name: String, action: String },

    /// Response to control a service
    ControlServiceResponse { name: String, success: bool, error: Option<String> },

    /// Request to start a terminal session
    StartTerminalRequest,

    /// Terminal data
    TerminalData { data: Vec<u8> },
    
    /// Request to resize terminal
    TerminalResize { cols: u16, rows: u16 },

    /// Request to read a configuration file
    ReadConfigRequest { path: String },

    /// Response containing file content
    ReadConfigResponse { path: String, content: String, error: Option<String> },

    /// Request to write a configuration file
    WriteConfigRequest { path: String, content: String },

    /// Response to write config
    WriteConfigResponse { path: String, success: bool, error: Option<String> },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub description: String,
    pub status: String,
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
