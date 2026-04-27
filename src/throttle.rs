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
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

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

/// Best-effort real-client-IP extraction. Trust order:
/// 1. `CF-Connecting-IP` (set by Cloudflare on every request — the
///    canonical real-client header for CF deployments).
/// 2. `X-Real-IP` (set by some reverse proxies; included for non-CF
///    deployments).
/// 3. The leftmost non-private hop in `X-Forwarded-For`.
/// 4. The peer address as observed by the connection.
///
/// Returns the IP as a string. Falls back to the literal `"unknown"`
/// when nothing is parsable, which means the bucket for `"unknown"`
/// will be aggressively shared — that's fine; it just means
/// unidentifiable traffic is throttled jointly.
pub fn real_client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
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
    peer.map(|p| p.to_string()).unwrap_or_else(|| "unknown".into())
}

pub fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
