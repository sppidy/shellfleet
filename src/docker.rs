use serde::Deserialize;
use shared::{
    DockerContainer, DockerContainerAction, SwarmAction, SwarmNode, SwarmRole, SwarmService,
    SwarmServiceSpecSummary, SwarmTask,
};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
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

#[derive(Debug, Deserialize)]
struct ServicePsRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Node")]
    node: String,
    #[serde(rename = "DesiredState")]
    desired_state: String,
    #[serde(rename = "CurrentState")]
    current_state: String,
    #[serde(rename = "Error")]
    #[serde(default)]
    error: String,
    #[serde(rename = "Image")]
    image: String,
}

pub async fn service_ps(name: &str) -> Result<Vec<SwarmTask>, String> {
    let output = Command::new("docker")
        .args(["service", "ps", name, "--format", "{{json .}}", "--no-trunc"])
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
        let row: ServicePsRow = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        out.push(SwarmTask {
            id: row.id,
            name: row.name,
            node: row.node,
            desired_state: row.desired_state,
            current_state: row.current_state,
            error: row.error,
            image: row.image,
        });
    }
    Ok(out)
}

pub async fn service_inspect(name: &str) -> Result<SwarmServiceSpecSummary, String> {
    let output = Command::new("docker")
        .args(["service", "inspect", name])
        .output()
        .await
        .map_err(|e| format!("docker spawn: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("parse inspect json: {e}"))?;
    let inspect = value
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| "empty inspect array".to_string())?;

    let spec = &inspect["Spec"];
    let task_template = &spec["TaskTemplate"];
    let container_spec = &task_template["ContainerSpec"];

    let image = container_spec["Image"].as_str().unwrap_or("").to_string();
    // Image typically looks like `repo/name:tag@sha256:…`. Split off the
    // digest if present so the UI can show a stable identifier.
    let (image_pretty, image_digest) = match image.split_once("@sha256:") {
        Some((tag, digest)) => (tag.to_string(), format!("sha256:{digest}")),
        None => (image.clone(), String::new()),
    };

    let mode_obj = &spec["Mode"];
    let (mode, replicas) = if let Some(repl) = mode_obj.get("Replicated") {
        (
            "replicated".to_string(),
            repl["Replicas"].as_u64().map(|v| v as u32),
        )
    } else if mode_obj.get("Global").is_some() {
        ("global".to_string(), None)
    } else {
        ("unknown".to_string(), None)
    };

    let env: Vec<String> = container_spec["Env"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mounts: Vec<String> = container_spec["Mounts"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| {
                    let typ = m["Type"].as_str().unwrap_or("?");
                    let source = m["Source"].as_str().unwrap_or("");
                    let target = m["Target"].as_str().unwrap_or("");
                    format!("type={typ},source={source},target={target}")
                })
                .collect()
        })
        .unwrap_or_default();
    let networks: Vec<String> = task_template["Networks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|n| n["Target"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let constraints: Vec<String> = task_template["Placement"]["Constraints"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let published_ports: Vec<String> = spec["EndpointSpec"]["Ports"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    let target = p["TargetPort"].as_u64().unwrap_or(0);
                    let published = p["PublishedPort"].as_u64().unwrap_or(0);
                    let proto = p["Protocol"].as_str().unwrap_or("tcp");
                    format!("{published}:{target}/{proto}")
                })
                .collect()
        })
        .unwrap_or_default();

    let created_at = inspect["CreatedAt"].as_str().unwrap_or("").to_string();
    let updated_at = inspect["UpdatedAt"].as_str().unwrap_or("").to_string();

    Ok(SwarmServiceSpecSummary {
        image: image_pretty,
        image_digest,
        mode,
        replicas,
        created_at,
        updated_at,
        env,
        mounts,
        networks,
        constraints,
        published_ports,
    })
}

/// Pipe a compose YAML to `docker stack deploy --compose-file -`. Returns
/// stdout+stderr so the operator can see what services were created /
/// updated.
pub async fn stack_deploy(
    stack_name: &str,
    compose_yaml: &str,
    prune: bool,
) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("docker");
    cmd.args([
        "stack",
        "deploy",
        "--compose-file",
        "-",
        "--with-registry-auth",
    ]);
    if prune {
        cmd.arg("--prune");
    }
    cmd.arg(stack_name);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, String::new(), Some(format!("docker spawn: {e}"))),
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(compose_yaml.as_bytes()).await {
            return (false, String::new(), Some(format!("write stdin: {e}")));
        }
        // Drop stdin so docker reads EOF.
        drop(stdin);
    }

    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("docker wait: {e}"))),
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


// ---------- images (v11) ----------

pub async fn list_images() -> Result<Vec<shared::DockerImage>, String> {
    // `docker images --format "{{json .}}"` emits one JSON object per
    // image, ndjson-style. We parse defensively and skip rows that
    // don't deserialise.
    let output = match Command::new("docker")
        .args([
            "images",
            "--no-trunc",
            "--format",
            "{{json .}}",
        ])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return Err(format!("spawn: {e}")),
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "ID", default)]
        id: String,
        #[serde(rename = "Repository", default)]
        repository: String,
        #[serde(rename = "Tag", default)]
        tag: String,
        #[serde(rename = "Size", default)]
        size: String,
        #[serde(rename = "CreatedSince", default)]
        created_since: String,
        #[serde(rename = "CreatedAt", default)]
        created_at: String,
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out: Vec<shared::DockerImage> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(trimmed) else {
            continue;
        };
        // Strip "sha256:" prefix from the id (cargo bin format leaves
        // it; --no-trunc keeps it).
        let id = row
            .id
            .strip_prefix("sha256:")
            .unwrap_or(&row.id)
            .to_string();
        out.push(shared::DockerImage {
            id,
            repository: row.repository,
            tag: row.tag,
            size_bytes: parse_docker_size(&row.size),
            created: if row.created_since.is_empty() {
                row.created_at
            } else {
                row.created_since
            },
        });
    }
    Ok(out)
}

/// Parses docker's human-friendly size string ("12.3MB", "1.4GB", "894kB")
/// into bytes. Returns 0 on parse failure rather than refusing the row.
fn parse_docker_size(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    // Find where digits/decimal end.
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value: f64 = match num.parse() {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let multiplier: f64 = match unit.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "TB" => 1_000_000_000_000.0,
        "KIB" => 1_024.0,
        "MIB" => 1_024.0 * 1_024.0,
        "GIB" => 1_024.0 * 1_024.0 * 1_024.0,
        _ => return 0,
    };
    (value * multiplier) as u64
}

pub async fn remove_image(id: &str, force: bool) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("docker");
    cmd.arg("rmi");
    if force {
        cmd.arg("--force");
    }
    cmd.arg(id);
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    let success = output.status.success();
    let err = if success {
        None
    } else {
        Some(format!("docker rmi exit {:?}", output.status.code()))
    };
    (success, log, err)
}

pub async fn pull_image(reference: &str) -> (bool, String, Option<String>) {
    let output = match Command::new("docker")
        .args(["pull", reference])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    let success = output.status.success();
    let err = if success {
        None
    } else {
        Some(format!("docker pull exit {:?}", output.status.code()))
    };
    (success, log, err)
}

fn combine_stdout_stderr(output: &std::process::Output) -> String {
    let mut s = String::new();
    if !output.stdout.is_empty() {
        s.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !s.is_empty() {
            s.push_str("\n--- stderr ---\n");
        }
        s.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if s.len() > 8_000 {
        let cut = s.len() - 8_000;
        let head = format!("…[{cut} bytes truncated]…\n");
        s.drain(..cut);
        s.insert_str(0, &head);
    }
    s
}

// ---------- networks (v12) ----------

pub async fn list_networks() -> Result<Vec<shared::DockerNetwork>, String> {
    let output = match Command::new("docker")
        .args(["network", "ls", "--no-trunc", "--format", "{{json .}}"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return Err(format!("spawn: {e}")),
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "ID", default)]
        id: String,
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "Driver", default)]
        driver: String,
        #[serde(rename = "Scope", default)]
        scope: String,
        #[serde(rename = "CreatedAt", default)]
        created_at: String,
        #[serde(rename = "IPv6", default)]
        ipv6: String,
        #[serde(rename = "Internal", default)]
        internal: String,
        #[serde(rename = "Attachable", default)]
        attachable: String,
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        out.push(shared::DockerNetwork {
            id: row.id,
            name: row.name,
            driver: row.driver,
            scope: row.scope,
            created: row.created_at,
            ipv6: matches!(row.ipv6.as_str(), "true" | "True" | "1"),
            internal: matches!(row.internal.as_str(), "true" | "True" | "1"),
            attachable: matches!(row.attachable.as_str(), "true" | "True" | "1"),
        });
    }
    Ok(out)
}

pub async fn inspect_network(id: &str) -> (bool, String, Option<String>) {
    let output = match Command::new("docker")
        .args(["network", "inspect", id])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if success {
        (true, stdout, None)
    } else {
        (false, stdout, Some(stderr))
    }
}

pub async fn create_network(
    name: &str,
    driver: &str,
    subnet: Option<&str>,
    attachable: bool,
    internal: bool,
) -> (bool, Option<String>, String, Option<String>) {
    let mut cmd = Command::new("docker");
    cmd.args(["network", "create", "--driver", driver]);
    if let Some(s) = subnet {
        if !s.is_empty() {
            cmd.args(["--subnet", s]);
        }
    }
    if attachable {
        cmd.arg("--attachable");
    }
    if internal {
        cmd.arg("--internal");
    }
    cmd.arg(name);
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return (false, None, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    if output.status.success() {
        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (true, Some(id), log, None)
    } else {
        (false, None, log, Some(format!("docker network create exit {:?}", output.status.code())))
    }
}

pub async fn remove_network(id: &str) -> (bool, String, Option<String>) {
    let output = match Command::new("docker")
        .args(["network", "rm", id])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    let success = output.status.success();
    let err = if success {
        None
    } else {
        Some(format!("docker network rm exit {:?}", output.status.code()))
    };
    (success, log, err)
}

// ---------- volumes (v12) ----------

pub async fn list_volumes() -> Result<Vec<shared::DockerVolume>, String> {
    let output = match Command::new("docker")
        .args(["volume", "ls", "--format", "{{json .}}"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return Err(format!("spawn: {e}")),
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "Driver", default)]
        driver: String,
        #[serde(rename = "Mountpoint", default)]
        mountpoint: String,
        #[serde(rename = "Size", default)]
        size: String,
        #[serde(rename = "Labels", default)]
        labels: String,
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        out.push(shared::DockerVolume {
            name: row.name,
            driver: row.driver,
            mountpoint: row.mountpoint,
            size_bytes: parse_docker_size(&row.size),
            created: String::new(),
            labels: row.labels,
        });
    }
    Ok(out)
}

pub async fn inspect_volume(name: &str) -> (bool, String, Option<String>) {
    let output = match Command::new("docker")
        .args(["volume", "inspect", name])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if success {
        (true, stdout, None)
    } else {
        (false, stdout, Some(stderr))
    }
}

pub async fn remove_volume(name: &str, force: bool) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("docker");
    cmd.args(["volume", "rm"]);
    if force {
        cmd.arg("--force");
    }
    cmd.arg(name);
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    let success = output.status.success();
    let err = if success {
        None
    } else {
        Some(format!("docker volume rm exit {:?}", output.status.code()))
    };
    (success, log, err)
}

pub async fn prune_volumes() -> (bool, Vec<String>, u64, String, Option<String>) {
    let output = match Command::new("docker")
        .args(["volume", "prune", "--force"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, Vec::new(), 0, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    if !output.status.success() {
        return (
            false,
            Vec::new(),
            0,
            log,
            Some(format!("docker volume prune exit {:?}", output.status.code())),
        );
    }
    let mut removed = Vec::new();
    let mut reclaimed_bytes: u64 = 0;
    let mut in_deleted = false;
    for line in log.lines() {
        let l = line.trim();
        if l.starts_with("Deleted Volumes") {
            in_deleted = true;
            continue;
        }
        if l.starts_with("Total reclaimed space:") {
            in_deleted = false;
            let value = l.trim_start_matches("Total reclaimed space:").trim();
            reclaimed_bytes = parse_docker_size(value);
            continue;
        }
        if in_deleted && !l.is_empty() {
            removed.push(l.to_string());
        }
    }
    (true, removed, reclaimed_bytes, log, None)
}

// ---------- swarm stacks (v12, manager-only) ----------

pub async fn list_stacks() -> Result<Vec<shared::SwarmStack>, String> {
    let output = match Command::new("docker")
        .args(["stack", "ls", "--format", "{{json .}}"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return Err(format!("spawn: {e}")),
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "Services", default)]
        services: String,
        #[serde(rename = "Orchestrator", default)]
        orchestrator: String,
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        out.push(shared::SwarmStack {
            name: row.name,
            services: row.services.parse().unwrap_or(0),
            orchestrator: row.orchestrator,
        });
    }
    Ok(out)
}

pub async fn stack_services(name: &str) -> Result<Vec<shared::SwarmService>, String> {
    let output = match Command::new("docker")
        .args(["stack", "services", "--format", "{{json .}}", name])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return Err(format!("spawn: {e}")),
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "ID", default)]
        id: String,
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "Mode", default)]
        mode: String,
        #[serde(rename = "Replicas", default)]
        replicas: String,
        #[serde(rename = "Image", default)]
        image: String,
        #[serde(rename = "Ports", default)]
        ports: String,
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        out.push(shared::SwarmService {
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

pub async fn stack_tasks(name: &str) -> Result<Vec<shared::SwarmTask>, String> {
    let output = match Command::new("docker")
        .args(["stack", "ps", "--no-trunc", "--format", "{{json .}}", name])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return Err(format!("spawn: {e}")),
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "ID", default)]
        id: String,
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "Node", default)]
        node: String,
        #[serde(rename = "DesiredState", default)]
        desired_state: String,
        #[serde(rename = "CurrentState", default)]
        current_state: String,
        #[serde(rename = "Error", default)]
        error: String,
        #[serde(rename = "Image", default)]
        image: String,
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Row>(line) else { continue };
        out.push(shared::SwarmTask {
            id: row.id,
            name: row.name,
            node: row.node,
            desired_state: row.desired_state,
            current_state: row.current_state,
            error: row.error,
            image: row.image,
        });
    }
    Ok(out)
}

pub async fn remove_stack(name: &str) -> (bool, String, Option<String>) {
    let output = match Command::new("docker")
        .args(["stack", "rm", name])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let log = combine_stdout_stderr(&output);
    let success = output.status.success();
    let err = if success {
        None
    } else {
        Some(format!("docker stack rm exit {:?}", output.status.code()))
    };
    (success, log, err)
}
