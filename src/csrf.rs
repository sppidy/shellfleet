//! CSRF protection via double-submit cookie pattern.
//!
//! On every request from a browser session (one carrying `auth_token`),
//! we ensure a `csrf` cookie is set. Mutating methods (POST/PUT/DELETE/
//! PATCH) must additionally carry an `X-CSRF` header that matches the
//! `csrf` cookie, otherwise the request is rejected with 403.
//!
//! Requests without an `auth_token` cookie are agent / public traffic
//! (device-auth bootstrap, /healthz, …) and pass through unchanged so
//! the agent doesn't need to learn a token-handshake protocol.

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, Method, Response, StatusCode},
    middleware::Next,
};
use rand::RngCore;

const CSRF_COOKIE: &str = "csrf";
const CSRF_HEADER: &str = "x-csrf";
const AUTH_COOKIE: &str = "auth_token";

fn random_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn cookie_value<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    let header = req.headers().get("cookie")?.to_str().ok()?;
    for part in header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k == name {
                return Some(v);
            }
        }
    }
    None
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    )
}

fn set_csrf_cookie(resp: &mut Response<Body>, token: &str) {
    // Strict, Path=/, NOT HttpOnly so the SPA can read it.
    let secure = std::env::var("COOKIE_SECURE")
        .ok()
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true);
    let secure_attr = if secure { "; Secure" } else { "" };
    let value = format!(
        "{CSRF_COOKIE}={token}; Path=/; SameSite=Strict{secure_attr}"
    );
    if let Ok(hv) = HeaderValue::from_str(&value) {
        resp.headers_mut().append(axum::http::header::SET_COOKIE, hv);
    }
}

/// Double-submit token comparison: the request is valid only when the
/// header token is non-empty and exactly equals the cookie token.
/// Behavior-preserving extraction of the previously-inline check so it
/// can be unit-tested. (W4 will swap the `==` for a constant-time compare.)
fn tokens_match(header: &str, cookie: &str) -> bool {
    use subtle::ConstantTimeEq;
    // Constant-time compare to avoid leaking the CSRF token via timing.
    !header.is_empty() && bool::from(header.as_bytes().ct_eq(cookie.as_bytes()))
}

pub async fn middleware(req: Request, next: Next) -> Response<Body> {
    let auth_present = cookie_value(&req, AUTH_COOKIE).is_some();
    let csrf_cookie = cookie_value(&req, CSRF_COOKIE).map(|s| s.to_string());

    if auth_present && is_mutating(req.method()) {
        let header_val = req
            .headers()
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let cookie_val = csrf_cookie.clone();
        let ok = match (header_val, cookie_val) {
            (Some(h), Some(c)) => tokens_match(&h, &c),
            _ => false,
        };
        if !ok {
            tracing::warn!(
                method = %req.method(),
                path = %req.uri().path(),
                "csrf check failed"
            );
            let mut resp = Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from("csrf"))
                .unwrap();
            // Refresh the cookie so the SPA can recover.
            let new_token = random_token();
            set_csrf_cookie(&mut resp, &new_token);
            return resp;
        }
    }

    let mut resp = next.run(req).await;

    // Seed the cookie on first browser session contact, or rotate after
    // a missing/invalid attempt that we already let through (GET).
    if csrf_cookie.is_none() && auth_present {
        let new_token = random_token();
        set_csrf_cookie(&mut resp, &new_token);
    } else if csrf_cookie.is_none() {
        // Public/agent path: still seed so the SPA's first /api/me call
        // (which carries auth_token after OAuth) gets a token even if
        // the request itself comes through before the user is authed.
        let new_token = random_token();
        set_csrf_cookie(&mut resp, &new_token);
    }

    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_only_when_equal_and_nonempty() {
        assert!(tokens_match("abc123", "abc123"));
        assert!(!tokens_match("abc123", "different"));
        // An empty token must never validate, even against another empty.
        assert!(!tokens_match("", ""));
        assert!(!tokens_match("abc", ""));
        assert!(!tokens_match("", "abc"));
    }
}
