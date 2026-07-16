mod app;
mod client;
mod credentials;
mod fleet;
mod identity;
mod session;
mod ui;

use app::{App, LinkState, Mode, View};
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
        let command = args
            .first()
            .and_then(|value| std::path::Path::new(value).file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("shellfleetctl");
        println!("{}", help_text(command));
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

    let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
    let pins = PathBuf::from(home).join(".config/shellfleet/host-pins.json");
    let connection = credentials::connection()?;
    let (outgoing, mut incoming) = client::connect(connection);
    let mut app = App::new(pins);
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
    incoming: &mut tokio::sync::mpsc::UnboundedReceiver<client::ClientEvent>,
) -> Result<(), String> {
    loop {
        while let Ok(message) = incoming.try_recv() {
            match message {
                client::ClientEvent::Fleet(fleet) => {
                    app.replace_fleet(fleet);
                }
                client::ClientEvent::WebSocket(message) => handle_server(app, *message)?,
                client::ClientEvent::DataState(state) => {
                    if let client::TransportState::Degraded(reason) = &state {
                        app.status = reason.clone();
                    }
                    app.set_data_state(link_state(state));
                }
                client::ClientEvent::EventState(state) => {
                    if let client::TransportState::Degraded(reason) = &state {
                        app.status = reason.clone();
                    }
                    app.set_event_state(link_state(state));
                }
                client::ClientEvent::Core(event) => {
                    app.record_core_event(event);
                }
                client::ClientEvent::WebSocketState(state) => {
                    if let client::TransportState::Degraded(reason) = &state {
                        app.status = reason.clone();
                    }
                    app.set_websocket_state(link_state(state));
                }
            }
        }
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|error| error.to_string())?;
        if event::poll(Duration::from_millis(40)).map_err(|error| error.to_string())?
            && let Event::Key(key) = event::read().map_err(|error| error.to_string())?
        {
            match handle_key(app, key, outgoing)? {
                KeyOutcome::Continue => {}
                KeyOutcome::Quit => break,
                KeyOutcome::BeginPrivileged(operation) => {
                    begin_privileged(terminal, app, outgoing, operation)?;
                }
            }
        }
    }
    Ok(())
}

fn link_state(state: client::TransportState) -> app::LinkState {
    match state {
        client::TransportState::Connecting => app::LinkState::Connecting,
        client::TransportState::Live => app::LinkState::Live,
        client::TransportState::Degraded(_) => app::LinkState::Degraded,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum KeyOutcome {
    Continue,
    Quit,
    BeginPrivileged(TrustedOperation),
}

fn handle_server(app: &mut App, message: UiMessage) -> Result<(), String> {
    match message {
        // Durable REST inventory is the fleet source of truth. The WebSocket
        // list is only a legacy compatibility signal and must never clear or
        // reorder the read plane during an interactive reconnect.
        UiMessage::ListAgentsResponse { .. } => {}
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
) -> Result<KeyOutcome, String> {
    match app.mode {
        Mode::Fleet => match key.code {
            KeyCode::Char('q') => return Ok(KeyOutcome::Quit),
            KeyCode::Up => app.select_previous(),
            KeyCode::Down => app.select_next(),
            KeyCode::Char('1') => app.view = View::Overview,
            KeyCode::Char('2') => app.view = View::Services,
            KeyCode::Char('3') => app.view = View::Containers,
            KeyCode::Char('4') => app.view = View::Activity,
            KeyCode::Char('5') => app.view = View::Privileged,
            KeyCode::Tab => app.view = next_view(app.view),
            KeyCode::BackTab => app.view = previous_view(app.view),
            KeyCode::Char('/') => app.mode = Mode::Filter,
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.command.clear();
                app.mode = Mode::Palette;
            }
            KeyCode::Char('?') => app.mode = Mode::Help,
            KeyCode::Char(':') if app.view == View::Privileged => {
                app.command.clear();
                app.mode = Mode::Command;
            }
            KeyCode::Char('r') if app.view == View::Privileged => {
                return Ok(KeyOutcome::BeginPrivileged(TrustedOperation::RootPty {
                    shell: "/bin/bash".into(),
                    ttl_secs: 900,
                    cols: 120,
                    rows: 40,
                }));
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
                return Ok(KeyOutcome::BeginPrivileged(TrustedOperation::RootCommand {
                    program: "/bin/sh".into(),
                    args: vec!["-lc".into(), command],
                    timeout_secs: 300,
                }));
            }
            KeyCode::Char(character) => app.command.push(character),
            _ => {}
        },
        Mode::Review => match key.code {
            KeyCode::Esc => {
                if let Some(pending) = app.pending.as_ref() {
                    let payload = shared::trusted::encode_client(&TrustedClientFrame::Close)?;
                    let message = Message::TrustedOperationClient {
                        request_id: pending.request_id.clone(),
                        start: false,
                        close: true,
                        payload,
                    };
                    send_to_selected(app, outgoing, message)?;
                }
                app.pending = None;
                app.mode = Mode::Fleet;
                app.status = "Privileged transaction cancelled.".into();
            }
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
        Mode::Filter => match key.code {
            KeyCode::Esc | KeyCode::Enter => app.mode = Mode::Fleet,
            KeyCode::Backspace => {
                app.filter.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.filter.push(character);
            }
            _ => {}
        },
        Mode::Palette => match key.code {
            KeyCode::Esc => {
                app.command.clear();
                app.mode = Mode::Fleet;
            }
            KeyCode::Backspace => {
                app.command.pop();
            }
            KeyCode::Enter => return apply_palette(app),
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.command.push(character);
            }
            _ => {}
        },
        Mode::Help => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('?') {
                app.mode = Mode::Fleet;
            }
        }
    }
    Ok(KeyOutcome::Continue)
}

fn next_view(view: View) -> View {
    match view {
        View::Overview => View::Services,
        View::Services => View::Containers,
        View::Containers => View::Activity,
        View::Activity => View::Privileged,
        View::Privileged => View::Overview,
    }
}

fn previous_view(view: View) -> View {
    match view {
        View::Overview => View::Privileged,
        View::Services => View::Overview,
        View::Containers => View::Services,
        View::Activity => View::Containers,
        View::Privileged => View::Activity,
    }
}

fn apply_palette(app: &mut App) -> Result<KeyOutcome, String> {
    let command = std::mem::take(&mut app.command).trim().to_ascii_lowercase();
    app.mode = Mode::Fleet;
    app.view = match command.as_str() {
        "overview" | "1" => View::Overview,
        "services" | "2" => View::Services,
        "containers" | "3" => View::Containers,
        "activity" | "4" => View::Activity,
        "privileged" | "5" => View::Privileged,
        "quit" | "q" => return Ok(KeyOutcome::Quit),
        "" => return Ok(KeyOutcome::Continue),
        _ => {
            app.status = format!("Unknown cockpit command: {command}");
            return Ok(KeyOutcome::Continue);
        }
    };
    Ok(KeyOutcome::Continue)
}

fn begin_privileged(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    outgoing: &tokio::sync::mpsc::UnboundedSender<UiMessage>,
    operation: TrustedOperation,
) -> Result<(), String> {
    if app.websocket_state != LinkState::Live {
        app.mode = Mode::Fleet;
        app.status = "Interactive channel unavailable; fleet reads remain available.".into();
        return Ok(());
    }
    if !app.approver_unlocked()
        && let Err(error) = unlock_approver(terminal, app)
    {
        app.mode = Mode::Fleet;
        app.status = error;
        return Ok(());
    }
    let message = match app.begin(operation) {
        Ok(message) => message,
        Err(error) => {
            app.mode = Mode::Fleet;
            app.status = error;
            return Ok(());
        }
    };
    send_to_selected(app, outgoing, message)
}

fn unlock_approver(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    disable_raw_mode().map_err(|error| error.to_string())?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).map_err(|error| error.to_string())?;
    terminal.show_cursor().map_err(|error| error.to_string())?;

    let result = (|| {
        let key_path = identity::default_key_path()?;
        if !key_path.is_file() {
            return Err(format!(
                "No approver key at {}; run `shellfleet keygen` before privileged access.",
                key_path.display()
            ));
        }
        let passphrase = key_passphrase("Approver key passphrase: ")?;
        identity::load(&key_path, &passphrase)
    })();

    let restore = (|| {
        execute!(terminal.backend_mut(), EnterAlternateScreen)
            .map_err(|error| error.to_string())?;
        enable_raw_mode().map_err(|error| error.to_string())?;
        terminal.hide_cursor().map_err(|error| error.to_string())?;
        terminal.clear().map_err(|error| error.to_string())
    })();
    restore?;
    app.unlock_approver(result?);
    app.status = "Approver key unlocked for this process only.".into();
    Ok(())
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

fn help_text(command: &str) -> String {
    format!(
        "ShellFleet Fleet Cockpit\n\nUSAGE:\n  {command}                              Open the durable fleet cockpit\n  {command} login <dashboard-url>        Authorize a CLI session in the browser\n  {command} logout                       Remove the local CLI session\n  {command} keygen [PATH]                Create an encrypted approver key\n\nCOCKPIT:\n  1-5 / Tab                  Switch fleet views\n  /                          Filter hosts and capabilities\n  Ctrl-P                     Open the command palette\n  ?                          Show context help\n\nSECURITY:\n  Fleet reads use a purpose-bound session. Approver key is requested only\n  when you start an explicit action from the Privileged view.\n\nENVIRONMENT:\n  SHELLFLEET_URL              Dashboard origin used by login and fleet reads\n  SHELLFLEET_WS_URL           Override the saved interactive /ui/ws URL\n  SHELLFLEET_AUTH_TOKEN       Override the saved CLI session (automation only)\n  SHELLFLEET_KEY_PASSPHRASE   Optional non-interactive key passphrase"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;

    fn app() -> App {
        App::new(PathBuf::from("/nonexistent/pins"))
    }

    fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Result<KeyOutcome, String> {
        let (outgoing, _) = tokio::sync::mpsc::unbounded_channel();
        handle_key(app, KeyEvent::new(code, modifiers), &outgoing)
    }

    #[test]
    fn browse_keys_navigate_views_and_context_help() {
        let mut app = app();
        assert_eq!(
            press(&mut app, KeyCode::Char('5'), KeyModifiers::NONE).unwrap(),
            KeyOutcome::Continue
        );
        assert_eq!(app.view, View::Privileged);
        press(&mut app, KeyCode::Char('?'), KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Help);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Fleet);
    }

    #[test]
    fn root_shortcuts_are_scoped_to_the_privileged_view() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'), KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Fleet);

        app.view = View::Privileged;
        let outcome = press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE).unwrap();
        assert!(matches!(
            outcome,
            KeyOutcome::BeginPrivileged(TrustedOperation::RootPty { .. })
        ));
        assert!(!app.approver_unlocked());
    }

    #[test]
    fn filter_and_palette_are_keyboard_discoverable() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Filter);
        for character in "dock".chars() {
            press(&mut app, KeyCode::Char(character), KeyModifiers::NONE).unwrap();
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.filter, "dock");
        assert_eq!(app.mode, Mode::Fleet);

        press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL).unwrap();
        assert_eq!(app.mode, Mode::Palette);
        for character in "services".chars() {
            press(&mut app, KeyCode::Char(character), KeyModifiers::NONE).unwrap();
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.view, View::Services);
        assert_eq!(app.mode, Mode::Fleet);
    }

    #[test]
    fn help_copy_describes_the_fleet_first_product() {
        let help = help_text("shellfleetctl");
        assert!(help.contains("shellfleetctl"));
        assert!(help.contains("durable fleet cockpit"));
        assert!(help.contains("Approver key is requested only"));
        assert!(!help.contains("trusted fleet TUI"));
    }
}
