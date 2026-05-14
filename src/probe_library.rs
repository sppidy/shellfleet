//! Static catalog of stock probes shipped by the agent .deb. The
//! agent installs the scripts under /etc/shellfleet/probes.d/ (via
//! /usr/share symlinks). This catalog tells the SPA what each script
//! does and what env vars it understands, so the operator can pick
//! one from a dropdown instead of remembering script names.

use axum::{response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use crate::AppState;

#[derive(Serialize)]
pub struct CatalogEntry {
    /// Script filename — must match the agent's
    /// /etc/shellfleet/probes.d/<name> for the probe to actually
    /// run.
    pub script: String,
    pub title: String,
    pub description: String,
    /// Env vars the script reads, with default values. Operator can
    /// override per probe via the form.
    pub default_env: Vec<EnvDefault>,
    /// Suggested probe interval (seconds).
    pub interval_secs: u32,
    /// Suggested timeout (seconds).
    pub timeout_secs: u32,
}

#[derive(Serialize)]
pub struct EnvDefault {
    pub key: String,
    pub value: String,
    pub description: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/", get(list_handler))
}

async fn list_handler() -> impl IntoResponse {
    Json(catalog())
}

fn catalog() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            script: "apt-pending-count.sh".into(),
            title: "Apt — pending updates".into(),
            description: "Red while any packages are awaiting upgrade. Runs `apt list --upgradable`.".into(),
            default_env: vec![],
            interval_secs: 3600,
            timeout_secs: 30,
        },
        CatalogEntry {
            script: "disk-usage.sh".into(),
            title: "Root disk usage".into(),
            description: "Red when the configured mount exceeds THRESHOLD%.".into(),
            default_env: vec![
                EnvDefault {
                    key: "THRESHOLD".into(),
                    value: "90".into(),
                    description: "Percent used at which the probe goes red.".into(),
                },
                EnvDefault {
                    key: "MOUNT".into(),
                    value: "/".into(),
                    description: "Filesystem to inspect with `df`.".into(),
                },
            ],
            interval_secs: 300,
            timeout_secs: 10,
        },
        CatalogEntry {
            script: "failed-services.sh".into(),
            title: "Systemd — failed units".into(),
            description: "Red when any --failed systemd unit exists; lists names in detail.".into(),
            default_env: vec![],
            interval_secs: 60,
            timeout_secs: 10,
        },
        CatalogEntry {
            script: "swap-pressure.sh".into(),
            title: "Swap pressure".into(),
            description: "Red when sustained swap (si+so) exceeds THRESHOLD_KB per second.".into(),
            default_env: vec![EnvDefault {
                key: "THRESHOLD_KB".into(),
                value: "256".into(),
                description: "Bytes per second of combined swap-in/swap-out tolerated before red.".into(),
            }],
            interval_secs: 60,
            timeout_secs: 10,
        },
        CatalogEntry {
            script: "load-average.sh".into(),
            title: "Load average per CPU".into(),
            description: "Red when 1-minute load average divided by CPU count exceeds THRESHOLD.".into(),
            default_env: vec![EnvDefault {
                key: "THRESHOLD".into(),
                value: "1.0".into(),
                description: "Load-per-CPU above which the probe goes red.".into(),
            }],
            interval_secs: 60,
            timeout_secs: 5,
        },
        CatalogEntry {
            script: "docker-running.sh".into(),
            title: "Docker — recent exits".into(),
            description: "Red when containers have exited within the last LOOKBACK seconds.".into(),
            default_env: vec![EnvDefault {
                key: "LOOKBACK".into(),
                value: "3600".into(),
                description: "How far back to scan for exited containers (seconds).".into(),
            }],
            interval_secs: 300,
            timeout_secs: 15,
        },
        CatalogEntry {
            script: "swarm-pending.sh".into(),
            title: "Swarm — pending services (manager)".into(),
            description: "Manager-only. Red when any service has Replicas != desired.".into(),
            default_env: vec![],
            interval_secs: 60,
            timeout_secs: 15,
        },
        CatalogEntry {
            script: "cert-expiry-days.sh".into(),
            title: "Letsencrypt — expiry".into(),
            description: "Red when any matching cert expires in fewer than THRESHOLD days.".into(),
            default_env: vec![
                EnvDefault {
                    key: "THRESHOLD".into(),
                    value: "14".into(),
                    description: "Days remaining at which the probe goes red.".into(),
                },
                EnvDefault {
                    key: "CERT_GLOB".into(),
                    value: "/etc/letsencrypt/live/*/fullchain.pem".into(),
                    description: "Glob of certs to inspect.".into(),
                },
            ],
            interval_secs: 21_600,
            timeout_secs: 10,
        },
        CatalogEntry {
            script: "journal-errors.sh".into(),
            title: "Journal — recent errors".into(),
            description: "Red when journalctl reports more than THRESHOLD priority<=err lines in the last LOOKBACK seconds.".into(),
            default_env: vec![
                EnvDefault {
                    key: "THRESHOLD".into(),
                    value: "0".into(),
                    description: "Tolerated error lines before the probe goes red.".into(),
                },
                EnvDefault {
                    key: "LOOKBACK".into(),
                    value: "900".into(),
                    description: "How far back to scan (seconds).".into(),
                },
            ],
            interval_secs: 300,
            timeout_secs: 15,
        },
        CatalogEntry {
            script: "ntp-drift.sh".into(),
            title: "NTP drift".into(),
            description: "Red when system clock is more than THRESHOLD_MS off NTP time.".into(),
            default_env: vec![EnvDefault {
                key: "THRESHOLD_MS".into(),
                value: "1000".into(),
                description: "Milliseconds of drift tolerated before red.".into(),
            }],
            interval_secs: 600,
            timeout_secs: 10,
        },
    ]
}
