use shared::{ContainerSpec, ServiceSpec};
use tokio::process::Command;

fn collect_log(stdout: &[u8], stderr: &[u8]) -> String {
    let mut s = String::from_utf8_lossy(stdout).into_owned();
    let err = String::from_utf8_lossy(stderr);
    if !err.is_empty() {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&err);
    }
    s
}

fn split_command(cmd: &str) -> Vec<String> {
    // Naive shell-style split that respects single/double quotes. Good
    // enough for the dashboard form's "Command" textbox; users with
    // exotic quoting needs can pass the full container spec via a
    // SwarmCreateServiceRequest from a tool.
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in cmd.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(q) if q == ch => quote = None,
            Some(_) => cur.push(ch),
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub async fn create_container(
    spec: &ContainerSpec,
) -> (bool, Option<String>, String, Option<String>) {
    if spec.image.trim().is_empty() {
        return (
            false,
            None,
            String::new(),
            Some("image is required".to_string()),
        );
    }

    let mut cmd = Command::new("docker");
    cmd.arg("run");
    if spec.detached {
        cmd.arg("-d");
    }
    if let Some(ref name) = spec.name {
        if !name.trim().is_empty() {
            cmd.arg("--name").arg(name.trim());
        }
    }
    for p in &spec.ports {
        let p = p.trim();
        if !p.is_empty() {
            cmd.arg("-p").arg(p);
        }
    }
    for e in &spec.env {
        let e = e.trim();
        if !e.is_empty() {
            cmd.arg("-e").arg(e);
        }
    }
    for v in &spec.volumes {
        let v = v.trim();
        if !v.is_empty() {
            cmd.arg("-v").arg(v);
        }
    }
    if let Some(ref restart) = spec.restart_policy {
        let r = restart.trim();
        if !r.is_empty() && r != "no" {
            cmd.arg("--restart").arg(r);
        }
    }
    if let Some(ref network) = spec.network {
        let n = network.trim();
        if !n.is_empty() {
            cmd.arg("--network").arg(n);
        }
    }
    if spec.pull {
        cmd.arg("--pull").arg("always");
    }
    cmd.arg(&spec.image);
    if let Some(ref command) = spec.command {
        for arg in split_command(command) {
            cmd.arg(arg);
        }
    }

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            return (
                false,
                None,
                String::new(),
                Some(format!("docker spawn: {e}")),
            );
        }
    };

    let success = output.status.success();
    let log = collect_log(&output.stdout, &output.stderr);
    let container_id = if success {
        Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .last()
                .unwrap_or("")
                .trim()
                .to_string(),
        )
        .filter(|s| !s.is_empty())
    } else {
        None
    };
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, container_id, log, err)
}

pub async fn create_service(spec: &ServiceSpec) -> (bool, Option<String>, String, Option<String>) {
    if spec.image.trim().is_empty() {
        return (
            false,
            None,
            String::new(),
            Some("image is required".to_string()),
        );
    }
    if spec.name.trim().is_empty() {
        return (
            false,
            None,
            String::new(),
            Some("name is required".to_string()),
        );
    }

    let mut cmd = Command::new("docker");
    cmd.args(["service", "create", "--detach=true"]);
    cmd.arg("--name").arg(spec.name.trim());

    if let Some(ref mode) = spec.mode {
        let m = mode.trim();
        if !m.is_empty() {
            cmd.arg("--mode").arg(m);
        }
    }
    if let Some(replicas) = spec.replicas {
        // --replicas is rejected when mode=global; let the caller decide.
        if spec.mode.as_deref() != Some("global") {
            cmd.arg("--replicas").arg(replicas.to_string());
        }
    }
    for p in &spec.ports {
        let p = p.trim();
        if !p.is_empty() {
            cmd.arg("--publish").arg(p);
        }
    }
    for e in &spec.env {
        let e = e.trim();
        if !e.is_empty() {
            cmd.arg("--env").arg(e);
        }
    }
    for m in &spec.mounts {
        let m = m.trim();
        if !m.is_empty() {
            cmd.arg("--mount").arg(m);
        }
    }
    for c in &spec.constraints {
        let c = c.trim();
        if !c.is_empty() {
            cmd.arg("--constraint").arg(c);
        }
    }
    for n in &spec.networks {
        let n = n.trim();
        if !n.is_empty() {
            cmd.arg("--network").arg(n);
        }
    }
    if let Some(ref cond) = spec.restart_condition {
        let c = cond.trim();
        if !c.is_empty() {
            cmd.arg("--restart-condition").arg(c);
        }
    }
    cmd.arg(&spec.image);
    if let Some(ref command) = spec.command {
        for arg in split_command(command) {
            cmd.arg(arg);
        }
    }

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            return (
                false,
                None,
                String::new(),
                Some(format!("docker spawn: {e}")),
            );
        }
    };

    let success = output.status.success();
    let log = collect_log(&output.stdout, &output.stderr);
    let service_id = if success {
        // `docker service create --detach=true` prints the service ID on stdout
        // followed by the rolling update progress. Take the first non-empty
        // line which is the ID.
        Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .to_string(),
        )
        .filter(|s| !s.is_empty())
    } else {
        None
    };
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, service_id, log, err)
}
