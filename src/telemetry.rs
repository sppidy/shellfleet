//! Anonymous usage telemetry reporter.
//!
//! Default-ON, easy opt-out: `SHELLFLEET_TELEMETRY=off` (env, the hard
//! switch) and an admin UI toggle (`meta.telemetry_enabled`). On a schedule
//! the server POSTs a small ANONYMOUS report to `TELEMETRY_URL` — only a
//! random instance id, version, CE/EE edition, integer counts, and a fixed
//! set of feature names. No PII (no logins, hostnames, IPs, or agent ids).

use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use crate::{db, ee, AppState};

/// The telemetry collector endpoint. Hardcoded (not configurable) — the only
/// telemetry knob is the on/off switch (SHELLFLEET_TELEMETRY / admin toggle).
const TELEMETRY_URL: &str = "https://telemetry.sppidy.in/v1/telemetry";
const REPORT_INTERVAL_SECS: u64 = 24 * 3600;
const STARTUP_DELAY_SECS: u64 = 30;

#[derive(Serialize)]
struct Report {
    instance_id: String,
    version: &'static str,
    edition: &'static str,
    users: i64,
    agents: usize,
    features: Vec<String>,
}

/// Pure opt-out decision. The `SHELLFLEET_TELEMETRY` env is the hard
/// switch — `off`/`false`/`0`/`no` disables regardless of the admin toggle;
/// otherwise the toggle decides (defaults on).
pub fn telemetry_enabled(env_value: Option<&str>, toggle_enabled: bool) -> bool {
    if let Some(v) = env_value {
        if matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no") {
            return false;
        }
    }
    toggle_enabled
}

fn env_forced_off() -> bool {
    std::env::var("SHELLFLEET_TELEMETRY")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no"))
        .unwrap_or(false)
}

async fn toggle_value(state: &AppState) -> bool {
    db::get_meta(&state.db, "telemetry_enabled")
        .await
        .ok()
        .flatten()
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Build an anonymous usage report. Emits ONLY allow-listed, non-PII fields.
async fn gather(state: &AppState) -> Report {
    let instance_id = db::ensure_instance_id(&state.db).await.unwrap_or_default();
    let users = db::count_users(&state.db).await.unwrap_or(0);

    let mut features: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let agents = {
        let map = state.agents.lock().await;
        for entry in map.values() {
            for c in &entry.capabilities {
                if matches!(c.as_str(), "docker" | "swarm" | "k8s" | "systemd") {
                    features.insert(c.clone());
                }
            }
        }
        map.len()
    };

    if ee::ee_active() {
        features.insert("ee".into());
    }
    if std::env::var("BACKUPS_ENABLED").map(|v| v == "true" || v == "1").unwrap_or(false) {
        features.insert("backups".into());
    }
    if std::env::var("METRICS_CONFIG_PATH").map(|v| !v.is_empty()).unwrap_or(false) {
        features.insert("metrics".into());
    }

    Report {
        instance_id,
        version: env!("CARGO_PKG_VERSION"),
        edition: if ee::ee_active() { "ee" } else { "ce" },
        users,
        agents,
        features: features.into_iter().collect(),
    }
}

pub fn spawn_reporter(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;
        let mut announced = false;
        loop {
            let env_val = std::env::var("SHELLFLEET_TELEMETRY").ok();
            if telemetry_enabled(env_val.as_deref(), toggle_value(&state).await) {
                let report = gather(&state).await;
                if !announced {
                    tracing::info!(
                        instance = %report.instance_id,
                        "anonymous usage telemetry is ON (no PII — counts/version/edition/features \
                         only). Disable with SHELLFLEET_TELEMETRY=off or the admin toggle."
                    );
                    announced = true;
                }
                match reqwest::Client::new()
                    .post(TELEMETRY_URL)
                    .json(&report)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => tracing::debug!("telemetry sent"),
                    Ok(r) => tracing::debug!(status = %r.status(), "telemetry non-2xx"),
                    Err(e) => tracing::debug!(error = %e, "telemetry send failed"),
                }
            }
            tokio::time::sleep(Duration::from_secs(REPORT_INTERVAL_SECS)).await;
        }
    });
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/", get(status_handler).post(toggle_handler))
}

async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let instance_id = db::ensure_instance_id(&state.db).await.unwrap_or_default();
    let toggle = toggle_value(&state).await;
    let env_val = std::env::var("SHELLFLEET_TELEMETRY").ok();
    Json(json!({
        "instance_id": instance_id,
        "enabled": telemetry_enabled(env_val.as_deref(), toggle),
        "toggle": toggle,
        "env_forced_off": env_forced_off(),
    }))
}

#[derive(Deserialize)]
struct ToggleBody {
    enabled: bool,
}

async fn toggle_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ToggleBody>,
) -> impl IntoResponse {
    let _ = db::set_meta(&state.db, "telemetry_enabled", if body.enabled { "1" } else { "0" }).await;
    Json(json!({ "ok": true, "toggle": body.enabled }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_off_is_a_hard_switch_overriding_the_toggle() {
        for off in ["off", "false", "0", "no", "OFF", " Off "] {
            assert!(!telemetry_enabled(Some(off), true), "env '{off}' must disable");
        }
        // Anything else leaves the toggle in charge.
        assert!(telemetry_enabled(Some("on"), true));
        assert!(telemetry_enabled(Some("1"), true));
        assert!(telemetry_enabled(None, true));
        // Toggle off (admin) disables even with env unset/on.
        assert!(!telemetry_enabled(None, false));
        assert!(!telemetry_enabled(Some("on"), false));
    }


    #[test]
    fn report_serializes_only_allowlisted_non_pii_fields() {
        let r = Report {
            instance_id: "00000000-0000".into(),
            version: "0.0.0",
            edition: "ce",
            users: 1,
            agents: 2,
            features: vec!["docker".into()],
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        let expected: std::collections::BTreeSet<&str> =
            ["instance_id", "version", "edition", "users", "agents", "features"]
                .into_iter()
                .collect();
        assert_eq!(keys, expected, "telemetry report must not carry any other (PII) fields");
    }
}
