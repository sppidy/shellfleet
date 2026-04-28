mod apt;
mod backup;
mod config;
mod deploy;
mod docker;
mod health;
mod journal;
mod journal_stream;
mod logs;
mod stats;
mod systemd;
mod terminal;

#[cfg(feature = "kube")]
mod k8s;

/// Write the bearer token to disk with mode 0600. Tries the operator
/// path first; on permission failure (e.g. the agent isn't running as
/// root and `/etc/sys-manager` doesn't exist) falls back to a CWD-
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

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use shared::Message;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use url::Url;

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

async fn get_agent_token(api_url: &str) -> String {
    // 1. Check local file
    let token_path = "/etc/sys-manager/agent-token.txt";
    if let Ok(token) = std::fs::read_to_string(token_path) {
        if !token.trim().is_empty() {
            return token.trim().to_string();
        }
    }
    // Fallback for Windows/dev
    if let Ok(token) = std::fs::read_to_string("agent-token.txt") {
        if !token.trim().is_empty() {
            return token.trim().to_string();
        }
    }

    // 2. Perform Device Auth Flow
    let client = reqwest::Client::new();
    
    println!("Requesting device authorization...");
    let auth_res = client.post(format!("{}/api/device/request", api_url))
        .send().await.expect("Failed to contact server");
        
    let auth_data: DeviceAuthResponse = auth_res.json().await.expect("Failed to parse response");

    println!("=======================================================");
    println!("To authenticate this agent, please visit:");
    println!("{}", auth_data.verification_uri);
    println!("And enter the code: {}", auth_data.user_code);
    println!("=======================================================");

    loop {
        tokio::time::sleep(Duration::from_secs(auth_data.interval)).await;
        
        let req_body = serde_json::json!({
            "device_code": auth_data.device_code
        });
        
        let token_res = client.post(format!("{}/api/device/token", api_url))
            .json(&req_body)
            .send().await;
            
        if let Ok(res) = token_res {
            if let Ok(data) = res.json::<DeviceTokenResponse>().await {
                match data {
                    DeviceTokenResponse::Token { access_token } => {
                        println!("Agent successfully authorized!");
                        // The token grants WebSocket connect privileges,
                        // so the on-disk file is mode 0600 — readable
                        // only by the agent's user (typically root via
                        // the systemd unit). Write+chmod is split into
                        // two steps because std::fs::write doesn't
                        // expose a perm-on-create knob.
                        write_token_secure(token_path, &access_token);
                        return access_token;
                    }
                    DeviceTokenResponse::Error { error } => {
                        if error == "authorization_pending" {
                            // Continue polling
                        } else {
                            panic!("Authorization failed: {}", error);
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

    let api_url = std::env::var("SERVER_API_URL").unwrap_or_else(|_| "https://dashboard.example.com".to_string());
    
    // Perform Tailscale-like auth
    let token = get_agent_token(&api_url).await;

    let wss_url_str = std::env::var("SERVER_WS_URL").unwrap_or_else(|_| "wss://dashboard.example.com/agent/ws".to_string());
    let url_with_auth = format!("{}?token={}", wss_url_str, token);
    let url = Url::parse(&url_with_auth).unwrap();
    
    println!("Connecting to server WebSocket...");

    let (ws_stream, _) = match connect_async(url.as_str()).await {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Failed to connect to server: {}. Your token might have been revoked.", e);
            std::process::exit(1);
        }
    };
    
    println!("WebSocket handshake completed.");

    let (mut write, mut read) = ws_stream.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Send a register message
    let hostname = hostname::get().unwrap_or_else(|_| "unknown-agent".into()).to_string_lossy().to_string();

    // Probe each subsystem and advertise what we find. The dashboard
    // hides tabs that aren't represented here, so a host with no docker
    // never has a Docker tab cluttering its view. K8s detection lands
    // in v1 with the kube-rs feature; for now we never advertise it.
    let mut capabilities: Vec<String> = Vec::with_capacity(4);
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

    let _ = tx.send(Message::Register {
        hostname,
        protocol_version: shared::PROTOCOL_VERSION,
        capabilities,
    });

    // If we exited mid-apt-run last time (e.g. libc/systemd self-upgrade
    // killed us), synthesise the AptUpgradeResponse so the server-side
    // scheduler doesn't sit on `last_status="running"` forever.
    if let Some(pending) = apt::take_pending_run() {
        let mut log = pending.log;
        log.push_str("\n[agent restarted during upgrade; this run was interrupted]\n");
        let _ = tx.send(Message::AptUpgradeResponse {
            package: pending.package,
            success: false,
            log,
            error: Some("agent restarted during upgrade".into()),
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
                last_read = tokio::time::Instant::now();
                if let WsMessage::Text(text) = msg {
                    if let Ok(parsed_msg) = serde_json::from_str::<Message>(&text) {
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
                            }
                            Message::TerminalData { session_id, data } => {
                                // Empty session_id is the container-exec singleton;
                                // anything else is a host PTY keyed by id.
                                if session_id.is_empty() {
                                    if let Some(session) = &exec_session {
                                        let _ = session.tx_input.send(data);
                                    }
                                } else if let Some(session) = term_sessions.get(&session_id) {
                                    let _ = session.tx_input.send(data);
                                }
                            }
                            Message::TerminalResize { session_id, cols, rows } => {
                                if session_id.is_empty() {
                                    if let Some(session) = &exec_session {
                                        let _ = session.tx_resize.send((cols, rows));
                                    }
                                } else if let Some(session) = term_sessions.get(&session_id) {
                                    let _ = session.tx_resize.send((cols, rows));
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
                                let resp = match config::check(&path) {
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
                                let resp = match config::check(&path) {
                                    Err(e) => Message::WriteConfigResponse {
                                        path: path.clone(),
                                        success: false,
                                        error: Some(e.to_string()),
                                    },
                                    Ok(safe_path) => match std::fs::write(&safe_path, content) {
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
                                    },
                                };
                                let _ = tx.send(resp);
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
