use shared::AptUpgradable;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Where the agent journals an in-flight apt run. If a libc/systemd
/// self-upgrade kills the agent mid-run, this file lets the next agent
/// process synthesise an `AptUpgradeResponse` instead of leaving the
/// scheduler hanging on `last_status="running"` forever.
fn apt_state_path() -> PathBuf {
    PathBuf::from("/var/lib/sys-manager/apt-run.json")
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AptRunState {
    pub package: Option<String>,
    pub started_at: u64,
    pub log: String,
    /// Truncated to a sane limit on every flush.
    pub bytes_written: usize,
}

const APT_LOG_CAP: usize = 16_000;

fn truncate_inplace(s: &mut String, cap: usize) {
    if s.len() > cap {
        let cut = s.len() - cap;
        let head = format!("…[{cut} bytes truncated]…\n");
        s.drain(..cut);
        s.insert_str(0, &head);
    }
}

fn write_state(state: &AptRunState) {
    if let Ok(json) = serde_json::to_string(state) {
        let path = apt_state_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, json);
    }
}

fn clear_state() {
    let _ = std::fs::remove_file(apt_state_path());
}

/// Read any persisted apt run state. The agent calls this once at
/// startup; if a run was in flight when we exited, we synthesise an
/// `AptUpgradeResponse` for it.
pub fn take_pending_run() -> Option<AptRunState> {
    let path = apt_state_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let state: AptRunState = serde_json::from_str(&data).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(state)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn last_apt_update_secs() -> u64 {
    // First successful apt-get update writes this stamp on Debian-derived
    // systems. Fall back to the package cache mtime — it's regenerated on
    // every apt-get update too.
    for path in [
        "/var/lib/apt/periodic/update-success-stamp",
        "/var/cache/apt/pkgcache.bin",
    ] {
        if let Ok(meta) = std::fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if let Ok(d) = modified.duration_since(UNIX_EPOCH) {
                    return d.as_secs();
                }
            }
        }
    }
    0
}

fn parse_apt_list(stdout: &str) -> Vec<AptUpgradable> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        // Skip the leading "Listing... Done" header and blank lines.
        if line.is_empty() || line.starts_with("Listing") || line.starts_with("WARNING") {
            continue;
        }
        // Format: name/source new-version arch [upgradable from: old-version]
        // Example: sys-manager-agent/now 1.1.0-ci... amd64 [upgradable from: 1.0.0]
        let mut parts = line.split_whitespace();
        let name_source = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        let new_version = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        let _arch = parts.next();
        let mut current_version = String::new();
        let rest: Vec<&str> = parts.collect();
        let joined = rest.join(" ");
        if let Some(start) = joined.find("upgradable from: ") {
            let tail = &joined[start + "upgradable from: ".len()..];
            current_version = tail.trim_end_matches(']').trim().to_string();
        }
        let mut split = name_source.splitn(2, '/');
        let name = split.next().unwrap_or("").to_string();
        let source = split.next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        out.push(AptUpgradable {
            name,
            current_version,
            new_version: new_version.to_string(),
            source,
        });
    }
    out
}

pub async fn list_upgradable() -> Result<Vec<AptUpgradable>, String> {
    let output = Command::new("apt")
        .args(["list", "--upgradable"])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C")
        .output()
        .await
        .map_err(|e| format!("apt spawn: {e}"))?;
    // apt warns about CLI usage on stderr; that's fine, we only parse stdout.
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_apt_list(&stdout))
}

fn truncate_log(buf: &[u8], limit: usize) -> String {
    let text = String::from_utf8_lossy(buf);
    if text.len() <= limit {
        text.into_owned()
    } else {
        let cut = text.len() - limit;
        format!("…[{cut} bytes truncated]…\n{}", &text[text.len() - limit..])
    }
}

pub async fn refresh() -> (bool, String, Option<String>) {
    let output = match Command::new("apt-get")
        .arg("update")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C")
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let success = output.status.success();
    let mut log = truncate_log(&output.stdout, 4000);
    if !output.stderr.is_empty() {
        log.push_str("\n--- stderr ---\n");
        log.push_str(&truncate_log(&output.stderr, 4000));
    }
    let _ = now_unix();
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, log, err)
}

pub async fn upgrade(package: Option<String>) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("apt-get");
    cmd.env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C")
        .arg("-y")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match &package {
        Some(p) => {
            cmd.arg("install").arg("--only-upgrade").arg(p);
        }
        None => {
            cmd.arg("upgrade");
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let started_at = now_unix();
    let mut state = AptRunState {
        package: package.clone(),
        started_at,
        log: String::new(),
        bytes_written: 0,
    };
    write_state(&state);

    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if let Some(out) = stdout {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx.send(line);
            }
        });
    }
    if let Some(err) = stderr {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx.send(format!("[stderr] {line}"));
            }
        });
    }
    drop(line_tx); // close once children drop their senders

    let mut last_flush = tokio::time::Instant::now();
    let flush_every = std::time::Duration::from_secs(2);
    loop {
        tokio::select! {
            line = line_rx.recv() => match line {
                Some(l) => {
                    state.log.push_str(&l);
                    state.log.push('\n');
                    state.bytes_written += l.len() + 1;
                    truncate_inplace(&mut state.log, APT_LOG_CAP);
                    if last_flush.elapsed() >= flush_every {
                        write_state(&state);
                        last_flush = tokio::time::Instant::now();
                    }
                }
                None => break,
            },
            _ = tokio::time::sleep_until(last_flush + flush_every) => {
                write_state(&state);
                last_flush = tokio::time::Instant::now();
            }
        }
    }

    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => {
            // Don't clear state — the next process may want to report
            // the partial log.
            return (false, state.log, Some(format!("wait: {e}")));
        }
    };
    clear_state();
    let success = status.success();
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", status.code()))
    };
    (success, state.log, err)
}
