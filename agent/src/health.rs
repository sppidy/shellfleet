//! Per-agent health probes — HTTP and TCP. Each probe runs on its own
//! tokio task; reports flow back through the shared message channel
//! when the state transitions (or on first sample after a sync).

use agent::Outgoing;
use futures_util::StreamExt;
use shared::{HealthProbeKind, HealthProbeResult, HealthProbeSpec, HealthProbeState, Message};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Instant;

const HTTP_BODY_CAP: usize = 1_048_576;

fn append_body_chunk(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), ()> {
    if body.len().saturating_add(chunk.len()) > HTTP_BODY_CAP {
        return Err(());
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// Where exec-kind probes are required to live. Anything else is rejected.
fn probes_dir() -> PathBuf {
    PathBuf::from("/etc/shellfleet/probes.d")
}

#[derive(Default, Clone)]
pub struct HealthProbes {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// id → (signature, JoinHandle). Signature is the serde_json
    /// representation of the spec, so we can detect when *anything*
    /// changed and respawn.
    tasks: HashMap<String, (String, JoinHandle<()>)>,
}

impl HealthProbes {
    /// Apply a new probe set. Spawns/aborts tasks so the running set
    /// matches `probes`.
    pub async fn sync(&self, probes: Vec<HealthProbeSpec>, tx: Outgoing) {
        let mut inner = self.inner.lock().await;
        let mut keep: HashMap<String, ()> = HashMap::with_capacity(probes.len());
        for p in probes {
            let sig = match serde_json::to_string(&p) {
                Ok(s) => s,
                Err(_) => continue,
            };
            keep.insert(p.id.clone(), ());
            if let Some((existing_sig, _)) = inner.tasks.get(&p.id) {
                if existing_sig == &sig {
                    continue;
                }
                // Spec changed — abort old and respawn below.
                if let Some((_, h)) = inner.tasks.remove(&p.id) {
                    h.abort();
                }
            }
            let tx_clone = tx.clone();
            let id_for_task = p.id.clone();
            let id_for_map = p.id.clone();
            let handle = tokio::spawn(async move {
                run_probe(p, tx_clone).await;
                tracing_drop(&id_for_task);
            });
            inner.tasks.insert(id_for_map, (sig, handle));
        }
        // Abort tasks not present in the new set.
        let to_remove: Vec<String> = inner
            .tasks
            .keys()
            .filter(|k| !keep.contains_key(*k))
            .cloned()
            .collect();
        for id in to_remove {
            if let Some((_, h)) = inner.tasks.remove(&id) {
                h.abort();
            }
        }
    }
}

fn tracing_drop(_id: &str) {}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn run_probe(spec: HealthProbeSpec, tx: Outgoing) {
    let interval = Duration::from_secs(spec.interval_secs.max(1) as u64);
    let timeout = Duration::from_secs(spec.timeout_secs.max(1) as u64);
    let mut last_state: Option<HealthProbeState> = None;
    // A short initial delay so a brand-new probe doesn't fire all at once
    // alongside every other probe on the host.
    let jitter = Duration::from_millis(
        (spec.id.bytes().fold(0u64, |a, b| a.wrapping_add(b as u64)) % 5_000) as u64,
    );
    tokio::time::sleep(jitter).await;
    loop {
        let started = Instant::now();
        let (state, detail) = match spec.kind {
            HealthProbeKind::Http => probe_http(&spec, timeout).await,
            HealthProbeKind::Tcp => probe_tcp(&spec, timeout).await,
            HealthProbeKind::Exec => probe_exec(&spec, timeout).await,
        };
        let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        if last_state != Some(state) {
            // Send a report whenever state flips (or on first sample).
            let _ = tx.send(Message::HealthProbeReport {
                results: vec![HealthProbeResult {
                    id: spec.id.clone(),
                    state,
                    latency_ms,
                    detail: detail.clone(),
                    at: now_unix(),
                }],
            });
            last_state = Some(state);
        }
        tokio::time::sleep(interval).await;
    }
}

async fn probe_http(spec: &HealthProbeSpec, timeout: Duration) -> (HealthProbeState, String) {
    let client = match reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => return (HealthProbeState::Red, format!("client build: {e}")),
    };
    let resp = match client.get(&spec.target).send().await {
        Ok(r) => r,
        Err(e) => return (HealthProbeState::Red, format!("connect: {e}")),
    };
    let status = resp.status().as_u16();
    let status_ok = match spec.expect_status {
        Some(want) => status == want,
        None => resp.status().is_success(),
    };
    if !status_ok {
        return (HealthProbeState::Red, format!("unexpected status {status}"));
    }
    if let Some(want_body) = &spec.expect_body {
        let mut bytes = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => return (HealthProbeState::Red, format!("read body: {e}")),
            };
            if append_body_chunk(&mut bytes, &chunk).is_err() {
                return (
                    HealthProbeState::Red,
                    format!("response body exceeds {HTTP_BODY_CAP} bytes"),
                );
            }
        }
        let body = String::from_utf8_lossy(&bytes);
        if !body.contains(want_body) {
            return (
                HealthProbeState::Red,
                format!("body missing {want_body:?} (status {status})"),
            );
        }
    }
    (HealthProbeState::Green, format!("ok ({status})"))
}

async fn probe_tcp(spec: &HealthProbeSpec, timeout: Duration) -> (HealthProbeState, String) {
    let target = spec.target.clone();
    let connect = tokio::net::TcpStream::connect(target);
    match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(_)) => (HealthProbeState::Green, "ok".to_string()),
        Ok(Err(e)) => (HealthProbeState::Red, format!("connect: {e}")),
        Err(_) => (
            HealthProbeState::Red,
            format!("timeout after {}s", timeout.as_secs()),
        ),
    }
}

async fn probe_exec(spec: &HealthProbeSpec, timeout: Duration) -> (HealthProbeState, String) {
    // Reject anything that looks like a path. Operator scripts must
    // live in /etc/shellfleet/probes.d/<filename>.
    let target = spec.target.trim();
    if target.is_empty() || target.contains('/') || target.contains('\\') || target.contains("..") {
        return (
            HealthProbeState::Red,
            format!(
                "invalid exec target {target:?} (must be a filename in /etc/shellfleet/probes.d/)"
            ),
        );
    }
    let path = probes_dir().join(target);
    if !Path::new(&path).is_file() {
        return (
            HealthProbeState::Red,
            format!("script not found: {}", path.display()),
        );
    }
    let mut cmd = Command::new(&path);
    // A probe inherits no agent-process environment. In particular, values
    // such as LD_PRELOAD must never be able to alter the probe interpreter or
    // a helper it invokes. Keep a predictable PATH for shebangs that use
    // `/usr/bin/env` and then add only vetted operator-provided variables.
    cmd.env_clear();
    cmd.env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    // Per-probe env (`KEY=VALUE`). Anything malformed is silently
    // skipped — operators see the bad pair in the spec, not a
    // mid-script failure.
    for kv in &spec.env {
        if let Some((k, v)) = kv.split_once('=') {
            if is_safe_probe_env_key(k) {
                cmd.env(k, v);
            } else {
                eprintln!("health probe {target:?}: ignored unsafe environment variable {k:?}");
            }
        }
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (HealthProbeState::Red, format!("spawn: {e}")),
    };
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return (HealthProbeState::Red, format!("wait: {e}")),
        Err(_) => {
            return (
                HealthProbeState::Red,
                format!("timeout after {}s", timeout.as_secs()),
            );
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    if output.status.success() {
        let detail = if first_line.is_empty() {
            "ok".to_string()
        } else {
            first_line
        };
        (HealthProbeState::Green, detail)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = format!(
            "exit {} {}",
            output.status.code().unwrap_or(-1),
            if !first_line.is_empty() {
                first_line
            } else {
                stderr
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>()
            }
        );
        (HealthProbeState::Red, detail)
    }
}

/// A probe spec crosses the server-to-root-agent trust boundary. Dynamic
/// loader, shell, and language-runtime controls can change what an otherwise
/// fixed probe script executes, so reject them even though ordinary `NAME=value`
/// configuration is allowed.
fn is_safe_probe_env_key(key: &str) -> bool {
    if key.is_empty()
        || !key.bytes().enumerate().all(|(i, b)| {
            (b == b'_' || b.is_ascii_alphanumeric())
                && (i != 0 || b == b'_' || b.is_ascii_alphabetic())
        })
    {
        return false;
    }

    !matches!(
        key,
        "PATH"
            | "BASH_ENV"
            | "ENV"
            | "CDPATH"
            | "GLOBIGNORE"
            | "SHELLOPTS"
            | "BASHOPTS"
            | "PYTHONHOME"
            | "PYTHONPATH"
            | "PYTHONSTARTUP"
            | "PERL5LIB"
            | "PERL5OPT"
            | "RUBYLIB"
            | "RUBYOPT"
            | "NODE_OPTIONS"
            | "JAVA_TOOL_OPTIONS"
            | "JDK_JAVA_OPTIONS"
    ) && !key.starts_with("LD_")
        && !key.starts_with("DYLD_")
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_environment_rejects_execution_controls() {
        for key in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "PATH",
            "BASH_ENV",
            "PYTHONPATH",
            "NODE_OPTIONS",
        ] {
            assert!(!super::is_safe_probe_env_key(key), "{key} must be rejected");
        }
        assert!(super::is_safe_probe_env_key("CHECK_INTERVAL"));
        assert!(!super::is_safe_probe_env_key("1INVALID"));
    }

    #[test]
    fn response_body_cap_rejects_oversized_chunks() {
        let mut body = Vec::new();
        assert!(super::append_body_chunk(&mut body, &[1; 512]).is_ok());
        assert!(super::append_body_chunk(&mut body, &vec![2; super::HTTP_BODY_CAP]).is_err());
        assert!(body.len() <= super::HTTP_BODY_CAP);
    }
}
