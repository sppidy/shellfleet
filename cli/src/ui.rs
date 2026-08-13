use crate::{
    app::{App, Mode, View},
    fleet::{docker, services, swarm, system},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, TableState, Tabs, Wrap,
    },
};
use shared::{SwarmRole, fleet::ConnectionStatus};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;

pub fn draw(frame: &mut Frame, app: &App) {
    let bands = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(frame.area());
    draw_header(frame, app, bands[0]);
    draw_tabs(frame, app, bands[1]);
    match app.view {
        View::Overview => draw_overview(frame, app, bands[2]),
        View::Services => draw_services(frame, app, bands[2]),
        View::Containers => draw_containers(frame, app, bands[2]),
        View::Activity => draw_activity(frame, app, bands[2]),
    }
    draw_footer(frame, app, bands[3]);
    match app.mode {
        Mode::Help => draw_help(frame),
        Mode::Palette => draw_palette(frame, app),
        Mode::Filter => draw_filter(frame, app),
        _ => {}
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let label = app.connection_label();
    let state_color = match label {
        "LIVE" => Color::Green,
        "DEGRADED" | "STALE" => Color::Yellow,
        _ => Color::Red,
    };
    let total = app.fleet.hosts.len();
    let online = app.online_count();
    let line = Line::from(vec![
        Span::styled(
            " SHELLFLEET ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " fleet cockpit ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{label} "), Style::default().fg(state_color)),
        Span::styled(
            format!("{online}/{total} online"),
            Style::default().fg(Color::Gray),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles = ["1 Overview", "2 Services", "3 Containers", "4 Activity"];
    let selected = match app.view {
        View::Overview => 0,
        View::Services => 1,
        View::Containers => 2,
        View::Activity => 3,
    };
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .divider(" │ ")
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(tabs, area);
}

fn draw_overview(frame: &mut Frame, app: &App, area: Rect) {
    if app.fleet.hosts.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "Connecting to the durable Fleet API…\n\nLast-known hosts remain visible during live transport interruptions.",
            )
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().title(" Fleet ").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let filter = app.filter.to_ascii_lowercase();
    let hosts = app
        .fleet
        .hosts
        .iter()
        .filter(|host| {
            filter.is_empty()
                || host.hostname.to_ascii_lowercase().contains(&filter)
                || host
                    .capabilities
                    .iter()
                    .any(|capability| capability.to_ascii_lowercase().contains(&filter))
        })
        .collect::<Vec<_>>();
    let rows = hosts.iter().map(|host| {
        let stats = system(host);
        let docker = docker(host);
        let role = docker
            .as_ref()
            .map(|snapshot| match snapshot.swarm_role {
                SwarmRole::Manager => "MGR",
                SwarmRole::Worker => "WRK",
                SwarmRole::NotInSwarm => "—",
            })
            .unwrap_or("—");
        let memory = stats.as_ref().map_or_else(
            || "—".into(),
            |stats| {
                percent(
                    stats.mem_total_kb - stats.mem_available_kb,
                    stats.mem_total_kb,
                )
            },
        );
        let disk = stats.as_ref().map_or_else(
            || "—".into(),
            |stats| percent(stats.root_disk_used_kb, stats.root_disk_total_kb),
        );
        let caps = host
            .capabilities
            .iter()
            .filter(|capability| {
                matches!(capability.as_str(), "systemd" | "docker" | "swarm" | "k8s")
            })
            .map(|capability| capability.to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(" ");
        let style = if host.agent_id == app.selected_agent().unwrap_or_default() {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else if host.status == ConnectionStatus::Offline {
            Style::default().fg(MUTED)
        } else {
            Style::default().fg(Color::Gray)
        };
        Row::new(vec![
            Cell::from(if host.status == ConnectionStatus::Online {
                "online"
            } else {
                "offline"
            }),
            Cell::from(
                stats
                    .as_ref()
                    .map(|value| value.hostname.clone())
                    .unwrap_or_else(|| host.hostname.clone()),
            ),
            Cell::from(role),
            Cell::from(
                stats
                    .as_ref()
                    .map_or_else(|| "—".into(), |value| format!("{:.2}", value.load_1)),
            ),
            Cell::from(memory),
            Cell::from(disk),
            Cell::from(caps),
        ])
        .style(style)
    });
    let header = Row::new([
        "STATUS",
        "HOST",
        "ROLE",
        "LOAD",
        "MEM",
        "DISK",
        "CAPABILITIES",
    ])
    .style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(17),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .title(" Fleet health ")
            .borders(Borders::ALL),
    );
    let selected = hosts
        .iter()
        .position(|host| Some(host.agent_id.as_str()) == app.selected_agent());
    let mut state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_services(frame: &mut Frame, app: &App, area: Rect) {
    let Some(host) = app.selected_host() else {
        return draw_empty(frame, area, "Services", "Select a host from Overview.");
    };
    let Some(mut entries) = services(host) else {
        return draw_empty(
            frame,
            area,
            "Services",
            "No durable systemd snapshot is available.",
        );
    };
    entries.sort_by(|left, right| {
        (left.active_state != "failed")
            .cmp(&(right.active_state != "failed"))
            .then_with(|| left.name.cmp(&right.name))
    });
    let rows = entries.into_iter().map(|service| {
        let style = if service.active_state == "failed" {
            Style::default().fg(Color::Red)
        } else if service.active_state == "active" {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Gray)
        };
        Row::new(vec![
            service.active_state,
            service.name,
            service.status,
            service.description,
        ])
        .style(style)
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(24),
                Constraint::Length(12),
                Constraint::Min(16),
            ],
        )
        .header(
            Row::new(["ACTIVE", "UNIT", "STATE", "DESCRIPTION"]).style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1)
        .block(
            Block::default()
                .title(format!(" Services · {} ", host.hostname))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn draw_containers(frame: &mut Frame, app: &App, area: Rect) {
    let Some(host) = app.selected_host() else {
        return draw_empty(frame, area, "Containers", "Select a host from Overview.");
    };
    let local = docker(host);
    let cluster = swarm(host);
    if local.is_none() && cluster.is_none() {
        return draw_empty(
            frame,
            area,
            "Containers",
            "No durable Docker snapshot is available.",
        );
    }
    let docker_state = if local.as_ref().is_some_and(|snapshot| snapshot.available) {
        "available"
    } else {
        "unavailable"
    };
    let swarm_state = if cluster.as_ref().is_some_and(|snapshot| snapshot.is_manager) {
        "manager"
    } else {
        "local"
    };
    let local_rows = local
        .into_iter()
        .flat_map(|snapshot| snapshot.containers)
        .map(|container| {
            Row::new(vec![
                "container".to_string(),
                container.names,
                container.state,
                container.image,
                container.ports,
            ])
        });
    let swarm_rows = cluster
        .into_iter()
        .flat_map(|snapshot| snapshot.services)
        .map(|service| {
            Row::new(vec![
                "swarm".to_string(),
                service.name,
                service.replicas,
                service.image,
                service.ports,
            ])
        });
    frame.render_widget(
        Table::new(
            local_rows.chain(swarm_rows),
            [
                Constraint::Length(10),
                Constraint::Length(18),
                Constraint::Length(10),
                Constraint::Length(22),
                Constraint::Min(10),
            ],
        )
        .header(
            Row::new(["SCOPE", "NAME", "STATE", "IMAGE", "PORTS"]).style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1)
        .block(
            Block::default()
                .title(format!(
                    " Containers · {} · Docker {docker_state} · Swarm {swarm_state} ",
                    host.hostname
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn draw_activity(frame: &mut Frame, app: &App, area: Rect) {
    let items = app.activity.iter().map(|entry| {
        let host = entry
            .agent_id
            .as_deref()
            .and_then(|agent| app.fleet.hosts.iter().find(|host| host.agent_id == agent))
            .map(|host| host.hostname.as_str())
            .unwrap_or("fleet");
        ListItem::new(format!(
            "{}  {:<22}  {}",
            entry.observed_at, host, entry.summary
        ))
    });
    let list = if app.activity.is_empty() {
        List::new(vec![ListItem::new("Waiting for durable fleet events…")])
    } else {
        List::new(items.collect::<Vec<_>>())
    };
    frame.render_widget(
        list.style(Style::default().fg(Color::Gray))
            .block(Block::default().title(" Activity ").borders(Borders::ALL)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hints = match app.mode {
        Mode::Filter => "type to filter  Enter apply  Esc cancel",
        Mode::Palette => "type command  Enter select  Esc cancel",
        Mode::Help => "Esc close help",
        Mode::Fleet => "↑↓ host  1-4 view  / filter  Ctrl-P menu  ? help  q quit",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(hints, Style::default().fg(Color::Gray))),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(MUTED)),
                Span::styled(&app.status, Style::default().fg(Color::Yellow)),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame.area(), 64, 16);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(
            "Keyboard help\n\n1-4 / Tab   switch view\n↑↓          select host\n/           filter fleet\nCtrl-P      command palette\n?           open this help\nq           quit from browse mode\nEsc         close the current mode",
        )
        .block(Block::default().borders(Borders::ALL).title(" Keyboard help "))
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_palette(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 60, 10);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!(
            "> {}_\n\noverview  services  containers  activity  quit",
            app.command
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Command palette "),
        ),
        area,
    );
}

fn draw_filter(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 56, 5);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!("Host or capability: {}_", app.filter)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Fleet filter "),
        ),
        area,
    );
}

fn draw_empty(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(Color::Gray))
            .block(
                Block::default()
                    .title(format!(" {title} "))
                    .borders(Borders::ALL),
            ),
        area,
    );
}

fn percent(used: u64, total: u64) -> String {
    used.saturating_mul(100)
        .checked_div(total)
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "—".into())
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::draw;
    use crate::app::{App, LinkState, Mode, View};
    use ratatui::{Terminal, backend::TestBackend};
    use shared::fleet::{ConnectionStatus, FleetHost, FleetResponse, SnapshotValue};
    use std::collections::BTreeMap;

    fn app() -> App {
        let mut app = App::new();
        app.replace_fleet(FleetResponse {
            generated_at: 100,
            offline_after_seconds: 45,
            hosts: vec![FleetHost {
                agent_id: "agent-1234567890".into(),
                hostname: "worker-a".into(),
                status: ConnectionStatus::Online,
                protocol_version: 19,
                capabilities: vec![
                    "trusted-root".into(),
                    "systemd".into(),
                    "docker".into(),
                    "swarm".into(),
                ],
                metadata: BTreeMap::new(),
                first_seen_at: 1,
                last_seen_at: 100,
                disconnected_at: None,
                system: Some(SnapshotValue {
                    observed_at: 100,
                    value: serde_json::json!({
                        "type": "SystemStatsResponse",
                        "payload": {
                            "hostname": "worker-a", "kernel": "6.8.0", "uptime_secs": 7200,
                            "cpu_count": 4, "load_1": 0.5, "load_5": 0.4, "load_15": 0.3,
                            "mem_total_kb": 1000, "mem_available_kb": 400,
                            "swap_total_kb": 0, "swap_free_kb": 0,
                            "root_disk_total_kb": 2000, "root_disk_used_kb": 500
                        }
                    }),
                }),
                services: Some(SnapshotValue {
                    observed_at: 100,
                    value: serde_json::json!({
                        "type": "ListServicesResponse",
                        "payload": {"services": [{
                            "name": "docker.service", "description": "Docker Engine",
                            "status": "running", "active_state": "active"
                        }]}
                    }),
                }),
                docker: Some(SnapshotValue {
                    observed_at: 100,
                    value: serde_json::json!({
                        "type": "DockerListResponse",
                        "payload": {"available": true, "swarm_role": "manager", "containers": [{
                            "id": "abc", "names": "web", "image": "nginx:stable",
                            "state": "running", "status": "Up", "ports": "80/tcp"
                        }], "error": null}
                    }),
                }),
                swarm: Some(SnapshotValue {
                    observed_at: 100,
                    value: serde_json::json!({
                        "type": "SwarmListResponse",
                        "payload": {"available": true, "is_manager": true, "services": [{
                            "id": "svc", "name": "web-stack", "mode": "replicated",
                            "replicas": "1/1", "image": "nginx:stable", "ports": "*:80"
                        }], "nodes": [], "error": null}
                    }),
                }),
            }],
        });
        app.set_data_state(LinkState::Live);
        app.set_event_state(LinkState::Live);
        app
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn overview_is_useful_and_unprivileged_at_80x24() {
        let screen = render(&app(), 80, 24);
        assert!(screen.contains("SHELLFLEET"));
        assert!(screen.contains("1 Overview"));
        assert!(screen.contains("4 Activity"));
        assert!(!screen.contains("Privileged"));
        assert!(screen.contains("worker-a"));
        assert!(screen.contains("DOCKER"));
        assert!(screen.contains("LIVE"));
        assert!(screen.contains("Loading durable fleet data"));
        assert!(!screen.contains("VERIFIED ROOT"));
        assert!(!screen.contains("agent-1234567890"));
    }

    #[test]
    fn service_and_container_views_render_typed_snapshots() {
        let mut app = app();
        app.view = View::Services;
        assert!(render(&app, 80, 24).contains("docker.service"));
        app.view = View::Containers;
        let screen = render(&app, 80, 24);
        assert!(screen.contains("nginx:stable"));
        assert!(screen.contains("web-stack"));
    }

    #[test]
    fn help_overlay_fits_the_supported_viewport() {
        let mut app = app();
        app.mode = Mode::Help;
        let screen = render(&app, 80, 24);
        assert!(screen.contains("Keyboard help"));
        assert!(screen.contains("Ctrl-P"));
        assert!(screen.contains("Esc"));
    }
}
