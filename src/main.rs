mod app;
mod client;
mod credentials;
mod identity;
mod session;
mod ui;

use app::{App, Mode};
use base64::Engine;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use shared::{
    Message, UiMessage,
    trusted::{TrustedClientFrame, TrustedHostFrame, TrustedOperation},
};
use std::{io, path::PathBuf, time::Duration};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("shellfleet: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if matches!(
        args.get(1).map(String::as_str),
        Some("--help" | "-h" | "help")
    ) {
        println!(
            "ShellFleet Operator Cockpit\n\nUSAGE:\n  shellfleet                              Open the trusted fleet TUI\n  shellfleet login <dashboard-url>        Approve a browser-free CLI session\n  shellfleet logout                       Remove the local CLI session\n  shellfleet keygen [PATH]                Create an encrypted Ed25519 approver key\n\nENVIRONMENT:\n  SHELLFLEET_URL              Dashboard URL used by `login` when no argument is given\n  SHELLFLEET_WS_URL           Override the saved Dashboard /ui/ws URL\n  SHELLFLEET_AUTH_TOKEN       Override the saved CLI session (automation only)\n  SHELLFLEET_KEY_PASSPHRASE   Optional non-interactive key passphrase"
        );
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("keygen") {
        let path = args
            .get(2)
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(identity::default_key_path)?;
        let passphrase = key_passphrase("New approver key passphrase: ")?;
        let key = identity::keygen(&path, &passphrase)?;
        let public =
            base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());
        println!("Encrypted approver key: {}", path.display());
        println!("Approver public key: {public}");
        println!("Enroll locally on each host as root:");
        println!("  shellfleet-approval-gate --enroll-approver operator {public}");
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("login") {
        return credentials::login(args.get(2)).await;
    }
    if args.get(1).map(String::as_str) == Some("logout") {
        return credentials::logout();
    }

    let key_path = identity::default_key_path()?;
    let signer = identity::load(&key_path, &key_passphrase("Approver key passphrase: ")?)?;
    let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
    let pins = PathBuf::from(home).join(".config/shellfleet/host-pins.json");
    let (url, token) = credentials::connection()?;
    let (outgoing, mut incoming) = client::connect(&url, &token).await?;
    let mut app = App::new(signer, pins);
    let _ = outgoing.send(UiMessage::ListAgentsRequest);

    enable_raw_mode().map_err(|error| error.to_string())?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|error| error.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|error| error.to_string())?;
    let result = cockpit(&mut terminal, &mut app, &outgoing, &mut incoming).await;
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

async fn cockpit(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    outgoing: &tokio::sync::mpsc::UnboundedSender<UiMessage>,
    incoming: &mut tokio::sync::mpsc::UnboundedReceiver<UiMessage>,
) -> Result<(), String> {
    loop {
        while let Ok(message) = incoming.try_recv() {
            handle_server(app, message)?;
        }
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|error| error.to_string())?;
        if event::poll(Duration::from_millis(40)).map_err(|error| error.to_string())?
            && let Event::Key(key) = event::read().map_err(|error| error.to_string())?
            && handle_key(app, key, outgoing)?
        {
            break;
        }
    }
    Ok(())
}

fn handle_server(app: &mut App, message: UiMessage) -> Result<(), String> {
    match message {
        UiMessage::ListAgentsResponse { agents, .. } => {
            app.agents = agents;
            app.agents.sort();
            app.selected = app.selected.min(app.agents.len().saturating_sub(1));
        }
        UiMessage::PermissionDenied { reason, .. } => app.status = reason,
        UiMessage::AgentMessage {
            agent_id,
            message:
                Message::TrustedOperationHost {
                    request_id,
                    payload,
                    ..
                },
        } => {
            let expected = app
                .pending
                .as_ref()
                .map(|pending| (pending.agent.as_str(), pending.request_id.as_str()));
            if expected != Some((agent_id.as_str(), request_id.as_str())) {
                return Err("trusted response was routed from the wrong host/request".into());
            }
            match shared::trusted::decode_host(&payload)? {
                TrustedHostFrame::Challenge(challenge) => app.challenge(&agent_id, *challenge)?,
                frame => app.host_frame(frame)?,
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_key(
    app: &mut App,
    key: KeyEvent,
    outgoing: &tokio::sync::mpsc::UnboundedSender<UiMessage>,
) -> Result<bool, String> {
    match app.mode {
        Mode::Fleet => match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Up => app.selected = app.selected.saturating_sub(1),
            KeyCode::Down => {
                app.selected = (app.selected + 1).min(app.agents.len().saturating_sub(1));
            }
            KeyCode::Char(':') => {
                app.command.clear();
                app.mode = Mode::Command;
            }
            KeyCode::Char('r') => {
                let message = app.begin(TrustedOperation::RootPty {
                    shell: "/bin/bash".into(),
                    ttl_secs: 900,
                    cols: 120,
                    rows: 40,
                })?;
                send_to_selected(app, outgoing, message)?;
            }
            _ => {}
        },
        Mode::Command => match key.code {
            KeyCode::Esc => app.mode = Mode::Fleet,
            KeyCode::Backspace => {
                app.command.pop();
            }
            KeyCode::Enter if !app.command.trim().is_empty() => {
                let command = std::mem::take(&mut app.command);
                let message = app.begin(TrustedOperation::RootCommand {
                    program: "/bin/sh".into(),
                    args: vec!["-lc".into(), command],
                    timeout_secs: 300,
                })?;
                send_to_selected(app, outgoing, message)?;
            }
            KeyCode::Char(character) => app.command.push(character),
            _ => {}
        },
        Mode::Review => match key.code {
            KeyCode::Esc => app.mode = Mode::Fleet,
            KeyCode::Char('p') => {
                let agent = app
                    .pending
                    .as_ref()
                    .map(|pending| pending.agent.clone())
                    .ok_or("no pending transaction")?;
                app.pin_current(&agent)?;
            }
            KeyCode::Char('a') => {
                let message = app.approve()?;
                send_to_selected(app, outgoing, message)?;
            }
            _ => {}
        },
        Mode::Terminal => {
            if key.code == KeyCode::Esc {
                let pending = app.pending.as_ref().ok_or("no root session")?;
                let payload = shared::trusted::encode_client(&TrustedClientFrame::Close)?;
                let message = Message::TrustedOperationClient {
                    request_id: pending.request_id.clone(),
                    start: false,
                    close: true,
                    payload,
                };
                send_to_selected(app, outgoing, message)?;
                app.mode = Mode::Fleet;
            } else if let Some(bytes) = terminal_key_bytes(key) {
                let message = app.encrypted_input(&bytes)?;
                send_to_selected(app, outgoing, message)?;
            }
        }
    }
    Ok(false)
}

fn send_to_selected(
    app: &App,
    outgoing: &tokio::sync::mpsc::UnboundedSender<UiMessage>,
    message: Message,
) -> Result<(), String> {
    let agent_id = app
        .pending
        .as_ref()
        .map(|pending| pending.agent.clone())
        .or_else(|| app.selected_agent().map(String::from))
        .ok_or("no selected agent")?;
    outgoing
        .send(UiMessage::SendToAgent { agent_id, message })
        .map_err(|_| "Operator Cockpit connection closed".into())
}

fn terminal_key_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    match (key.code, key.modifiers) {
        (KeyCode::Char(character), KeyModifiers::CONTROL) if character.is_ascii_alphabetic() => {
            Some(vec![(character.to_ascii_lowercase() as u8) - b'a' + 1])
        }
        (KeyCode::Char(character), _) => Some(character.to_string().into_bytes()),
        (KeyCode::Enter, _) => Some(vec![b'\n']),
        (KeyCode::Backspace, _) => Some(vec![0x7f]),
        (KeyCode::Tab, _) => Some(vec![b'\t']),
        (KeyCode::Left, _) => Some(b"\x1b[D".to_vec()),
        (KeyCode::Right, _) => Some(b"\x1b[C".to_vec()),
        (KeyCode::Up, _) => Some(b"\x1b[A".to_vec()),
        (KeyCode::Down, _) => Some(b"\x1b[B".to_vec()),
        _ => None,
    }
}

fn key_passphrase(prompt: &str) -> Result<String, String> {
    std::env::var("SHELLFLEET_KEY_PASSPHRASE")
        .or_else(|_| rpassword::prompt_password(prompt))
        .map_err(|error| error.to_string())
}
