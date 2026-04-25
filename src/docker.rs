use serde::Deserialize;
use shared::{DockerContainer, SwarmNode, SwarmRole, SwarmService};
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct DockerPsRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "State")]
    state: String,
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "Ports")]
    ports: String,
}

#[derive(Debug, Deserialize)]
struct DockerInfoSwarm {
    #[serde(rename = "LocalNodeState")]
    local_node_state: String,
    #[serde(rename = "ControlAvailable")]
    control_available: bool,
}

#[derive(Debug, Deserialize)]
struct DockerInfo {
    #[serde(rename = "Swarm")]
    swarm: Option<DockerInfoSwarm>,
}

async fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .arg("--format")
        .arg("{{.Server.Version}}")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn swarm_role() -> SwarmRole {
    if !docker_available().await {
        return SwarmRole::NotInSwarm;
    }
    let output = Command::new("docker")
        .args(["info", "--format", "{{json .}}"])
        .output()
        .await;
    let Ok(output) = output else {
        return SwarmRole::NotInSwarm;
    };
    if !output.status.success() {
        return SwarmRole::NotInSwarm;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let info: DockerInfo = match serde_json::from_str(&stdout) {
        Ok(i) => i,
        Err(_) => return SwarmRole::NotInSwarm,
    };
    let Some(s) = info.swarm else {
        return SwarmRole::NotInSwarm;
    };
    if s.local_node_state != "active" {
        return SwarmRole::NotInSwarm;
    }
    if s.control_available {
        SwarmRole::Manager
    } else {
        SwarmRole::Worker
    }
}

pub async fn list_containers() -> Result<Vec<DockerContainer>, String> {
    let output = Command::new("docker")
        .args(["ps", "-a", "--format", "{{json .}}", "--no-trunc"])
        .output()
        .await
        .map_err(|e| format!("docker spawn: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: DockerPsRow = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        out.push(DockerContainer {
            id: row.id.chars().take(12).collect(),
            names: row.names,
            image: row.image,
            state: row.state,
            status: row.status,
            ports: row.ports,
        });
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct SvcRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Mode")]
    mode: String,
    #[serde(rename = "Replicas")]
    replicas: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Ports")]
    ports: String,
}

pub async fn list_swarm_services() -> Result<Vec<SwarmService>, String> {
    let output = Command::new("docker")
        .args(["service", "ls", "--format", "{{json .}}"])
        .output()
        .await
        .map_err(|e| format!("docker spawn: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: SvcRow = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        out.push(SwarmService {
            id: row.id,
            name: row.name,
            mode: row.mode,
            replicas: row.replicas,
            image: row.image,
            ports: row.ports,
        });
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct NodeRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Hostname")]
    hostname: String,
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "Availability")]
    availability: String,
    #[serde(rename = "ManagerStatus")]
    manager_status: String,
    #[serde(rename = "EngineVersion")]
    engine_version: String,
}

pub async fn list_swarm_nodes() -> Result<Vec<SwarmNode>, String> {
    let output = Command::new("docker")
        .args(["node", "ls", "--format", "{{json .}}"])
        .output()
        .await
        .map_err(|e| format!("docker spawn: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: NodeRow = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        out.push(SwarmNode {
            id: row.id,
            hostname: row.hostname,
            status: row.status,
            availability: row.availability,
            manager_status: row.manager_status,
            engine_version: row.engine_version,
        });
    }
    Ok(out)
}

