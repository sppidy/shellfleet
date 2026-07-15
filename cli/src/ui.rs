use crate::app::{App, Mode, fingerprint};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " SHELLFLEET ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Operator Cockpit",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("VERIFIED ROOT", Style::default().fg(Color::Green)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(title, vertical[0]);

    if matches!(app.mode, Mode::Terminal) {
        let terminal = Paragraph::new(app.output.join("\n"))
            .style(Style::default().fg(Color::Gray).bg(Color::Black))
            .block(
                Block::default()
                    .title(" Encrypted root terminal • Esc closes view ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(terminal, vertical[1]);
    } else {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(40)])
            .split(vertical[1]);
        let agents = app.agents.iter().enumerate().map(|(index, agent)| {
            let marker = if index == app.selected { "▶" } else { " " };
            let style = if index == app.selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(format!("{marker} {agent}")).style(style)
        });
        frame.render_widget(
            List::new(agents).block(Block::default().title(" Fleet ").borders(Borders::ALL)),
            horizontal[0],
        );

        let body = match app.mode {
            Mode::Command => format!(
                "ROOT COMMAND\n\n$ {}_\n\nThe host gate will canonicalize this as /bin/sh -lc and require your encrypted approver key.",
                app.command
            ),
            Mode::Review => review(app),
            _ => {
                let output = if app.output.is_empty() {
                    "No trusted-operation output yet.".into()
                } else {
                    app.output.join("\n")
                };
                format!("HOST OVERVIEW\n\n{output}")
            }
        };
        let border = if matches!(app.mode, Mode::Review) {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        frame.render_widget(
            Paragraph::new(body)
                .block(
                    Block::default()
                        .title(" Verified transaction workspace ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border)),
                )
                .wrap(Wrap { trim: false }),
            horizontal[1],
        );
    }

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" quit  "),
        Span::styled(" ↑↓ ", Style::default().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" host  "),
        Span::styled(" : ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" root command  "),
        Span::styled(" r ", Style::default().fg(Color::White).bg(Color::Red)),
        Span::raw(" root PTY  "),
        Span::styled(" p/a ", Style::default().fg(Color::Black).bg(Color::Green)),
        Span::raw(" pin/approve  "),
        Span::styled(&app.status, Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, vertical[2]);
}

fn review(app: &App) -> String {
    let Some(pending) = &app.pending else {
        return "No verified transaction.".into();
    };
    let Some(challenge) = &pending.challenge else {
        return format!(
            "WAITING FOR HOST\n\nRequest: {}\nOperation: {:?}",
            pending.request_id, pending.operation
        );
    };
    let manifest = &challenge.manifest;
    format!(
        "ROOT-EQUIVALENT TRANSACTION\n\nHost: {}\nIdentity: {}\nRequest: {}\nOperation: {:?}\nPolicy: {}\nCreated: {}\nExpires: {}\nNonce: {}…\n\nIdentity state: {}\n\nPress a only if every field is expected.",
        manifest.host_id,
        fingerprint(&challenge.host_identity_public),
        manifest.request_id,
        manifest.operation,
        manifest.policy_version,
        manifest.created_at,
        manifest.expires_at,
        hex(&manifest.nonce[..8]),
        if pending.pinned {
            "PIN MATCH + SIGNATURE VALID"
        } else {
            "UNPAIRED — APPROVAL BLOCKED"
        },
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
