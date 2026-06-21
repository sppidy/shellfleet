//! Passkey as a second factor — a TOTP-equivalent way to satisfy the
//! post-OAuth MFA challenge.
//!
//! The real WebAuthn assertion is verified in the EE sidecar
//! (`/api/ee/webauthn/auth/*`, backed by `webauthn-rs`). Those routes are
//! normally only reachable by a fully-authenticated user, so a user stuck at
//! the pending-MFA step can't call them directly. These CE routes are the
//! pending-MFA-allowed bridge:
//!
//! - `GET  /api/auth/passkey/available` — does the pending user have a passkey?
//!   (drives whether the `/mfa` page advertises the option)
//! - `POST /api/auth/passkey/begin`     — start the assertion for the pending login
//! - `POST /api/auth/passkey/finish`    — on EE-verified assertion, upgrade the
//!   pending cookie to the SAME `mfa:true` session a correct TOTP code yields.
//!
//! The login is taken from the pending cookie, never from the client, so a
//! passkey can only complete the second factor for the identity that already
//! passed OAuth. Whitelisted in `rbac::middleware` like `/auth/mfa/`.

use axum::{
    body::Bytes,
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::{auth, ee, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/available", get(available_handler))
        .route("/begin", post(begin_handler))
        .route("/finish", post(finish_handler))
}

/// EE base url + internal secret, or None when EE isn't active.
fn ee_ctx() -> Option<(String, String)> {
    let url = ee::ee_sidecar_url()?;
    let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
    Some((url, secret))
}

/// `{ available: bool }` — whether the pending user has a registered passkey.
/// Any negative (no EE, unlicensed, no creds, error) returns false so the UI
/// simply doesn't advertise the option.
async fn available_handler(jar: CookieJar) -> impl IntoResponse {
    let deny = || Json(serde_json::json!({ "available": false }));
    let Some(pending) = auth::pending_mfa_claims(&jar) else {
        return deny().into_response();
    };
    let Some((url, secret)) = ee_ctx() else {
        return deny().into_response();
    };
    let endpoint = format!("{}/api/ee/webauthn/credentials", url.trim_end_matches('/'));
    let available = match reqwest::Client::new()
        .get(&endpoint)
        .bearer_auth(&secret)
        .header("x-shellfleet-login", &pending.sub)
        .header("x-shellfleet-role", &pending.role)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let creds: serde_json::Value = resp.json().await.unwrap_or_default();
            creds.as_array().map(|a| !a.is_empty()).unwrap_or(false)
        }
        _ => false,
    };
    Json(serde_json::json!({ "available": available })).into_response()
}

/// Forward the assertion-begin to EE, carrying the pending login. Returns the
/// EE JSON ({ state_id, options }) verbatim for the browser to feed to
/// `navigator.credentials.get`.
async fn begin_handler(jar: CookieJar) -> impl IntoResponse {
    let Some(pending) = auth::pending_mfa_claims(&jar) else {
        return (StatusCode::UNAUTHORIZED, "no pending mfa").into_response();
    };
    let Some((url, secret)) = ee_ctx() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "passkeys require EE").into_response();
    };
    let endpoint = format!("{}/api/ee/webauthn/auth/begin", url.trim_end_matches('/'));
    match reqwest::Client::new()
        .post(&endpoint)
        .bearer_auth(&secret)
        .header("x-shellfleet-login", &pending.sub)
        .header("x-shellfleet-role", &pending.role)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "EE unreachable").into_response(),
    }
}

/// Verify the assertion in EE and, on success, mint the SAME full session a
/// correct TOTP code would (mfa:true, role re-resolved from the DB, 24h) and
/// set it on the `auth_token` cookie. `body` is the WebAuthn assertion JSON,
/// forwarded to EE untouched.
async fn finish_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> impl IntoResponse {
    if auth::is_dev_mode() {
        return Json(serde_json::json!({ "ok": true })).into_response();
    }
    let Some(pending) = auth::pending_mfa_claims(&jar) else {
        return (StatusCode::UNAUTHORIZED, "no pending mfa").into_response();
    };
    let Some((url, secret)) = ee_ctx() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "passkeys require EE").into_response();
    };

    let endpoint = format!("{}/api/ee/webauthn/auth/finish", url.trim_end_matches('/'));
    let resp = match reqwest::Client::new()
        .post(&endpoint)
        .bearer_auth(&secret)
        .header("x-shellfleet-login", &pending.sub)
        .header("x-shellfleet-role", &pending.role)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_GATEWAY, "EE unreachable").into_response(),
    };

    if !resp.status().is_success() {
        let code = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::UNAUTHORIZED);
        let msg = resp.text().await.unwrap_or_else(|_| "passkey verification failed".into());
        crate::db::record_audit(
            &state.db, crate::now_unix(), Some(&pending.sub), None,
            "auth.passkey.fail", false, None,
        )
        .await;
        return (code, msg).into_response();
    }

    // EE cryptographically verified the assertion for the pending login.
    // Issue the same mfa-satisfied session TOTP produces (role from the DB).
    let role = match crate::db::get_user(&state.db, &pending.sub).await {
        Ok(Some(r)) => auth::Role::parse(&r.role),
        _ => auth::Role::parse(&pending.role),
    };
    let token = match auth::issue_jwt(&pending.sub, role, true, 24 * 3600) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to issue post-passkey jwt");
            return (StatusCode::INTERNAL_SERVER_ERROR, "jwt error").into_response();
        }
    };
    crate::db::record_audit(
        &state.db, crate::now_unix(), Some(&pending.sub), None,
        "auth.passkey.ok", true, None,
    )
    .await;
    let cookie = auth::build_session_cookie(token);
    (jar.add(cookie), Json(serde_json::json!({ "ok": true }))).into_response()
}
