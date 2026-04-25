mod systemd;
mod terminal;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
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

    let mut term_session: Option<terminal::TerminalSession> = None;

    loop {
        tokio::select! {
            Some(msg) = read.next() => {
                if let Ok(WsMessage::Text(text)) = msg {
                    if let Ok(parsed_msg) = serde_json::from_str::<Message>(&text) {
                        match parsed_msg {
                            Message::ListServicesRequest => {
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    if let Ok(services) = systemd::list_services().await {
                                        let _ = tx_clone.send(Message::ListServicesResponse { services });
                                    }
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
                        break;
                    }
                }
            }
            else => {
                break;
            }
        }
    }
    
    println!("Connection closed.");
}
