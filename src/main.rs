// W0 safety-net baseline: this crate predates a clippy gate. The lints
// below are pre-existing and stylistic; they are allowed crate-wide so a
// CI `-D warnings` gate enforces the substantive lints without churn.
// Tracked for a focused cleanup in W6.
#![allow(
    clippy::collapsible_if,
    clippy::ptr_arg,
    clippy::unnecessary_cast,
    clippy::unnecessary_sort_by,
    clippy::trim_split_whitespace,
    clippy::field_reassign_with_default,
    clippy::doc_lazy_continuation
)]

mod apt;
mod backup;
mod config;
mod deploy;
mod docker;
mod exec;
mod health;
mod journal;
mod journal_stream;
mod logs;
mod stats;
mod systemd;
mod terminal;

#[cfg(feature = "kube")]
mod k8s;

#[cfg(feature = "kube")]
mod k8s_logs;

#[cfg(feature = "kube")]
mod k8s_exec;

/// Write the bearer token to disk with mode 0600. Tries the operator
/// path first; on permission failure (e.g. the agent isn't running as
/// root and `/etc/shellfleet` doesn't exist) falls back to a CWD-
/// local file. Errors are intentionally swallowed because the token
/// is also returned in-memory and the caller proceeds either way.
fn write_token_secure(primary: &str, token: &str) {
    fn write_with_mode(path: &str, contents: &str) -> std::io::Result<()> {
        // Open with O_CREAT|O_TRUNC|O_WRONLY and explicit 0600 on Unix.
        #[cfg(unix)]
        {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            f.write_all(contents.as_bytes())
        }
        #[cfg(not(unix))]
        {
            // Windows: best-effort. The dashboard targets Linux agents
            // so this branch is for local test builds only.
            std::fs::write(path, contents)
        }
    }
    if write_with_mode(primary, token).is_ok() {
        return;
    }
    let _ = write_with_mode("agent-token.txt", token);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drift_config_fingerprint_rejects_denied_paths() {
        assert!(drift_config_fingerprint("/etc/shadow").await.is_none());
    }

    #[tokio::test]
    async fn drift_config_fingerprint_hashes_allowed_paths() {
        let Some(config) = drift_config_fingerprint("/etc/hostname").await else {
            panic!("expected /etc/hostname to be readable in the test environment");
        };

        assert_eq!(config.path, "/etc/hostname");
        assert!(!config.hash.is_empty());
        assert!(config.size > 0);
    }
}

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use shared::Message;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest, http::header::AUTHORIZATION, protocol::Message as WsMessage,
    },
};

#[derive(Deserialize)]
struct DeviceAuthResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DeviceTokenResponse {
    Token { access_token: String },
    Error { error: String },
}

const TOKEN_PATH: &str = "/etc/shellfleet/agent-token.txt";

fn read_token() -> Option<String> {
    for path in [TOKEN_PATH, "agent-token.txt"] {
        if let Ok(t) = std::fs::read_to_string(path) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

async fn drift_config_fingerprint(path: &str) -> Option<shared::DriftConfigFile> {
    let safe_path = config::check_read(path).ok()?;
    let meta = tokio::fs::metadata(&safe_path).await.ok()?;
    let content = tokio::fs::read(&safe_path).await.ok()?;
    use sha2::Digest;
    let hash = format!("{:x}", sha2::Sha256::digest(&content));
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(shared::DriftConfigFile {
        path: path.to_string(),
        hash,
        size: meta.len(),
        mtime,
    })
}

async fn pair(api_url: &str) -> String {
    let client = reqwest::Client::new();

    println!("Requesting device authorization...");
    let auth_res = client
        .post(format!("{}/api/device/request", api_url))
        .send()
        .await
        .expect("Failed to contact server");

    let auth_data: DeviceAuthResponse = auth_res.json().await.expect("Failed to parse response");

    println!("=======================================================");
    println!("To authenticate this agent, please visit:");
    println!("{}", auth_data.verification_uri);
    println!("And enter the code: {}", auth_data.user_code);
    println!("=======================================================");

    loop {
        tokio::time::sleep(Duration::from_secs(auth_data.interval)).await;

        let req_body = serde_json::json!({ "device_code": auth_data.device_code });
        let token_res = client
            .post(format!("{}/api/device/token", api_url))
            .json(&req_body)
            .send()
            .await;

        if let Ok(res) = token_res {
            if let Ok(data) = res.json::<DeviceTokenResponse>().await {
                match data {
                    DeviceTokenResponse::Token { access_token } => {
                        println!("Agent successfully authorized!");
                        write_token_secure(TOKEN_PATH, &access_token);
                        return access_token;
                    }
                    DeviceTokenResponse::Error { error } => {
                        if error != "authorization_pending" {
                            eprintln!("Authorization failed: {}", error);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // rustls 0.23 demands the binary install a process-level default
    // CryptoProvider before any TLS handshake. kube-rs (and anything
    // else linking rustls 0.23) panics otherwise. `.ok()` because a
    // second install_default() call would error harmlessly.
    #[cfg(feature = "kube")]
    {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    let args: Vec<String> = std::env::args().collect();
    let is_pair = args.iter().any(|a| a == "--pair" || a == "pair");

    let api_url = std::env::var("SERVER_API_URL")
        .unwrap_or_else(|_| "https://dashboard.example.com".to_string());

    let token = if let Some(t) = read_token() {
        t
    } else if is_pair {
        pair(&api_url).await
    } else {
        eprintln!("No agent token found. Run `shellfleet-agent --pair` to pair this host.");
        std::process::exit(1);
    };

    let wss_url_str = std::env::var("SERVER_WS_URL")
        .unwrap_or_else(|_| "wss://dashboard.example.com/agent/ws".to_string());

    println!("Connecting to server WebSocket...");

    // Build the upgrade request with the bearer token in an
    // `Authorization` header rather than a `?token=` query string.
    // Query strings get logged by reverse proxies, edge CDN access
    // logs, server tracing, crash reports, and operator screenshots
    // — none of which we want to leak the long-lived agent token to.
    // The header is only on the upgrade exchange and is dropped from
    // the persistent WS frames that follow.
    let mut request = match wss_url_str.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to build WS request from SERVER_WS_URL={wss_url_str}: {e}");
            std::process::exit(1);
        }
    };
    let bearer = match format!("Bearer {token}").parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to encode bearer token: {e}");
            std::process::exit(1);
        }
    };
    request.headers_mut().insert(AUTHORIZATION, bearer);

    // Cap the connect. `connect_async` has no built-in timeout, so a hung
    // TCP/TLS connect (server mid-restart that accepts TCP but never finishes
    // the WS handshake, a half-open Cloudflare/Tailscale path, etc.) used to
    // block here FOREVER: the process stayed `active` but permanently offline,
    // never retrying, because the only retry path is exit -> systemd restart
    // and a hung await never reaches the error arm. The idle watchdog can't
    // help either — it only runs once a connection is established. Time the
    // connect out so a stuck attempt fails fast and systemd reconnects us.
    let connect_timeout = Duration::from_secs(30);
    let (ws_stream, _) = match tokio::time::timeout(connect_timeout, connect_async(request)).await {
        Ok(Ok(res)) => res,
        Ok(Err(e)) => {
            eprintln!(
                "Failed to connect to server: {}. Your token might have been revoked.",
                e
            );
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!(
                "Connect timed out after {}s; exiting so systemd reconnects.",
                connect_timeout.as_secs()
            );
            std::process::exit(1);
        }
    };

    println!("WebSocket handshake completed.");

    let (mut write, mut read) = ws_stream.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Send a register message
    let hostname = hostname::get()
        .unwrap_or_else(|_| "unknown-agent".into())
        .to_string_lossy()
        .to_string();

    // Probe each subsystem and advertise what we find. The dashboard
    // hides tabs that aren't represented here, so a host with no docker
    // never has a Docker tab cluttering its view. K8s detection lands
    // in v1 with the kube-rs feature; for now we never advertise it.
    let mut capabilities: Vec<String> = Vec::with_capacity(5);
    // Every agent can run one-shot commands for runbooks (sh -c).
    capabilities.push("exec".into());
    if systemd::systemd_available().await {
        capabilities.push("systemd".into());
    }
    if docker::docker_available().await {
        capabilities.push("docker".into());
        // swarm_role() also re-checks docker, but the redundant probe is
        // cheap and keeps the two answers consistent.
        match docker::swarm_role().await {
            shared::SwarmRole::Manager | shared::SwarmRole::Worker => {
                capabilities.push("swarm".into());
            }
            shared::SwarmRole::NotInSwarm => {}
        }
    }
    #[cfg(feature = "kube")]
    if k8s::k8s_available().await {
        capabilities.push("k8s".into());
    }
    println!("agent capabilities: {capabilities:?}");

    let initial_caps = capabilities.clone();
    let _ = tx.send(Message::Register {
        hostname,
        protocol_version: shared::PROTOCOL_VERSION,
        capabilities,
        metadata: std::collections::HashMap::new(),
    });

    // Re-probe capabilities periodically so late-starting subsystems
    // (e.g. Docker starting after the agent on boot) get picked up
    // without needing a full agent restart.
    let tx_caps = tx.clone();
    tokio::spawn(async move {
        // Probe fast (30s) during the boot window so late-starting subsystems
        // (Docker coming up after the agent) get picked up quickly, then back
        // off to a slow cadence once capabilities have stayed put — steady
        // state rarely changes and each probe shells out. Any change snaps the
        // cadence back to fast.
        const FAST: Duration = Duration::from_secs(30);
        const SLOW: Duration = Duration::from_secs(300);
        const SETTLE_ROUNDS: u32 = 6; // ~3 min of stability before slowing down
        let mut delay = FAST;
        let mut idle_rounds: u32 = 0;
        let mut current = initial_caps;
        loop {
            tokio::time::sleep(delay).await;
            let mut fresh: Vec<String> = Vec::with_capacity(5);
            // Always present (one-shot exec for runbooks). Must mirror the
            // initial capability list so the periodic refresh doesn't drop it.
            fresh.push("exec".into());
            if systemd::systemd_available().await {
                fresh.push("systemd".into());
            }
            if docker::docker_available().await {
                fresh.push("docker".into());
                match docker::swarm_role().await {
                    shared::SwarmRole::Manager | shared::SwarmRole::Worker => {
                        fresh.push("swarm".into());
                    }
                    shared::SwarmRole::NotInSwarm => {}
                }
            }
            #[cfg(feature = "kube")]
            if k8s::k8s_available().await {
                fresh.push("k8s".into());
            }
            if fresh != current {
                println!("capabilities changed: {current:?} -> {fresh:?}");
                let _ = tx_caps.send(Message::CapabilitiesUpdate {
                    capabilities: fresh.clone(),
                });
                current = fresh;
                // Something's in flux — probe fast again until it settles.
                idle_rounds = 0;
                delay = FAST;
            } else {
                idle_rounds = idle_rounds.saturating_add(1);
                if idle_rounds >= SETTLE_ROUNDS {
                    delay = SLOW;
                }
            }
        }
    });

    // If the agent was restarted by systemd/libc/self-package updates,
    // the apt transaction continues in its own transient systemd unit.
    // Resume watching that unit and only report once dpkg is finished.
    if apt::pending_run().is_some() {
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            if let Some((package, success, log, error)) = apt::resume_pending_upgrade().await {
                let _ = tx_clone.send(Message::AptUpgradeResponse {
                    package,
                    success,
                    log,
                    error,
                });
            }
        });
    }

    // Multi-PTY per host: each StartTerminalRequest carries a
    // dashboard-generated session_id and lands as its own entry.
    // Container exec stays singleton (a separate concern, scoped by
    // container_id, identified on the wire by an empty session_id).
    let mut term_sessions: std::collections::HashMap<String, terminal::TerminalSession> =
        std::collections::HashMap::new();
    let mut exec_session: Option<terminal::TerminalSession> = None;
    let log_streams = logs::LogStreams::default();
    let journal_streams = journal::JournalStreams::default();
    let journal_stream_mgr = journal_stream::JournalStreams::default();
    #[cfg(feature = "kube")]
    let k8s_log_streams = k8s_logs::K8sLogStreams::default();
    // Parallel map for k8s exec PTYs. Keyed by the same session_id
    // used for host shells, but stored separately because the
    // teardown shape differs (kube-rs AttachedProcess.join() vs
    // portable-pty Drop).
    #[cfg(feature = "kube")]
    let mut k8s_exec_sessions: std::collections::HashMap<String, k8s_exec::K8sExecSession> =
        std::collections::HashMap::new();
    let health_probes = health::HealthProbes::default();

    // Watchdog: if the WebSocket goes silent for 75s the connection is
    // probably dead at the TCP layer (Cloudflare or the kernel may drop
    // it without delivering an error). Exit so systemd restarts us; the
    // server uses the same window to reap stale agents.
    let idle_timeout = Duration::from_secs(75);
    let mut last_read = tokio::time::Instant::now();
    let mut watchdog = tokio::time::interval(Duration::from_secs(15));
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    watchdog.tick().await;

    // Application-level heartbeat (protocol v18+). Sending a `Ping` Text message
    // every 20s draws a `Pong` Text reply from the server; that round-trip is
    // what `last_read` keys off, so the watchdog can tell a genuinely-alive
    // server from a proxy that's merely keeping the socket warm. 20s gives ~3
    // chances inside the 75s window. The first tick is immediate, so liveness is
    // probed right after connect.
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    'main_loop: loop {
        tokio::select! {
            maybe_msg = read.next() => {
                let msg = match maybe_msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        eprintln!("agent ws read error: {e}");
                        break 'main_loop;
                    }
                    None => {
                        eprintln!("server closed the websocket");
                        break 'main_loop;
                    }
                };
                // The server pings every ~25s and drops the connection after
                // ~50s without a Pong. tokio-tungstenite auto-queues the Pong,
                // but on a split stream it is only sent when the write half is
                // polled — which, on a fully idle agent, never happens. Flush
                // the write half on each Ping so the Pong actually goes out and
                // the idle connection stays up (no reconnect churn).
                if matches!(msg, WsMessage::Ping(_)) {
                    let _ = write.flush().await;
                }
                if let WsMessage::Text(text) = msg {
                    // Liveness is keyed to real server *application* messages
                    // only — NOT WebSocket control frames. A proxy (Cloudflare)
                    // can keep this socket warm with its own Ping/Pong long after
                    // the server has reaped the session; counting those would
                    // strand us half-open forever. Our app-level Ping draws a
                    // `Pong` Text reply every ~20s, so a healthy path always
                    // resets this; a dead server side does not, and the watchdog
                    // below reconnects us.
                    last_read = tokio::time::Instant::now();
                    let parsed = serde_json::from_str::<Message>(&text);
                    if let Err(ref e) = parsed {
                        eprintln!("dropped un-parseable protocol message: {e}");
                    }
                    if let Ok(parsed_msg) = parsed {
                        match parsed_msg {
                            Message::ListServicesRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    match systemd::list_services().await {
                                        Ok(services) => {
                                            let _ = tx_clone.send(Message::ListServicesResponse { services });
                                        }
                                        Err(e) => {
                                            eprintln!("list_services failed: {e}");
                                            let _ = tx_clone.send(Message::ListServicesResponse { services: Vec::new() });
                                        }
                                    }
                                });
                            }
                            Message::SystemStatsRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx_clone.send(stats::snapshot().await);
                                });
                            }
                            Message::RunCommandRequest { request_id, command, timeout_secs } => {
                                // One-shot exec for EE runbooks (gated upstream
                                // by the runbook allow-list + CE ACL).
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let r = exec::run(&command, timeout_secs).await;
                                    let _ = tx_clone.send(Message::RunCommandResponse {
                                        request_id,
                                        exit_code: r.exit_code,
                                        stdout: r.stdout,
                                        stderr: r.stderr,
                                        error: r.error,
                                        truncated: r.truncated,
                                        timed_out: r.timed_out,
                                        duration_ms: r.duration_ms,
                                    });
                                });
                            }
                            Message::DockerImageListRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (available, images, error) = match docker::list_images().await {
                                        Ok(v) => (true, v, None),
                                        Err(e) => (false, Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::DockerImageListResponse {
                                        available,
                                        images,
                                        error,
                                    });
                                });
                            }
                            Message::DockerImageRemoveRequest { id, force } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) =
                                        docker::remove_image(&id, force).await;
                                    let _ = tx_clone.send(Message::DockerImageRemoveResponse {
                                        id,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerImagePullRequest { reference } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) = docker::pull_image(&reference).await;
                                    let _ = tx_clone.send(Message::DockerImagePullResponse {
                                        reference,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            // ----- networks (v12) -----
                            Message::DockerNetworkListRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (available, networks, error) = match docker::list_networks().await {
                                        Ok(v) => (true, v, None),
                                        Err(e) => (false, Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::DockerNetworkListResponse {
                                        available,
                                        networks,
                                        error,
                                    });
                                });
                            }
                            Message::DockerNetworkInspectRequest { id } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, json, error) = docker::inspect_network(&id).await;
                                    let _ = tx_clone.send(Message::DockerNetworkInspectResponse {
                                        id,
                                        success,
                                        json,
                                        error,
                                    });
                                });
                            }
                            Message::DockerNetworkCreateRequest { name, driver, subnet, attachable, internal } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, id, log, error) = docker::create_network(
                                        &name,
                                        &driver,
                                        subnet.as_deref(),
                                        attachable,
                                        internal,
                                    ).await;
                                    let _ = tx_clone.send(Message::DockerNetworkCreateResponse {
                                        name,
                                        success,
                                        id,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerNetworkRemoveRequest { id } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) = docker::remove_network(&id).await;
                                    let _ = tx_clone.send(Message::DockerNetworkRemoveResponse {
                                        id,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            // ----- volumes (v12) -----
                            Message::DockerVolumeListRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (available, volumes, error) = match docker::list_volumes().await {
                                        Ok(v) => (true, v, None),
                                        Err(e) => (false, Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::DockerVolumeListResponse {
                                        available,
                                        volumes,
                                        error,
                                    });
                                });
                            }
                            Message::DockerVolumeInspectRequest { name } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, json, error) = docker::inspect_volume(&name).await;
                                    let _ = tx_clone.send(Message::DockerVolumeInspectResponse {
                                        name,
                                        success,
                                        json,
                                        error,
                                    });
                                });
                            }
                            Message::DockerVolumeRemoveRequest { name, force } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) = docker::remove_volume(&name, force).await;
                                    let _ = tx_clone.send(Message::DockerVolumeRemoveResponse {
                                        name,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerVolumePruneRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, removed, space_reclaimed_bytes, log, error) =
                                        docker::prune_volumes().await;
                                    let _ = tx_clone.send(Message::DockerVolumePruneResponse {
                                        success,
                                        removed,
                                        space_reclaimed_bytes,
                                        log,
                                        error,
                                    });
                                });
                            }
                            // ----- swarm stacks (v12, manager-only) -----
                            Message::SwarmStackListRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    let is_manager = role == shared::SwarmRole::Manager;
                                    if !is_manager {
                                        let _ = tx_clone.send(Message::SwarmStackListResponse {
                                            available: false,
                                            is_manager: false,
                                            stacks: Vec::new(),
                                            error: None,
                                        });
                                        return;
                                    }
                                    let (stacks, err) = match docker::list_stacks().await {
                                        Ok(s) => (s, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::SwarmStackListResponse {
                                        available: err.is_none(),
                                        is_manager: true,
                                        stacks,
                                        error: err,
                                    });
                                });
                            }
                            Message::SwarmStackInspectRequest { name } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    if role != shared::SwarmRole::Manager {
                                        let _ = tx_clone.send(Message::SwarmStackInspectResponse {
                                            name,
                                            success: false,
                                            services: Vec::new(),
                                            tasks: Vec::new(),
                                            log: String::new(),
                                            error: Some("not a swarm manager".to_string()),
                                        });
                                        return;
                                    }
                                    let services = docker::stack_services(&name).await.unwrap_or_default();
                                    let (tasks, err) = match docker::stack_tasks(&name).await {
                                        Ok(t) => (t, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::SwarmStackInspectResponse {
                                        name,
                                        success: err.is_none(),
                                        services,
                                        tasks,
                                        log: String::new(),
                                        error: err,
                                    });
                                });
                            }
                            Message::SwarmStackRemoveRequest { name } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    if role != shared::SwarmRole::Manager {
                                        let _ = tx_clone.send(Message::SwarmStackRemoveResponse {
                                            name,
                                            success: false,
                                            log: String::new(),
                                            error: Some("not a swarm manager".to_string()),
                                        });
                                        return;
                                    }
                                    let (success, log, error) = docker::remove_stack(&name).await;
                                    let _ = tx_clone.send(Message::SwarmStackRemoveResponse {
                                        name,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerListRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    let (containers, error) = match docker::list_containers().await {
                                        Ok(c) => (c, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::DockerListResponse {
                                        available: error.is_none(),
                                        swarm_role: role,
                                        containers,
                                        error,
                                    });
                                });
                            }
                            Message::K8sListPodsRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (pods, error) = match k8s::list_pods().await {
                                        Ok(p) => (p, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (pods, error) = (
                                        Vec::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sListPodsResponse { pods, error });
                                });
                            }
                            Message::K8sListDeploymentsRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (deployments, error) = match k8s::list_deployments().await {
                                        Ok(d) => (d, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (deployments, error) = (
                                        Vec::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sListDeploymentsResponse {
                                        deployments,
                                        error,
                                    });
                                });
                            }
                            Message::K8sListServicesRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (services, error) = match k8s::list_services().await {
                                        Ok(s) => (s, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (services, error) = (
                                        Vec::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sListServicesResponse {
                                        services,
                                        error,
                                    });
                                });
                            }
                            Message::K8sListIngressesRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (ingresses, error) = match k8s::list_ingresses().await {
                                        Ok(i) => (i, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (ingresses, error) = (
                                        Vec::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sListIngressesResponse {
                                        ingresses,
                                        error,
                                    });
                                });
                            }
                            Message::K8sListPvcsRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (pvcs, error) = match k8s::list_pvcs().await {
                                        Ok(p) => (p, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (pvcs, error) = (
                                        Vec::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sListPvcsResponse { pvcs, error });
                                });
                            }
                            Message::K8sListEventsRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (events, error) = match k8s::list_events().await {
                                        Ok(e) => (e, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (events, error) = (
                                        Vec::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sListEventsResponse {
                                        events,
                                        error,
                                    });
                                });
                            }
                            Message::K8sLogsRequest {
                                stream_id,
                                namespace,
                                pod_name,
                                container,
                                tail_lines,
                                follow,
                            } => {
                                #[cfg(feature = "kube")]
                                {
                                    let args = k8s_logs::LogArgs {
                                        stream_id,
                                        namespace,
                                        pod_name,
                                        container,
                                        tail_lines,
                                        follow,
                                    };
                                    k8s_log_streams.start(args, tx.clone()).await;
                                }
                                #[cfg(not(feature = "kube"))]
                                {
                                    let _ = (namespace, pod_name, container, tail_lines, follow);
                                    let _ = tx.send(Message::K8sLogsEnd {
                                        stream_id,
                                        error: Some("agent built without k8s support".into()),
                                    });
                                }
                            }
                            Message::K8sLogsStop { stream_id } => {
                                #[cfg(feature = "kube")]
                                {
                                    k8s_log_streams.stop(&stream_id).await;
                                }
                                #[cfg(not(feature = "kube"))]
                                {
                                    let _ = stream_id;
                                }
                            }
                            Message::K8sExecRequest {
                                session_id,
                                namespace,
                                pod_name,
                                container,
                                command,
                            } => {
                                #[cfg(feature = "kube")]
                                {
                                    // Idempotency: a re-issued session_id supersedes
                                    // the previous attached process so a transient WS
                                    // reconnect on the dashboard doesn't strand a
                                    // stale exec.
                                    if let Some(prev) = k8s_exec_sessions.remove(&session_id) {
                                        prev.abort();
                                    }
                                    let args = k8s_exec::ExecArgs {
                                        session_id: session_id.clone(),
                                        namespace,
                                        pod_name,
                                        container,
                                        command,
                                    };
                                    match k8s_exec::spawn_exec(args, tx.clone()).await {
                                        Ok(s) => {
                                            k8s_exec_sessions.insert(session_id.clone(), s);
                                            let _ = tx.send(Message::K8sExecResponse {
                                                session_id,
                                                success: true,
                                                error: None,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Message::K8sExecResponse {
                                                session_id,
                                                success: false,
                                                error: Some(e),
                                            });
                                        }
                                    }
                                }
                                #[cfg(not(feature = "kube"))]
                                {
                                    let _ = (namespace, pod_name, container, command);
                                    let _ = tx.send(Message::K8sExecResponse {
                                        session_id,
                                        success: false,
                                        error: Some("agent built without k8s support".into()),
                                    });
                                }
                            }
                            Message::K8sApplyRequest { yaml, dry_run, force } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (result, error) = match k8s::apply(&yaml, dry_run, force).await {
                                        Ok(r) => (r, None),
                                        Err(e) => (String::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (result, error) = {
                                        let _ = (yaml, dry_run, force);
                                        (
                                            String::new(),
                                            Some("agent built without k8s support".into()),
                                        )
                                    };
                                    let _ = tx_clone.send(Message::K8sApplyResponse { result, error });
                                });
                            }
                            Message::K8sScaleRequest { kind, namespace, name, replicas } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (success, error) = match k8s::scale(&kind, &namespace, &name, replicas).await {
                                        Ok(()) => (true, None),
                                        Err(e) => (false, Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (success, error) = {
                                        let _ = replicas;
                                        (false, Some("agent built without k8s support".into()))
                                    };
                                    let _ = tx_clone.send(Message::K8sScaleResponse {
                                        kind, namespace, name, success, error,
                                    });
                                });
                            }
                            Message::K8sDeletePodRequest { namespace, name, grace_period_secs } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (success, error) = match k8s::delete_pod(&namespace, &name, grace_period_secs).await {
                                        Ok(()) => (true, None),
                                        Err(e) => (false, Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (success, error) = {
                                        let _ = grace_period_secs;
                                        (false, Some("agent built without k8s support".into()))
                                    };
                                    let _ = tx_clone.send(Message::K8sDeletePodResponse {
                                        namespace, name, success, error,
                                    });
                                });
                            }
                            Message::K8sDescribeRequest { kind, namespace, name } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    #[cfg(feature = "kube")]
                                    let (yaml, error) = match k8s::describe(
                                        &kind,
                                        namespace.as_deref(),
                                        &name,
                                    ).await {
                                        Ok(y) => (y, None),
                                        Err(e) => (String::new(), Some(e)),
                                    };
                                    #[cfg(not(feature = "kube"))]
                                    let (yaml, error) = (
                                        String::new(),
                                        Some("agent built without k8s support".into()),
                                    );
                                    let _ = tx_clone.send(Message::K8sDescribeResponse {
                                        kind,
                                        namespace,
                                        name,
                                        yaml,
                                        error,
                                    });
                                });
                            }
                            Message::SwarmServiceActionRequest { name, action } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    if role != shared::SwarmRole::Manager {
                                        let _ = tx_clone.send(Message::SwarmServiceActionResponse {
                                            name,
                                            success: false,
                                            log: String::new(),
                                            error: Some("not a swarm manager".to_string()),
                                        });
                                        return;
                                    }
                                    let (success, log, error) = docker::run_swarm_action(&name, &action).await;
                                    let _ = tx_clone.send(Message::SwarmServiceActionResponse {
                                        name,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::AptStatusRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let last_update_secs = apt::last_apt_update_secs();
                                    let (upgradable, error) = match apt::list_upgradable().await {
                                        Ok(u) => (u, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::AptStatusResponse {
                                        available: error.is_none(),
                                        upgradable,
                                        last_update_secs,
                                        error,
                                    });
                                });
                            }
                            Message::AptRefreshRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) = apt::refresh().await;
                                    let _ = tx_clone.send(Message::AptRefreshResponse {
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerContainerActionRequest { id, action } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) = docker::run_container_action(&id, action).await;
                                    let _ = tx_clone.send(Message::DockerContainerActionResponse {
                                        id,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerLogsRequest { container_id, tail, follow } => {
                                let streams = log_streams.clone();
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    streams.start(container_id, tail, follow, tx_clone).await;
                                });
                            }
                            Message::DockerLogsStop { container_id } => {
                                let streams = log_streams.clone();
                                tokio::spawn(async move {
                                    streams.stop(&container_id).await;
                                });
                            }
                            Message::JournalLogsRequest { unit, lines, follow } => {
                                let streams = journal_streams.clone();
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    streams.start(unit, lines, follow, tx_clone).await;
                                });
                            }
                            Message::JournalLogsStop { unit } => {
                                let streams = journal_streams.clone();
                                tokio::spawn(async move {
                                    streams.stop(&unit).await;
                                });
                            }
                            Message::JournalStreamRequest {
                                stream_id,
                                units,
                                priority,
                                since,
                                grep,
                                identifier,
                                lines,
                                follow,
                            } => {
                                let mgr = journal_stream_mgr.clone();
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    mgr.start(
                                        journal_stream::StreamArgs {
                                            stream_id,
                                            units,
                                            priority,
                                            since,
                                            grep,
                                            identifier,
                                            lines,
                                            follow,
                                        },
                                        tx_clone,
                                    )
                                    .await;
                                });
                            }
                            Message::JournalStreamStop { stream_id } => {
                                let mgr = journal_stream_mgr.clone();
                                tokio::spawn(async move {
                                    mgr.stop(&stream_id).await;
                                });
                            }
                            Message::HealthProbeSyncRequest { probes } => {
                                let probes_mgr = health_probes.clone();
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    probes_mgr.sync(probes, tx_clone).await;
                                });
                            }
                            Message::BackupRunRequest { id, name, paths, dest, mode } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, archive_path, bytes, log, error) =
                                        backup::run_backup(&name, &paths, &dest, mode).await;
                                    let _ = tx_clone.send(Message::BackupRunResponse {
                                        id,
                                        name,
                                        success,
                                        archive_path,
                                        bytes,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::BackupListArchivesRequest { id, name: _, dest } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, archives, error) =
                                        backup::list_archives(&dest).await;
                                    let _ = tx_clone.send(Message::BackupListArchivesResponse {
                                        id,
                                        success,
                                        archives,
                                        error,
                                    });
                                });
                            }
                            Message::BackupRestoreRequest { id, archive_uri, dest_root } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, log, error) =
                                        backup::restore(&archive_uri, &dest_root).await;
                                    let _ = tx_clone.send(Message::BackupRestoreResponse {
                                        id,
                                        archive_uri,
                                        dest_root,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::SwarmServiceInspectRequest { name } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    if role != shared::SwarmRole::Manager {
                                        let _ = tx_clone.send(Message::SwarmServiceInspectResponse {
                                            name,
                                            success: false,
                                            tasks: Vec::new(),
                                            spec: None,
                                            log: String::new(),
                                            error: Some("not a swarm manager".to_string()),
                                        });
                                        return;
                                    }
                                    let tasks = docker::service_ps(&name).await.unwrap_or_default();
                                    let (spec, error) = match docker::service_inspect(&name).await {
                                        Ok(s) => (Some(s), None),
                                        Err(e) => (None, Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::SwarmServiceInspectResponse {
                                        name,
                                        success: error.is_none(),
                                        tasks,
                                        spec,
                                        log: String::new(),
                                        error,
                                    });
                                });
                            }
                            Message::SwarmStackDeployRequest { stack_name, compose_yaml, prune } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    if role != shared::SwarmRole::Manager {
                                        let _ = tx_clone.send(Message::SwarmStackDeployResponse {
                                            stack_name,
                                            success: false,
                                            log: String::new(),
                                            error: Some("not a swarm manager".to_string()),
                                        });
                                        return;
                                    }
                                    let (success, log, error) = docker::stack_deploy(&stack_name, &compose_yaml, prune).await;
                                    let _ = tx_clone.send(Message::SwarmStackDeployResponse {
                                        stack_name,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::DockerCreateContainerRequest { spec } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (success, container_id, log, error) = deploy::create_container(&spec).await;
                                    let _ = tx_clone.send(Message::DockerCreateContainerResponse {
                                        success,
                                        container_id,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::SwarmCreateServiceRequest { spec } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    if role != shared::SwarmRole::Manager {
                                        let _ = tx_clone.send(Message::SwarmCreateServiceResponse {
                                            success: false,
                                            service_id: None,
                                            log: String::new(),
                                            error: Some("not a swarm manager".to_string()),
                                        });
                                        return;
                                    }
                                    let (success, service_id, log, error) = deploy::create_service(&spec).await;
                                    let _ = tx_clone.send(Message::SwarmCreateServiceResponse {
                                        success,
                                        service_id,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::AptUpgradeRequest { package } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let pkg_for_resp = package.clone();
                                    let (success, log, error) = apt::upgrade(package).await;
                                    let _ = tx_clone.send(Message::AptUpgradeResponse {
                                        package: pkg_for_resp,
                                        success,
                                        log,
                                        error,
                                    });
                                });
                            }
                            Message::SwarmListRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let role = docker::swarm_role().await;
                                    let is_manager = role == shared::SwarmRole::Manager;
                                    if !is_manager {
                                        let _ = tx_clone.send(Message::SwarmListResponse {
                                            available: false,
                                            is_manager: false,
                                            services: Vec::new(),
                                            nodes: Vec::new(),
                                            error: None,
                                        });
                                        return;
                                    }
                                    let (services, svc_err) = match docker::list_swarm_services().await {
                                        Ok(s) => (s, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    let (nodes, node_err) = match docker::list_swarm_nodes().await {
                                        Ok(n) => (n, None),
                                        Err(e) => (Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::SwarmListResponse {
                                        available: true,
                                        is_manager: true,
                                        services,
                                        nodes,
                                        error: svc_err.or(node_err),
                                    });
                                });
                            }
                            Message::ControlServiceRequest { name, action } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let success = systemd::control_service(&name, &action).await.is_ok();
                                    let _ = tx_clone.send(Message::ControlServiceResponse {
                                        name,
                                        success,
                                        error: if success { None } else { Some("Failed".into()) },
                                    });
                                });
                            }
                            Message::StartTerminalRequest { session_id } => {
                                if session_id.is_empty() {
                                    eprintln!("StartTerminalRequest with empty session_id rejected");
                                } else if term_sessions.contains_key(&session_id) {
                                    // Idempotent: re-issuing the same id (e.g. after a
                                    // transient WS reconnect on the dashboard) is a no-op
                                    // rather than tearing down the live PTY.
                                    println!("Terminal session_id={} already exists", session_id);
                                } else {
                                    match terminal::spawn_terminal(session_id.clone(), tx.clone()) {
                                        Ok(session) => {
                                            term_sessions.insert(session_id.clone(), session);
                                            println!("Terminal spawned session_id={}", session_id);
                                        }
                                        Err(e) => eprintln!("Failed to spawn terminal: {}", e),
                                    }
                                }
                            }
                            Message::StopTerminalRequest { session_id } => {
                                // Dropping the TerminalSession closes the
                                // tx_input sender; the write thread exits;
                                // the child gets EOF on stdin and reaps.
                                term_sessions.remove(&session_id);
                                #[cfg(feature = "kube")]
                                if let Some(s) = k8s_exec_sessions.remove(&session_id) {
                                    s.abort();
                                }
                            }
                            Message::TerminalData { session_id, data } => {
                                // Empty session_id is the container-exec singleton;
                                // anything else is a host PTY OR a k8s exec PTY
                                // keyed by id (parallel maps, host wins on collision).
                                if session_id.is_empty() {
                                    if let Some(session) = &exec_session {
                                        let _ = session.tx_input.send(data);
                                    }
                                } else if let Some(session) = term_sessions.get(&session_id) {
                                    let _ = session.tx_input.send(data);
                                } else {
                                    #[cfg(feature = "kube")]
                                    if let Some(session) = k8s_exec_sessions.get(&session_id) {
                                        let _ = session.tx_input.send(data);
                                    }
                                }
                            }
                            Message::TerminalResize { session_id, cols, rows } => {
                                if session_id.is_empty() {
                                    if let Some(session) = &exec_session {
                                        let _ = session.tx_resize.send((cols, rows));
                                    }
                                } else if let Some(session) = term_sessions.get(&session_id) {
                                    let _ = session.tx_resize.send((cols, rows));
                                } else {
                                    #[cfg(feature = "kube")]
                                    if let Some(session) = k8s_exec_sessions.get(&session_id) {
                                        let _ = session.tx_resize.send((cols, rows));
                                    }
                                }
                            }
                            Message::DockerExecStartRequest { container_id, shell } => {
                                let tx_clone = tx.clone();
                                // One exec session at a time — replace any existing.
                                if let Some(prev) = exec_session.take() {
                                    drop(prev);
                                }
                                let cid = container_id.clone();
                                let shell_arg = shell.clone();
                                match terminal::spawn_docker_exec(&cid, &shell_arg, tx.clone()) {
                                    Ok(s) => {
                                        exec_session = Some(s);
                                        let _ = tx_clone.send(Message::DockerExecStartResponse {
                                            container_id: cid,
                                            success: true,
                                            error: None,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = tx_clone.send(Message::DockerExecStartResponse {
                                            container_id: cid,
                                            success: false,
                                            error: Some(e),
                                        });
                                    }
                                }
                            }
                            Message::DockerExecStopRequest => {
                                // Drop the session — the writer thread exits when
                                // tx_input is dropped, the read thread exits on EOF,
                                // and the docker exec child reaps.
                                exec_session = None;
                            }
                            Message::DockerSystemPruneRequest { dry_run, prune_volumes } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let outcome = if dry_run {
                                        docker::system_prune_preview(prune_volumes).await
                                    } else {
                                        docker::system_prune_apply(prune_volumes).await
                                    };
                                    let _ = tx_clone.send(Message::DockerSystemPruneResponse {
                                        dry_run,
                                        success: outcome.success,
                                        reclaimed_bytes: outcome.reclaimed_bytes,
                                        containers_removed: outcome.containers_removed,
                                        images_removed: outcome.images_removed,
                                        networks_removed: outcome.networks_removed,
                                        volumes_removed: outcome.volumes_removed,
                                        log: outcome.log,
                                        error: outcome.error,
                                    });
                                });
                            }
                            Message::DockerStatsRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let (available, snapshots, error) = match docker::container_stats().await {
                                        Ok(v) => (true, v, None),
                                        Err(e) => (false, Vec::new(), Some(e)),
                                    };
                                    let _ = tx_clone.send(Message::DockerStatsResponse {
                                        available,
                                        snapshots,
                                        error,
                                    });
                                });
                            }
                            Message::ReadConfigRequest { path } => {
                                // `check_read` canonicalises and re-runs
                                // the deny/allow match on the resolved
                                // location, so a symlink under an allowed
                                // prefix that points at /etc/shadow is
                                // rejected here before std::fs::read.
                                let resp = match config::check_read(&path) {
                                    Err(e) => Message::ReadConfigResponse {
                                        path: path.clone(),
                                        content: "".to_string(),
                                        error: Some(e.to_string()),
                                    },
                                    Ok(safe_path) => match std::fs::read_to_string(&safe_path) {
                                        Ok(c) => Message::ReadConfigResponse {
                                            path: path.clone(),
                                            content: c,
                                            error: None,
                                        },
                                        Err(e) => Message::ReadConfigResponse {
                                            path: path.clone(),
                                            content: "".to_string(),
                                            error: Some(e.to_string()),
                                        },
                                    },
                                };
                                let _ = tx.send(resp);
                            }
                            Message::WriteConfigRequest { path, content } => {
                                // `check_write` canonicalises the parent
                                // dir; `write_no_follow` opens the final
                                // path with O_NOFOLLOW so a symlink at
                                // the leaf component can't redirect the
                                // write off-target.
                                let resp = match config::check_write(&path) {
                                    Err(e) => Message::WriteConfigResponse {
                                        path: path.clone(),
                                        success: false,
                                        error: Some(e.to_string()),
                                    },
                                    Ok((parent, name)) => {
                                        match config::write_no_follow(
                                            &parent,
                                            &name,
                                            content.as_bytes(),
                                        ) {
                                            Ok(_) => Message::WriteConfigResponse {
                                                path: path.clone(),
                                                success: true,
                                                error: None,
                                            },
                                            Err(e) => Message::WriteConfigResponse {
                                                path: path.clone(),
                                                success: false,
                                                error: Some(e.to_string()),
                                            },
                                        }
                                    }
                                };
                                let _ = tx.send(resp);
                            }
                            Message::DriftSnapshotRequest { snapshot_id, categories, config_paths } => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    let mut packages = Vec::new();
                                    let mut services = Vec::new();
                                    let mut containers = Vec::new();
                                    let mut configs = Vec::new();

                                    let cats: std::collections::HashSet<&str> =
                                        if categories.is_empty() {
                                            ["packages", "services", "containers", "configs"].into_iter().collect()
                                        } else {
                                            categories.iter().map(|s| s.as_str()).collect()
                                        };

                                    if cats.contains("packages") {
                                        if let Ok(output) = tokio::process::Command::new("dpkg-query")
                                            .args(["-W", "-f", "${Package}\t${Version}\t${Status}\n"])
                                            .output()
                                            .await
                                        {
                                            let stdout = String::from_utf8_lossy(&output.stdout);
                                            for line in stdout.lines() {
                                                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                                                if parts.len() >= 2 {
                                                    packages.push(shared::DriftPackage {
                                                        name: parts[0].to_string(),
                                                        version: parts[1].to_string(),
                                                        status: parts.get(2).unwrap_or(&"").to_string(),
                                                    });
                                                }
                                            }
                                        }
                                    }

                                    if cats.contains("services") {
                                        if let Ok(output) = tokio::process::Command::new("systemctl")
                                            .args(["list-units", "--type=service", "--no-pager", "--no-legend", "--plain"])
                                            .output()
                                            .await
                                        {
                                            let stdout = String::from_utf8_lossy(&output.stdout);
                                            for line in stdout.lines() {
                                                let parts: Vec<&str> = line.split_whitespace().collect();
                                                if parts.len() >= 4 {
                                                    services.push(shared::DriftService {
                                                        name: parts[0].to_string(),
                                                        active: parts[2].to_string(),
                                                        enabled: parts[3].to_string(),
                                                    });
                                                }
                                            }
                                        }
                                    }

                                    if cats.contains("containers") {
                                        if let Ok(output) = tokio::process::Command::new("docker")
                                            .args(["ps", "-a", "--format", "{{.Names}}\t{{.Image}}\t{{.State}}"])
                                            .output()
                                            .await
                                        {
                                            let stdout = String::from_utf8_lossy(&output.stdout);
                                            for line in stdout.lines() {
                                                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                                                if parts.len() >= 3 {
                                                    containers.push(shared::DriftContainer {
                                                        name: parts[0].to_string(),
                                                        image: parts[1].to_string(),
                                                        state: parts[2].to_string(),
                                                    });
                                                }
                                            }
                                        }
                                    }

                                    if cats.contains("configs") {
                                        for path in &config_paths {
                                            if let Some(config) = drift_config_fingerprint(path).await {
                                                configs.push(config);
                                            }
                                        }
                                    }

                                    let _ = tx_clone.send(Message::DriftSnapshotResponse {
                                        snapshot_id,
                                        packages,
                                        services,
                                        containers,
                                        configs,
                                        error: None,
                                    });
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(msg) = rx.recv() => {
                if let Ok(text) = serde_json::to_string(&msg) {
                    if write.send(WsMessage::Text(text.into())).await.is_err() {
                        eprintln!("agent ws write failed, exiting");
                        break 'main_loop;
                    }
                }
            }
            _ = heartbeat.tick() => {
                // Route the Ping through the same outgoing channel as everything
                // else so it serializes with real responses on the write half.
                let _ = tx.send(Message::Ping);
            }
            _ = watchdog.tick() => {
                if last_read.elapsed() > idle_timeout {
                    eprintln!(
                        "agent ws idle for {}s, exiting so systemd reconnects",
                        last_read.elapsed().as_secs()
                    );
                    break 'main_loop;
                }
            }
            else => {
                break 'main_loop;
            }
        }
    }

    println!("Connection closed.");
    // Exiting non-zero makes systemd re-run with a fresh connection sooner
    // than waiting for an unresponsive websocket loop.
    std::process::exit(1);
}
