//! Tiny per-key in-memory throttle. Used to slow down brute-force
//! attacks on:
//!   - `/api/auth/mfa/verify` (key: GitHub login from pending JWT)
//!   - `/api/device/approve`  (key: caller IP)
//!
//! After `MAX_FAILS` failed attempts within the rolling window, the key
//! is locked for `LOCK_SECS`. Each successful action clears the
//! counter for that key. State lives in a single `Mutex<HashMap<...>>`
//! attached to AppState — no Redis, no external dep.
//!
//! This is **not** a substitute for an edge rate limiter (Cloudflare
//! WAF, nginx limit_req, etc.) — it is a defence-in-depth that runs
//! even when the edge is bypassed (Tailscale, dev tunnel, direct VM
//! access). The CE budget for "extra deps" is zero, so this module
//! deliberately stays small.

use axum::http::HeaderMap;
use ipnet::IpNet;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};

/// How many fails before locking the key.
pub const MAX_FAILS: u32 = 10;
/// How long the lock lasts after `MAX_FAILS`.
pub const LOCK_SECS: i64 = 15 * 60;
/// Idle time after which a key's record is considered stale and gets
/// dropped (so the map doesn't grow unboundedly).
const RECORD_TTL_SECS: i64 = 24 * 3600;

/// Token-bucket params for the public anonymous-endpoint limiter.
pub const ANON_BUCKET_CAPACITY: f64 = 30.0;
/// Refill 1 token every 2 seconds → 30 req / minute steady state with
/// 30 req burst. Large enough that a normal user opening the dashboard
/// (which fans out many /api/me checks on first paint) won't get
/// blocked, small enough that scripted brute-force is throttled.
pub const ANON_BUCKET_REFILL_PER_SEC: f64 = 0.5;

#[derive(Debug, Default)]
struct Record {
    fails: u32,
    locked_until: i64,
    last_touched: i64,
}

#[derive(Default)]
pub struct Throttle {
    records: Mutex<HashMap<String, Record>>,
}

pub enum CheckResult {
    /// Caller is currently locked out — `retry_after_secs` until the
    /// next attempt is permitted.
    Locked { retry_after_secs: i64 },
    /// Caller may proceed. Call `record_failure` if the action fails
    /// or `record_success` if it succeeds.
    Ok,
}

impl Throttle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether `key` is currently allowed to attempt the
    /// guarded action. Garbage-collects stale records on the way.
    pub fn check(&self, key: &str, now: i64) -> CheckResult {
        let mut map = self.records.lock().unwrap();
        // Opportunistic GC. Cheap because `retain` is O(n) and n is
        // bounded by the number of distinct attackers in the last day.
        map.retain(|_, r| now - r.last_touched < RECORD_TTL_SECS);

        match map.get(key) {
            Some(r) if r.locked_until > now => CheckResult::Locked {
                retry_after_secs: r.locked_until - now,
            },
            _ => CheckResult::Ok,
        }
    }

    pub fn record_failure(&self, key: &str, now: i64) {
        let mut map = self.records.lock().unwrap();
        let r = map.entry(key.to_string()).or_default();
        r.fails = r.fails.saturating_add(1);
        r.last_touched = now;
        if r.fails >= MAX_FAILS {
            r.locked_until = now + LOCK_SECS;
        }
    }

    pub fn record_success(&self, key: &str) {
        let mut map = self.records.lock().unwrap();
        map.remove(key);
    }
}

// ---------------------------------------------------------------------
// Per-IP token-bucket limiter for anonymous public endpoints.
//
// This is *defence in depth* on top of the edge limiter (Cloudflare
// WAF rate-limit rules). The edge sees the real client IP before our
// origin sees it; we only see Cloudflare's egress IP unless we read
// the `CF-Connecting-IP` header it sets on each request. We do, and
// then we re-throttle per-real-IP at the origin so a Tailscale-direct
// client (which bypasses Cloudflare) is still rate-limited.
// ---------------------------------------------------------------------

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last_touched: f64,
}

#[derive(Default)]
pub struct IpBucketLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl IpBucketLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spend one token. Returns true if the request is allowed.
    pub fn allow(&self, ip: &str, now: f64) -> bool {
        let mut map = self.buckets.lock().unwrap();
        // Periodic GC: drop buckets idle for > RECORD_TTL_SECS.
        map.retain(|_, b| now - b.last_touched < RECORD_TTL_SECS as f64);
        let b = map.entry(ip.to_string()).or_insert_with(|| Bucket {
            tokens: ANON_BUCKET_CAPACITY,
            last_touched: now,
        });
        let elapsed = (now - b.last_touched).max(0.0);
        b.tokens = (b.tokens + elapsed * ANON_BUCKET_REFILL_PER_SEC).min(ANON_BUCKET_CAPACITY);
        b.last_touched = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Cloudflare's published egress ranges (https://www.cloudflare.com/ips/),
/// hard-coded as the default trusted-proxy set so a fresh deploy gets
/// safe defaults. The operator can override via `TRUSTED_PROXY_CIDRS`
/// (comma-separated) — useful if you're behind a different proxy or
/// running edge-direct (in which case set it to the empty string).
const CLOUDFLARE_CIDRS: &[&str] = &[
    // IPv4
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    // IPv6
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

fn trusted_proxy_set() -> &'static Vec<IpNet> {
    static SET: OnceLock<Vec<IpNet>> = OnceLock::new();
    SET.get_or_init(|| {
        let raw = std::env::var("TRUSTED_PROXY_CIDRS").ok();
        let source: Vec<String> = match raw.as_deref() {
            Some("") => Vec::new(),
            Some(v) => v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            None => CLOUDFLARE_CIDRS.iter().map(|s| s.to_string()).collect(),
        };
        source
            .into_iter()
            .filter_map(|s| match s.parse::<IpNet>() {
                Ok(n) => Some(n),
                Err(e) => {
                    tracing::warn!(cidr = %s, error = %e, "ignoring invalid TRUSTED_PROXY_CIDRS entry");
                    None
                }
            })
            .collect()
    })
}

fn peer_is_trusted_proxy(peer: IpAddr) -> bool {
    let set = trusted_proxy_set();
    set.iter().any(|n| n.contains(&peer))
}

/// Best-effort real-client-IP extraction. Forwarding headers are only
/// honoured when the connection's peer address is in the trusted-proxy
/// set (Cloudflare CIDRs by default; configurable via
/// `TRUSTED_PROXY_CIDRS`). This prevents an attacker who reaches the
/// origin directly — bypassing Cloudflare via a misconfigured DNS or
/// a leaked Tailscale ACL — from spoofing `CF-Connecting-IP` to dodge
/// the per-IP rate limiter.
///
/// Trust order, after the proxy gate:
/// 1. `CF-Connecting-IP` (Cloudflare's canonical real-client header).
/// 2. `X-Real-IP` (other reverse proxies).
/// 3. The leftmost parseable hop in `X-Forwarded-For`.
/// Falls back to peer IP otherwise.
pub fn real_client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    let peer_trusted = peer.map(peer_is_trusted_proxy).unwrap_or(false);
    if peer_trusted {
        if let Some(v) = headers.get("cf-connecting-ip").and_then(|h| h.to_str().ok()) {
            if v.parse::<IpAddr>().is_ok() {
                return v.to_string();
            }
        }
        if let Some(v) = headers.get("x-real-ip").and_then(|h| h.to_str().ok()) {
            if v.parse::<IpAddr>().is_ok() {
                return v.to_string();
            }
        }
        if let Some(v) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
            for part in v.split(',') {
                let trimmed = part.trim();
                if trimmed.parse::<IpAddr>().is_ok() {
                    return trimmed.to_string();
                }
            }
        }
    }
    peer.map(|p| p.to_string()).unwrap_or_else(|| "unknown".into())
}

pub fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn untrusted_peer_cannot_spoof_client_ip() {
        // 192.0.2.1 (TEST-NET-1) is never in the default trusted
        // (Cloudflare) proxy set, so forwarding headers must be ignored.
        let peer: IpAddr = "192.0.2.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "8.8.8.8".parse().unwrap());
        headers.insert("x-forwarded-for", "9.9.9.9".parse().unwrap());
        assert_eq!(
            real_client_ip(&headers, Some(peer)),
            "192.0.2.1",
            "an untrusted peer must not be able to spoof its client IP"
        );
    }

    #[test]
    fn missing_peer_falls_back_to_unknown() {
        let headers = HeaderMap::new();
        assert_eq!(real_client_ip(&headers, None), "unknown");
    }
}
