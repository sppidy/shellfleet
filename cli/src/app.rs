use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};
use shared::{
    fleet::{ConnectionStatus, CoreEvent, CoreEventKind, FleetHost, FleetResponse},
    trusted::{
        SignedTrustedManifest, TrustedClientFrame, TrustedHostFrame, TrustedOperation,
        TrustedPlaintext,
    },
};
use std::{collections::BTreeMap, path::PathBuf};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::session::ClientTransport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fleet,
    Command,
    Review,
    Terminal,
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
    Privileged,
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

pub struct Pending {
    pub agent: String,
    pub request_id: String,
    pub operation: TrustedOperation,
    ephemeral: StaticSecret,
    pub challenge: Option<SignedTrustedManifest>,
    pub pinned: bool,
    pub transport: Option<ClientTransport>,
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
    pub output: Vec<String>,
    pub activity: Vec<ActivityEntry>,
    pub data_state: LinkState,
    pub event_state: LinkState,
    pub websocket_state: LinkState,
    pub pending: Option<Pending>,
    signer: Option<SigningKey>,
    pins_path: PathBuf,
    pins: BTreeMap<String, String>,
}

impl App {
    pub fn new(pins_path: PathBuf) -> Self {
        let pins = std::fs::read(&pins_path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default();
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
            output: Vec::new(),
            activity: Vec::new(),
            data_state: LinkState::Connecting,
            event_state: LinkState::Connecting,
            websocket_state: LinkState::Connecting,
            pending: None,
            signer: None,
            pins_path,
            pins,
        }
    }

    pub fn approver_unlocked(&self) -> bool {
        self.signer.is_some()
    }

    pub fn unlock_approver(&mut self, signer: SigningKey) {
        self.signer = Some(signer);
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

    pub fn set_websocket_state(&mut self, state: LinkState) {
        self.websocket_state = state;
    }

    pub fn connection_label(&self) -> &'static str {
        match (self.data_state, self.websocket_state) {
            (LinkState::Live, LinkState::Live) => "LIVE",
            (LinkState::Live, _) => "READ ONLY",
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

    pub fn begin(&mut self, operation: TrustedOperation) -> Result<shared::Message, String> {
        if self.signer.is_none() {
            return Err("approver key is locked; unlock it before a privileged action".into());
        }
        let host = self.selected_host().ok_or("no host selected")?;
        if host.status != ConnectionStatus::Online {
            return Err(format!(
                "{} is offline; privileged action blocked",
                host.hostname
            ));
        }
        let agent = host.agent_id.clone();
        let mut random = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let request_id = format!("tui-{}", hex(&random));
        let ephemeral = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let start = TrustedClientFrame::Start {
            request_id: request_id.clone(),
            operation: operation.clone(),
            client_ephemeral_public: PublicKey::from(&ephemeral).to_bytes(),
        };
        let payload = shared::trusted::encode_client(&start)?;
        self.pending = Some(Pending {
            agent: agent.clone(),
            request_id: request_id.clone(),
            operation,
            ephemeral,
            challenge: None,
            pinned: false,
            transport: None,
        });
        self.mode = Mode::Review;
        self.status = format!("Waiting for host-signed manifest from {agent}");
        Ok(shared::Message::TrustedOperationClient {
            request_id,
            start: true,
            close: false,
            payload,
        })
    }

    pub fn challenge(&mut self, agent: &str, signed: SignedTrustedManifest) -> Result<(), String> {
        let pending = self.pending.as_mut().ok_or("no trusted request pending")?;
        if signed.manifest.request_id != pending.request_id
            || signed.manifest.operation != pending.operation
            || signed.manifest.client_ephemeral_public
                != PublicKey::from(&pending.ephemeral).to_bytes()
        {
            return Err("host challenge does not match the requested transaction".into());
        }
        let verifying = VerifyingKey::from_bytes(&signed.host_identity_public)
            .map_err(|_| "invalid host identity key")?;
        let signature = Signature::from_slice(&signed.signature)
            .map_err(|_| "invalid host signature encoding")?;
        verifying
            .verify(
                &signed
                    .manifest
                    .canonical_bytes()
                    .map_err(|error| error.to_string())?,
                &signature,
            )
            .map_err(|_| "host manifest signature is invalid")?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(signed.host_identity_public);
        pending.pinned = match self.pins.get(agent) {
            Some(existing) if existing == &encoded => true,
            Some(_) => return Err("HOST IDENTITY CHANGED; explicit re-pairing is required".into()),
            None => false,
        };
        self.status = if pending.pinned {
            "Verified host identity. Review exact fields and press a to approve.".into()
        } else {
            format!(
                "UNPAIRED HOST {}. Verify out of band, then press p to pin.",
                fingerprint(&signed.host_identity_public)
            )
        };
        pending.challenge = Some(signed);
        Ok(())
    }

    pub fn pin_current(&mut self, agent: &str) -> Result<(), String> {
        let pending = self.pending.as_mut().ok_or("no manifest to pin")?;
        let challenge = pending.challenge.as_ref().ok_or("no manifest to pin")?;
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(challenge.host_identity_public);
        self.pins.insert(agent.to_owned(), encoded);
        if let Some(parent) = self.pins_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(
            &self.pins_path,
            serde_json::to_vec_pretty(&self.pins).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        pending.pinned = true;
        self.status = "Host identity pinned. Press a to approve exact transaction.".into();
        Ok(())
    }

    pub fn approve(&mut self) -> Result<shared::Message, String> {
        let signer = self
            .signer
            .as_ref()
            .ok_or("approver key is locked; unlock it before approval")?;
        let pending = self.pending.as_mut().ok_or("no trusted request pending")?;
        if !pending.pinned {
            return Err("host identity is not pinned".into());
        }
        let challenge = pending
            .challenge
            .clone()
            .ok_or("no host challenge received")?;
        let canonical = challenge
            .manifest
            .canonical_bytes()
            .map_err(|error| error.to_string())?;
        let signature = signer.sign(&canonical).to_bytes().to_vec();
        pending.transport = Some(ClientTransport::new(
            &pending.ephemeral,
            challenge.manifest.broker_transport_public,
            &canonical,
            &pending.request_id,
        )?);
        let payload = shared::trusted::encode_client(&TrustedClientFrame::Approve {
            signed_manifest: Box::new(challenge),
            approver_public: signer.verifying_key().to_bytes(),
            approver_signature: signature,
        })?;
        if matches!(pending.operation, TrustedOperation::RootPty { .. }) {
            self.mode = Mode::Terminal;
        }
        self.status = "Signed approval sent directly to host gate; root channel active.".into();
        Ok(shared::Message::TrustedOperationClient {
            request_id: pending.request_id.clone(),
            start: false,
            close: false,
            payload,
        })
    }

    pub fn encrypted_input(&mut self, bytes: &[u8]) -> Result<shared::Message, String> {
        let pending = self.pending.as_mut().ok_or("no active root session")?;
        let transport = pending
            .transport
            .as_mut()
            .ok_or("root session not approved")?;
        let (counter, data) = transport.encrypt(bytes)?;
        let payload =
            shared::trusted::encode_client(&TrustedClientFrame::Ciphertext { counter, data })?;
        Ok(shared::Message::TrustedOperationClient {
            request_id: pending.request_id.clone(),
            start: false,
            close: false,
            payload,
        })
    }

    pub fn host_frame(&mut self, frame: TrustedHostFrame) -> Result<(), String> {
        match frame {
            TrustedHostFrame::Ciphertext { counter, data } => {
                let pending = self.pending.as_mut().ok_or("unexpected root ciphertext")?;
                let transport = pending
                    .transport
                    .as_mut()
                    .ok_or("root session not approved")?;
                let plaintext = transport.decrypt(counter, &data)?;
                let event: TrustedPlaintext = serde_json::from_slice(&plaintext)
                    .map_err(|_| "invalid root plaintext frame")?;
                match event {
                    TrustedPlaintext::Output { stream, data } => {
                        let text = String::from_utf8_lossy(&data);
                        self.output.extend(text.lines().map(|line| {
                            if stream == "stderr" {
                                format!("! {line}")
                            } else {
                                line.to_owned()
                            }
                        }));
                    }
                    TrustedPlaintext::Exit { code, message } => {
                        self.status = format!("{message} (exit {code})");
                        self.mode = Mode::Fleet;
                    }
                }
            }
            TrustedHostFrame::Error { message } => return Err(message),
            TrustedHostFrame::Closed => {
                self.status = "Trusted root session closed.".into();
                self.mode = Mode::Fleet;
                self.pending = None;
            }
            TrustedHostFrame::Result { exit_code, message } => {
                self.status = format!("{message} ({exit_code})");
            }
            TrustedHostFrame::Challenge(_) => return Err("duplicate host challenge".into()),
        }
        Ok(())
    }
}

pub fn fingerprint(public: &[u8; 32]) -> String {
    let digest = Sha256::digest(public);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
    fn app_starts_in_overview_without_an_approver_key() {
        let app = App::new(PathBuf::from("/nonexistent/pins"));
        assert_eq!(app.view, View::Overview);
        assert!(!app.approver_unlocked());
    }

    #[test]
    fn fleet_replacement_preserves_agent_selection() {
        let mut app = App::new(PathBuf::from("/nonexistent/pins"));
        app.replace_fleet(fleet(&[("agent-a", "host-a"), ("agent-b", "host-b")]));
        app.select_next();
        assert_eq!(app.selected_host().unwrap().hostname, "host-b");

        app.replace_fleet(fleet(&[("agent-b", "host-b"), ("agent-a", "host-a")]));
        assert_eq!(app.selected_host().unwrap().hostname, "host-b");
    }

    #[test]
    fn read_plane_stays_available_when_websocket_is_degraded() {
        let mut app = App::new(PathBuf::from("/nonexistent/pins"));
        app.set_data_state(LinkState::Live);
        app.set_websocket_state(LinkState::Degraded);
        assert_eq!(app.connection_label(), "READ ONLY");
    }

    #[test]
    fn background_refresh_does_not_downgrade_last_good_data() {
        let mut app = App::new(PathBuf::from("/nonexistent/pins"));
        app.set_data_state(LinkState::Live);
        app.set_data_state(LinkState::Connecting);
        assert_eq!(app.data_state, LinkState::Live);
    }

    #[test]
    fn privileged_action_rejects_an_offline_target() {
        let signer = SigningKey::from_bytes(&[9; 32]);
        let mut app = App::new(PathBuf::from("/nonexistent/pins"));
        app.unlock_approver(signer);
        let mut response = fleet(&[("host-a", "host-a")]);
        response.hosts[0].status = ConnectionStatus::Offline;
        app.replace_fleet(response);
        let error = app
            .begin(TrustedOperation::RootPty {
                shell: "/bin/bash".into(),
                ttl_secs: 60,
                cols: 80,
                rows: 24,
            })
            .unwrap_err();
        assert!(error.contains("offline"));
    }

    #[test]
    fn core_events_become_bounded_human_readable_activity() {
        let mut app = App::new(PathBuf::from("/nonexistent/pins"));
        app.record_core_event(CoreEvent {
            id: 7,
            kind: CoreEventKind::HostDisconnected,
            agent_id: Some("agent-a".into()),
            observed_at: 99,
        });
        assert_eq!(app.activity.len(), 1);
        assert_eq!(app.activity[0].summary, "Host disconnected");
    }

    #[test]
    fn challenge_uses_signed_operation_not_server_display_text() {
        let signer = SigningKey::from_bytes(&[9; 32]);
        let mut app = App::new(PathBuf::from("/nonexistent/pins"));
        app.unlock_approver(signer);
        app.replace_fleet(fleet(&[("host-a", "host-a")]));
        let operation = TrustedOperation::RootPty {
            shell: "/bin/bash".into(),
            ttl_secs: 600,
            cols: 80,
            rows: 24,
        };
        app.begin(operation.clone()).unwrap();
        assert_eq!(app.pending.as_ref().unwrap().operation, operation);
    }
}
