//! Tower middleware that runs the per-real-IP token-bucket limiter
//! on the anonymous-attacker surface — `/auth/*`, `/api/me`, and
//! `/api/auth/mfa/verify`. These are the routes a user reaches before
//! they have a fully-verified session, so the per-actor `Throttle`
//! can't cover them.
//!
//! Real client IP comes from `CF-Connecting-IP` first (Cloudflare's
//! canonical header), then `X-Real-IP`, then the leftmost parseable
//! `X-Forwarded-For`, then the connection peer address. This works
//! for the live deploy (Cloudflare → origin) and for direct
//! Tailscale access (peer address).
//!
//! Behavior on limit hit: 429 with a short `Retry-After: 2` so well-
//! behaved clients back off. Bypassed in dev mode.

use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{Response, StatusCode},
    middleware::Next,
};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::AppState;

/// Returns true if the request path is part of the anonymous-attacker
/// surface that needs per-IP throttling. We deliberately don't gate
/// the entire /api/* tree this way because authenticated power users
/// (kicking off multiple container ops in rapid succession) would
/// otherwise hit the limit during normal use. The per-actor `Throttle`
/// already covers the high-value authed endpoints (MFA, device
/// approve); this layer covers what an unauthenticated attacker can
/// still touch.
fn is_limited_path(path: &str) -> bool {
    path.starts_with("/auth/")
        || path == "/api/me"
        || path.starts_with("/api/auth/mfa/")
        // Device-auth: `/api/device/request` and `/api/device/approve`
        // live on the unauth surface. `/api/device/token` and
        // `/api/device/refresh` are excluded — agents poll/token-rotate
        // there during pairing/reconnect and multiple agents behind the
        // same Docker gateway IP would exhaust the bucket.
        || path == "/api/device/request"
        || path == "/api/device/approve"
        || path == "/api/cli-auth/request"
        // Public API-key surface: `/api/v1/*` is authenticated by a
        // bearer API key, not a session cookie. Without per-IP throttling
        // here an attacker can brute-force API keys unbounded — each
        // attempt is a DB lookup with no backoff.
        || path.starts_with("/api/v1/")
}

pub async fn middleware(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response<Body> {
    if crate::auth::is_dev_mode() {
        return next.run(req).await;
    }

    let path = req.uri().path();
    if !is_limited_path(path) {
        return next.run(req).await;
    }

    let ip = crate::throttle::real_client_ip(req.headers(), Some(peer.ip()));
    let now = crate::throttle::now_secs_f64();
    if !state.anon_ip_limiter.allow(&ip, now) {
        tracing::warn!(%ip, %path, "anon ip rate limit hit");
        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(axum::http::header::RETRY_AFTER, "2")
            .body(Body::from("rate limited"))
            .unwrap();
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::is_limited_path;

    #[test]
    fn api_v1_is_part_of_the_per_ip_limited_surface() {
        assert!(is_limited_path("/api/v1/agents"));
        assert!(is_limited_path("/api/v1/"));
        assert!(!is_limited_path("/api/v10/agents"));
    }
}
