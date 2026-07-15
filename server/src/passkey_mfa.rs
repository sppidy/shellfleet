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
    Json, Router,
    body::Bytes,
    extract::State,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::{AppState, auth, ee, internal_auth};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Second-factor (post-OAuth, pending-MFA) — bypass TOTP with a passkey.
        .route("/available", get(available_handler))
        .route("/begin", post(begin_handler))
        .route("/finish", post(finish_handler))
        // Passwordless primary login (pre-auth) — discoverable passkey, no OAuth.
        .route("/login/begin", post(login_begin_handler))
        .route("/login/finish", post(login_finish_handler))
}

/// EE base URL, or None when EE isn't active.
fn ee_ctx() -> Option<String> {
    ee::ee_sidecar_url()
}

/// `{ available: bool }` — whether the pending user has a registered passkey.
/// Any negative (no EE, unlicensed, no creds, error) returns false so the UI
/// simply doesn't advertise the option.
async fn available_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let deny = || Json(serde_json::json!({ "available": false }));
    let pending = match auth::pending_mfa_user(&jar, &state.db).await {
        Ok(pending) => pending,
        Err(_) => return deny().into_response(),
    };
    let Some(url) = ee_ctx() else {
        return deny().into_response();
    };
    let endpoint = format!("{}/api/ee/webauthn/credentials", url.trim_end_matches('/'));
    let available = match internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::GET,
        &endpoint,
        Vec::new(),
        "",
        &pending.sub,
        &pending.role,
        std::time::Duration::from_secs(5),
    )
    .await
    {
        Ok(resp) if resp.status.is_success() => {
            let creds: serde_json::Value = resp.json().unwrap_or_default();
            creds.as_array().map(|a| !a.is_empty()).unwrap_or(false)
        }
        _ => false,
    };
    Json(serde_json::json!({ "available": available })).into_response()
}

/// Forward the assertion-begin to EE, carrying the pending login. Returns the
/// EE JSON ({ state_id, options }) verbatim for the browser to feed to
/// `navigator.credentials.get`.
async fn begin_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = match auth::pending_mfa_user(&jar, &state.db).await {
        Ok(pending) => pending,
        Err(error) => return error.into_response(),
    };
    let Some(url) = ee_ctx() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "passkeys require EE").into_response();
    };
    let endpoint = format!("{}/api/ee/webauthn/auth/begin", url.trim_end_matches('/'));
    match internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &endpoint,
        Vec::new(),
        "",
        &pending.sub,
        &pending.role,
        std::time::Duration::from_secs(10),
    )
    .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text();
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
    let pending = match auth::pending_mfa_user(&jar, &state.db).await {
        Ok(pending) => pending,
        Err(error) => return error.into_response(),
    };
    let Some(url) = ee_ctx() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "passkeys require EE").into_response();
    };

    let endpoint = format!("{}/api/ee/webauthn/auth/finish", url.trim_end_matches('/'));
    let resp = match internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &endpoint,
        body.to_vec(),
        "application/json",
        &pending.sub,
        &pending.role,
        std::time::Duration::from_secs(15),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_GATEWAY, "EE unreachable").into_response(),
    };

    if !resp.status.is_success() {
        let code = StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::UNAUTHORIZED);
        let msg = resp.text();
        crate::db::record_audit(
            &state.db,
            crate::now_unix(),
            Some(&pending.sub),
            None,
            "auth.passkey.fail",
            false,
            None,
        )
        .await;
        return (code, msg).into_response();
    }

    // EE cryptographically verified the assertion for the pending login.
    // Issue the same mfa-satisfied session TOTP produces (role from the DB).
    let role = auth::Role::parse(&pending.role);
    let token = match auth::issue_jwt(&pending.sub, role, true, 24 * 3600) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to issue post-passkey jwt");
            return (StatusCode::INTERNAL_SERVER_ERROR, "jwt error").into_response();
        }
    };
    crate::db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&pending.sub),
        None,
        "auth.passkey.ok",
        true,
        None,
    )
    .await;
    let cookie = auth::build_session_cookie(token);
    (jar.add(cookie), Json(serde_json::json!({ "ok": true }))).into_response()
}

// --------------------------------------------------------------------------
// Passwordless primary login (pre-auth). No session yet — the EE discoverable
// ceremony identifies the user from the passkey and verifies the signature;
// these routes are whitelisted in rbac::middleware and (being cookie-less) are
// not CSRF-checked. On success we set the auth_token cookie from the EE-minted,
// already-mfa-verified CE session JWT.
// --------------------------------------------------------------------------

/// Start a discoverable assertion (no user known yet). Returns the EE options
/// for `navigator.credentials.get`.
async fn login_begin_handler() -> impl IntoResponse {
    let Some(url) = ee_ctx() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "passkeys require EE").into_response();
    };
    let endpoint = format!("{}/api/ee/webauthn/login/begin", url.trim_end_matches('/'));
    match internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &endpoint,
        Vec::new(),
        "",
        "",
        "",
        std::time::Duration::from_secs(10),
    )
    .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text();
            (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "EE unreachable").into_response(),
    }
}

/// Verify the discoverable assertion in EE and, on success, set the session
/// cookie to the EE-minted JWT — but only after re-validating it is a real,
/// mfa-verified CE token (defence-in-depth on the EE→CE trust boundary).
async fn login_finish_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(url) = ee_ctx() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "passkeys require EE").into_response();
    };
    let endpoint = format!("{}/api/ee/webauthn/login/finish", url.trim_end_matches('/'));
    let resp = match internal_auth::send(
        &reqwest::Client::new(),
        reqwest::Method::POST,
        &endpoint,
        body.to_vec(),
        "application/json",
        "",
        "",
        std::time::Duration::from_secs(15),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_GATEWAY, "EE unreachable").into_response(),
    };
    if !resp.status.is_success() {
        let code = StatusCode::from_u16(resp.status.as_u16()).unwrap_or(StatusCode::UNAUTHORIZED);
        let msg = resp.text();
        return (code, msg).into_response();
    }

    let data: serde_json::Value = resp.json().unwrap_or_default();
    let token = data["token"].as_str().unwrap_or_default().to_string();
    // The token was minted by CE's own /internal/auth/resolve; re-decode it and
    // require mfa:true before trusting it as a session.
    let claims = match auth::claims_from_token(&token) {
        Some(c) if c.mfa => c,
        _ => return (StatusCode::BAD_GATEWAY, "invalid session token from EE").into_response(),
    };
    let current = match crate::db::get_user(&state.db, &claims.sub).await {
        Ok(Some(row)) if claims.iat >= row.session_epoch => row,
        Ok(Some(_)) => {
            return (StatusCode::UNAUTHORIZED, "session revoked — please sign in again")
                .into_response();
        }
        Ok(None) => return (StatusCode::BAD_GATEWAY, "unknown session user from EE").into_response(),
        Err(error) => {
            tracing::error!(%error, login = %claims.sub, "passkey login session verification failed");
            return (StatusCode::SERVICE_UNAVAILABLE, "session verification unavailable")
                .into_response();
        }
    };
    if auth::Role::parse(&claims.role) != auth::Role::parse(&current.role) {
        return (StatusCode::BAD_GATEWAY, "stale session role from EE").into_response();
    }
    crate::db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&claims.sub),
        None,
        "auth.passkey.login",
        true,
        None,
    )
    .await;
    let cookie = auth::build_session_cookie(token);
    (jar.add(cookie), Json(serde_json::json!({ "ok": true }))).into_response()
}
