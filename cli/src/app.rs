use shared::fleet::{ConnectionStatus, CoreEvent, CoreEventKind, FleetHost, FleetResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fleet,
    Filter,
    Palette,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Overview,
    Services,
    Containers,
    Activity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    Live,
    Degraded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEntry {
    pub id: u64,
    pub observed_at: i64,
    pub agent_id: Option<String>,
    pub summary: String,
}

pub struct App {
    pub agents: Vec<String>,
    pub selected: usize,
    pub fleet: FleetResponse,
    pub view: View,
    pub mode: Mode,
    pub command: String,
    pub filter: String,
    pub status: String,
    pub activity: Vec<ActivityEntry>,
    pub data_state: LinkState,
    pub event_state: LinkState,
}

impl App {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            selected: 0,
            fleet: FleetResponse {
                generated_at: 0,
                offline_after_seconds: 45,
                hosts: Vec::new(),
            },
            view: View::Overview,
            mode: Mode::Fleet,
            command: String::new(),
            filter: String::new(),
            status: "Loading durable fleet data…".into(),
            activity: Vec::new(),
            data_state: LinkState::Connecting,
            event_state: LinkState::Connecting,
        }
    }

    pub fn replace_fleet(&mut self, mut fleet: FleetResponse) {
        let selected = self.selected_agent().map(str::to_owned);
        fleet.hosts.sort_by(|left, right| {
            left.hostname
                .to_ascii_lowercase()
                .cmp(&right.hostname.to_ascii_lowercase())
                .then_with(|| left.agent_id.cmp(&right.agent_id))
        });
        self.agents = fleet
            .hosts
            .iter()
            .map(|host| host.agent_id.clone())
            .collect();
        self.fleet = fleet;
        self.selected = selected
            .and_then(|agent| self.agents.iter().position(|item| item == &agent))
            .unwrap_or(0)
            .min(self.agents.len().saturating_sub(1));
    }

    pub fn selected_host(&self) -> Option<&FleetHost> {
        let agent = self.agents.get(self.selected)?;
        self.fleet.hosts.iter().find(|host| &host.agent_id == agent)
    }

    pub fn selected_agent(&self) -> Option<&str> {
        self.selected_host().map(|host| host.agent_id.as_str())
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1).min(self.agents.len().saturating_sub(1));
    }

    pub fn set_data_state(&mut self, state: LinkState) {
        if !(self.data_state == LinkState::Live && state == LinkState::Connecting) {
            self.data_state = state;
        }
    }

    pub fn set_event_state(&mut self, state: LinkState) {
        self.event_state = state;
    }

    pub fn connection_label(&self) -> &'static str {
        match (self.data_state, self.event_state) {
            (LinkState::Live, LinkState::Live) => "LIVE",
            (LinkState::Live, _) => "DEGRADED",
            (LinkState::Connecting, _) if self.fleet.hosts.is_empty() => "CONNECTING",
            _ if !self.fleet.hosts.is_empty() => "STALE",
            _ => "OFFLINE",
        }
    }

    pub fn record_core_event(&mut self, event: CoreEvent) {
        let summary = match event.kind {
            CoreEventKind::HostConnected => "Host connected",
            CoreEventKind::HostDisconnected => "Host disconnected",
            CoreEventKind::HostUpdated => "Host snapshot updated",
            CoreEventKind::ResyncRequired => "Fleet resync required",
        };
        self.activity.insert(
            0,
            ActivityEntry {
                id: event.id,
                observed_at: event.observed_at,
                agent_id: event.agent_id,
                summary: summary.into(),
            },
        );
        self.activity.truncate(100);
    }

    pub fn online_count(&self) -> usize {
        self.fleet
            .hosts
            .iter()
            .filter(|host| host.status == ConnectionStatus::Online)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::fleet::{ConnectionStatus, CoreEvent, CoreEventKind, FleetHost, FleetResponse};
    use std::collections::BTreeMap;

    fn fleet(hosts: &[(&str, &str)]) -> FleetResponse {
        FleetResponse {
            generated_at: 20,
            offline_after_seconds: 45,
            hosts: hosts
                .iter()
                .map(|(agent_id, hostname)| FleetHost {
                    agent_id: (*agent_id).into(),
                    hostname: (*hostname).into(),
                    status: ConnectionStatus::Online,
                    protocol_version: 19,
                    capabilities: vec!["systemd".into()],
                    metadata: BTreeMap::new(),
                    first_seen_at: 1,
                    last_seen_at: 20,
                    disconnected_at: None,
                    system: None,
                    services: None,
                    docker: None,
                    swarm: None,
                })
                .collect(),
        }
    }

    #[test]
    fn app_starts_in_overview() {
        let app = App::new();
        assert_eq!(app.view, View::Overview);
    }

    #[test]
    fn fleet_replacement_preserves_agent_selection() {
        let mut app = App::new();
        app.replace_fleet(fleet(&[("agent-a", "host-a"), ("agent-b", "host-b")]));
        app.select_next();
        assert_eq!(app.selected_host().unwrap().hostname, "host-b");

        app.replace_fleet(fleet(&[("agent-b", "host-b"), ("agent-a", "host-a")]));
        assert_eq!(app.selected_host().unwrap().hostname, "host-b");
    }

    #[test]
    fn fleet_stays_available_when_events_are_degraded() {
        let mut app = App::new();
        app.set_data_state(LinkState::Live);
        app.set_event_state(LinkState::Degraded);
        assert_eq!(app.connection_label(), "DEGRADED");
    }

    #[test]
    fn background_refresh_does_not_downgrade_last_good_data() {
        let mut app = App::new();
        app.set_data_state(LinkState::Live);
        app.set_data_state(LinkState::Connecting);
        assert_eq!(app.data_state, LinkState::Live);
    }

    #[test]
    fn core_events_become_bounded_human_readable_activity() {
        let mut app = App::new();
        app.record_core_event(CoreEvent {
            id: 7,
            kind: CoreEventKind::HostDisconnected,
            agent_id: Some("agent-a".into()),
            observed_at: 99,
        });
        assert_eq!(app.activity.len(), 1);
        assert_eq!(app.activity[0].summary, "Host disconnected");
    }
}
