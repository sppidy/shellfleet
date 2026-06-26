//! Global IP allowlist enforcement.
//!
//! When EE is active and the operator has populated the allowlist, dashboard /
//! user-facing requests from non-listed client IPs are rejected. The list lives
//! in the EE sidecar; CE caches it (refreshed every 30s) and matches locally.
//!
//! Safe by default: an empty / all-disabled list means no restriction, so the
//! gate only ever activates once the operator adds an enabled CIDR. Agent and
//! infrastructure paths (agent WS, device pairing, /internal, /api/v1, /healthz)
//! are exempt so a user-IP list never severs the fleet.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use crate::{AppState, ee, throttle};

/// Pull the allowlist from EE into the CE cache every 30s. Treats any
/// failure / unlicensed (402) response as "no allowlist" (clears the cache).
pub fn spawn_refresher(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if let Some(ee_url) = ee::ee_sidecar_url() {
                let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
                let url = format!("{}/api/ee/ip-allowlist", ee_url.trim_end_matches('/'));
                match reqwest::Client::new()
                    .get(&url)
                    .bearer_auth(&secret)
                    .header("x-shellfleet-login", "system")
                    .header("x-shellfleet-role", "admin")
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(entries) = resp.json::<Vec<serde_json::Value>>().await {
                            let list: Vec<(String, bool)> = entries
                                .iter()
                                .filter_map(|e| {
                                    let cidr = e["cidr"].as_str()?.to_string();
                                    let enabled = e["enabled"].as_i64().unwrap_or(0) == 1;
                                    Some((cidr, enabled))
                                })
                                .collect();
                            *state.ip_allowlist.lock().await = list;
                        }
                    }
                    // Unlicensed / unreachable → no allowlist (fail-open).
                    _ => state.ip_allowlist.lock().await.clear(),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

/// True if `cidr` (a CIDR like `10.0.0.0/8` or a bare host like `1.2.3.4`)
/// contains `ip`. Handles bare IPs that `ipnet` won't parse without a prefix.
fn cidr_contains(cidr: &str, ip: &IpAddr) -> bool {
    if let Ok(net) = cidr.parse::<ipnet::IpNet>() {
        return net.contains(ip);
    }
    cidr.parse::<IpAddr>()
        .map(|single| &single == ip)
        .unwrap_or(false)
}

fn ip_allowed(entries: &[(String, bool)], ip_str: &str) -> bool {
    let enabled: Vec<&String> = entries
        .iter()
        .filter_map(|(cidr, en)| if *en { Some(cidr) } else { None })
        .collect();
    if enabled.is_empty() {
        return true; // no restriction configured
    }
    let Ok(ip) = ip_str.parse::<IpAddr>() else {
        return false;
    };
    enabled.iter().any(|cidr| cidr_contains(cidr, &ip))
}

/// Agent + infrastructure paths the user-IP allowlist must never block.
fn is_exempt(path: &str) -> bool {
    path == "/healthz"
        || path == "/agent/ws"
        || path.starts_with("/internal")
        || path.starts_with("/api/device")
        || path.starts_with("/api/v1")
}

pub async fn middleware(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    if !ee::ee_active() || is_exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let entries = state.ip_allowlist.lock().await.clone();
    if entries.iter().all(|(_, en)| !*en) {
        return next.run(req).await; // empty/all-disabled = allow
    }
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    let ip = throttle::real_client_ip(req.headers(), peer);
    if ip_allowed(&entries, &ip) {
        next.run(req).await
    } else {
        tracing::warn!(%ip, path = %req.uri().path(), "request blocked by IP allowlist");
        (StatusCode::FORBIDDEN, "ip not allowed").into_response()
    }
}
