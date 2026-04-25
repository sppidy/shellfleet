mod apt;
mod deploy;
mod docker;
mod health;
mod journal;
mod logs;
mod stats;
mod systemd;
mod terminal;

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
                        // Try to save to /etc/ first, fallback to local
                        if std::fs::write(token_path, &access_token).is_err() {
                            let _ = std::fs::write("agent-token.txt", &access_token);
                        }
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
    let _ = tx.send(Message::Register {
        hostname,
        protocol_version: shared::PROTOCOL_VERSION,
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

    let mut term_session: Option<terminal::TerminalSession> = None;
    let log_streams = logs::LogStreams::default();
    let journal_streams = journal::JournalStreams::default();
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
                            Message::HealthProbeSyncRequest { probes } => {
                                let probes_mgr = health_probes.clone();
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    probes_mgr.sync(probes, tx_clone).await;
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
                            Message::StartTerminalRequest => {
                                match terminal::spawn_terminal(tx.clone()) {
                                    Ok(session) => {
                                        term_session = Some(session);
                                        println!("Terminal spawned");
                                    }
                                    Err(e) => eprintln!("Failed to spawn terminal: {}", e),
                                }
                            }
                            Message::TerminalData { data } => {
                                if let Some(session) = &term_session {
                                    let _ = session.tx_input.send(data);
                                }
                            }
                            Message::TerminalResize { cols, rows } => {
                                if let Some(session) = &term_session {
                                    let _ = session.tx_resize.send((cols, rows));
                                }
                            }
                            Message::ReadConfigRequest { path } => {
                                let content = std::fs::read_to_string(&path);
                                let resp = match content {
                                    Ok(c) => Message::ReadConfigResponse {
                                        path: path.clone(),
                                        content: c,
                                        error: None,
                                    },
                                    Err(e) => Message::ReadConfigResponse {
                                        path: path.clone(),
                                        content: "".to_string(),
                                        error: Some(e.to_string()),
                                    }
                                };
                                let _ = tx.send(resp);
                            }
                            Message::WriteConfigRequest { path, content } => {
                                let res = std::fs::write(&path, content);
                                let resp = match res {
                                    Ok(_) => Message::WriteConfigResponse {
                                        path: path.clone(),
                                        success: true,
                                        error: None,
                                    },
                                    Err(e) => Message::WriteConfigResponse {
                                        path: path.clone(),
                                        success: false,
                                        error: Some(e.to_string()),
                                    }
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
