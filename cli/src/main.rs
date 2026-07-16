mod app;
mod client;
mod credentials;
mod fleet;
mod ui;

use app::{App, Mode, View};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::Duration};

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
    if args.get(1).map(String::as_str) == Some("login") {
        return credentials::login(args.get(2)).await;
    }
    if args.get(1).map(String::as_str) == Some("logout") {
        return credentials::logout();
    }

    let connection = credentials::connection()?;
    let mut incoming = client::connect(connection);
    let mut app = App::new();

    enable_raw_mode().map_err(|error| error.to_string())?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|error| error.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|error| error.to_string())?;
    let result = cockpit(&mut terminal, &mut app, &mut incoming).await;
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

async fn cockpit(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    incoming: &mut tokio::sync::mpsc::UnboundedReceiver<client::ClientEvent>,
) -> Result<(), String> {
    loop {
        while let Ok(message) = incoming.try_recv() {
            match message {
                client::ClientEvent::Fleet(fleet) => {
                    app.replace_fleet(fleet);
                }
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
            }
        }
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|error| error.to_string())?;
        if event::poll(Duration::from_millis(40)).map_err(|error| error.to_string())?
            && let Event::Key(key) = event::read().map_err(|error| error.to_string())?
        {
            match handle_key(app, key)? {
                KeyOutcome::Continue => {}
                KeyOutcome::Quit => break,
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
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<KeyOutcome, String> {
    match app.mode {
        Mode::Fleet => match key.code {
            KeyCode::Char('q') => return Ok(KeyOutcome::Quit),
            KeyCode::Up => app.select_previous(),
            KeyCode::Down => app.select_next(),
            KeyCode::Char('1') => app.view = View::Overview,
            KeyCode::Char('2') => app.view = View::Services,
            KeyCode::Char('3') => app.view = View::Containers,
            KeyCode::Char('4') => app.view = View::Activity,
            KeyCode::Tab => app.view = next_view(app.view),
            KeyCode::BackTab => app.view = previous_view(app.view),
            KeyCode::Char('/') => app.mode = Mode::Filter,
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.command.clear();
                app.mode = Mode::Palette;
            }
            KeyCode::Char('?') => app.mode = Mode::Help,
            _ => {}
        },
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
        View::Activity => View::Overview,
    }
}

fn previous_view(view: View) -> View {
    match view {
        View::Overview => View::Activity,
        View::Services => View::Overview,
        View::Containers => View::Services,
        View::Activity => View::Containers,
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
        "quit" | "q" => return Ok(KeyOutcome::Quit),
        "" => return Ok(KeyOutcome::Continue),
        _ => {
            app.status = format!("Unknown cockpit command: {command}");
            return Ok(KeyOutcome::Continue);
        }
    };
    Ok(KeyOutcome::Continue)
}

fn help_text(command: &str) -> String {
    format!(
        "ShellFleet Fleet Cockpit\n\nUSAGE:\n  {command}                              Open the durable fleet cockpit\n  {command} login <dashboard-url>        Authorize a CLI session in the browser\n  {command} logout                       Remove the local CLI session\n\nCOCKPIT:\n  1-4 / Tab                  Switch fleet views\n  /                          Filter hosts and capabilities\n  Ctrl-P                     Open the command palette\n  ?                          Show context help\n\nSECURITY:\n  The CLI session is purpose-bound to read-only fleet and event APIs.\n  Open the web dashboard when you need an interactive host terminal.\n\nENVIRONMENT:\n  SHELLFLEET_URL              Dashboard origin used by login and fleet reads\n  SHELLFLEET_AUTH_TOKEN       Override the saved CLI session (automation only)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;

    fn app() -> App {
        App::new()
    }

    fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Result<KeyOutcome, String> {
        handle_key(app, KeyEvent::new(code, modifiers))
    }

    #[test]
    fn browse_keys_navigate_views_and_context_help() {
        let mut app = app();
        assert_eq!(
            press(&mut app, KeyCode::Char('5'), KeyModifiers::NONE).unwrap(),
            KeyOutcome::Continue
        );
        assert_eq!(app.view, View::Overview);
        press(&mut app, KeyCode::Char('?'), KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Help);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Fleet);
    }

    #[test]
    fn root_shortcuts_are_not_part_of_the_cli() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'), KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Fleet);
        let outcome = press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE).unwrap();
        assert_eq!(outcome, KeyOutcome::Continue);
        assert_eq!(app.mode, Mode::Fleet);
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
        assert!(!help.contains("keygen"));
        assert!(!help.contains("Approver"));
        assert!(!help.contains("Privileged"));
        assert!(!help.contains("SHELLFLEET_WS_URL"));
        assert!(!help.contains("trusted fleet TUI"));
    }
}
