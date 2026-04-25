use serde::Deserialize;
use shared::{DockerContainer, DockerContainerAction, SwarmAction, SwarmNode, SwarmRole, SwarmService};
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

/// Run a lifecycle action against a single container by ID or name.
pub async fn run_container_action(
    id: &str,
    action: DockerContainerAction,
) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("docker");
    match action {
        DockerContainerAction::Start => cmd.args(["start", id]),
        DockerContainerAction::Stop => cmd.args(["stop", id]),
        DockerContainerAction::Restart => cmd.args(["restart", id]),
        // -f so we don't fail on a running container the operator
        // explicitly chose to remove.
        DockerContainerAction::Remove => cmd.args(["rm", "-f", id]),
    };
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("docker spawn: {e}"))),
    };
    let success = output.status.success();
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    let err_text = String::from_utf8_lossy(&output.stderr);
    if !err_text.is_empty() {
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&err_text);
    }
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, log, err)
}

/// Run a swarm management action against a service. Returns combined
/// stdout/stderr so the dashboard can show the operator what docker
/// said. Only valid on a manager — the caller should gate with
/// swarm_role().
pub async fn run_swarm_action(name: &str, action: &SwarmAction) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("docker");
    match action {
        SwarmAction::Scale(n) => {
            cmd.args(["service", "scale", &format!("{name}={n}")]);
        }
        SwarmAction::ForceUpdate => {
            cmd.args(["service", "update", "--force", name]);
        }
        SwarmAction::Remove => {
            cmd.args(["service", "rm", name]);
        }
    }
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("docker spawn: {e}"))),
    };
    let success = output.status.success();
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    let err_text = String::from_utf8_lossy(&output.stderr);
    if !err_text.is_empty() {
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&err_text);
    }
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, log, err)
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

