//! CE 2FA: TOTP (RFC 6238) enrollment + verification.
//!
//! Endpoints (all mounted under `/api/auth/mfa` so the existing CSRF
//! middleware applies):
//!
//! - `GET    /status`   — `{ enabled }` for the current user.
//! - `POST   /start`    — generate a fresh secret + recovery codes;
//!                        return them along with the `otpauth://` URI
//!                        for the dashboard to render as a QR. Nothing
//!                        is persisted yet — this lets the user back
//!                        out without leaving the account in a half-
//!                        enrolled state.
//! - `POST   /confirm`  — body `{ secret, code, recovery_codes }`.
//!                        Verifies the code against the candidate
//!                        secret, then persists the secret + the
//!                        SHA-256 hashes of the recovery codes.
//! - `POST   /verify`   — body `{ code }`. Used by the post-OAuth
//!                        challenge page: takes a pending-MFA cookie,
//!                        verifies the TOTP code (or burns a recovery
//!                        code), and re-issues a full-session JWT.
//! - `POST   /disable`  — body `{ code }`. Self-service disable —
//!                        requires a fresh TOTP code so a stolen
//!                        session cookie can't drop the second factor.
//!
//! Storage:
//!
//! - `users.totp_secret`            base32 string (no padding).
//! - `users.totp_recovery_hashes`   JSON array of hex-encoded SHA-256
//!                                  hashes of the *upper-cased*
//!                                  recovery code with dashes
//!                                  stripped.
//! - `users.totp_enabled`           1 once `confirm` succeeds.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;

use crate::{AppState, auth};

const TOTP_PERIOD: u64 = 30;
const TOTP_DIGITS: u32 = 6;
const TOTP_ISSUER: &str = "shellfleet";
/// ±1 30s window — total acceptance window 90s, the standard skew
/// budget per RFC 6238 §6 advice.
const TOTP_SKEW_STEPS: i64 = 1;
const RECOVERY_CODE_COUNT: usize = 10;
const SECRET_BYTES: usize = 20; // 160-bit, the RFC-recommended minimum

type HmacSha1 = Hmac<Sha1>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(status_handler))
        .route("/start", post(start_handler))
        .route("/confirm", post(confirm_handler))
        .route("/verify", post(verify_handler))
        .route("/disable", post(disable_handler))
}

// ---------------------------------------------------------------------
// TOTP primitives
// ---------------------------------------------------------------------

/// Generate a new 160-bit secret encoded as base32 (no padding) — the
/// form accepted by every authenticator app and the `otpauth://` URI.
fn generate_secret_b32() -> String {
    let mut buf = [0u8; SECRET_BYTES];
    rand::thread_rng().fill_bytes(&mut buf);
    BASE32_NOPAD.encode(&buf)
}

/// HOTP per RFC 4226. `counter` is the moving factor; for TOTP the
/// caller divides Unix time by the period.
fn hotp(secret_bytes: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret_bytes).expect("hmac key");
    mac.update(&counter.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let code = ((u32::from(hash[offset]) & 0x7f) << 24)
        | ((u32::from(hash[offset + 1]) & 0xff) << 16)
        | ((u32::from(hash[offset + 2]) & 0xff) << 8)
        | (u32::from(hash[offset + 3]) & 0xff);
    code % 10u32.pow(TOTP_DIGITS)
}

fn current_step(now: u64) -> u64 {
    now / TOTP_PERIOD
}

/// Returns true if `code` matches the TOTP for `secret_b32` at the
/// current step or within ±TOTP_SKEW_STEPS of it.
fn verify_totp(secret_b32: &str, code: &str) -> bool {
    let Ok(secret_bytes) = BASE32_NOPAD.decode(secret_b32.to_ascii_uppercase().as_bytes()) else {
        return false;
    };
    let code = code.trim();
    if code.len() != TOTP_DIGITS as usize || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let step = current_step(now) as i64;
    for delta in -TOTP_SKEW_STEPS..=TOTP_SKEW_STEPS {
        let candidate_step = step.saturating_add(delta).max(0) as u64;
        let candidate = format!(
            "{:0width$}",
            hotp(&secret_bytes, candidate_step),
            width = TOTP_DIGITS as usize
        );
        if bool::from(candidate.as_bytes().ct_eq(code.as_bytes())) {
            return true;
        }
    }
    false
}

/// Build an `otpauth://totp/...` URI for the authenticator app.
fn otpauth_uri(login: &str, secret_b32: &str) -> String {
    let label = format!("{TOTP_ISSUER}:{login}");
    format!(
        "otpauth://totp/{label}?secret={secret_b32}&issuer={TOTP_ISSUER}&algorithm=SHA1&digits={TOTP_DIGITS}&period={TOTP_PERIOD}",
        label = urlencoding::encode(&label),
        secret_b32 = secret_b32,
        TOTP_ISSUER = TOTP_ISSUER,
        TOTP_DIGITS = TOTP_DIGITS,
        TOTP_PERIOD = TOTP_PERIOD,
    )
}

// ---------------------------------------------------------------------
// Recovery codes
// ---------------------------------------------------------------------

fn generate_recovery_codes() -> Vec<String> {
    // 10 codes of the form XXXX-XXXX (8 base32 chars, dash for
    // readability). We keep the alphabet to base32 so the codes are
    // unambiguous on paper / in a screenshot.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut rng = rand::thread_rng();
    (0..RECOVERY_CODE_COUNT)
        .map(|_| {
            let mut buf = [0u8; 8];
            rng.fill_bytes(&mut buf);
            let chars: String = buf
                .iter()
                .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
                .collect();
            format!("{}-{}", &chars[0..4], &chars[4..8])
        })
        .collect()
}

fn normalize_recovery(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn hash_recovery(code: &str) -> String {
    let mut h = Sha256::new();
    h.update(normalize_recovery(code).as_bytes());
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

fn hashes_for_codes(codes: &[String]) -> Vec<String> {
    codes.iter().map(|c| hash_recovery(c)).collect()
}

// ---------------------------------------------------------------------
// Endpoint handlers
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct StatusResponse {
    enabled: bool,
}

async fn status_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if auth::is_dev_mode() {
        return Json(StatusResponse { enabled: false }).into_response();
    }
    // Allow status checks even from a pending-MFA session — the UI
    // needs to know whether the user is mid-enrollment.
    let cookie = match jar.get(auth::session_cookie_name()) {
        Some(c) => c,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };
    let claims = match auth::claims_from_token(cookie.value()) {
        Some(c) => c,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };
    let enabled = match crate::db::get_user(&state.db, &claims.sub).await {
        Ok(Some(row)) if claims.iat >= row.session_epoch => row.totp_enabled != 0,
        Ok(Some(_)) => {
            return (StatusCode::UNAUTHORIZED, "session revoked — please sign in again")
                .into_response();
        }
        Ok(None) => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
        Err(error) => {
            tracing::error!(%error, login = %claims.sub, "session verification failed for mfa status");
            return (StatusCode::SERVICE_UNAVAILABLE, "session verification unavailable")
                .into_response();
        }
    };
    Json(StatusResponse { enabled }).into_response()
}

#[derive(Serialize)]
struct StartResponse {
    secret: String,
    otpauth_uri: String,
    recovery_codes: Vec<String>,
}

async fn start_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let claims = match auth::current_user(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };
    let secret = generate_secret_b32();
    let codes = generate_recovery_codes();
    let uri = otpauth_uri(&claims.sub, &secret);
    Json(StartResponse {
        secret,
        otpauth_uri: uri,
        recovery_codes: codes,
    })
    .into_response()
}

#[derive(Deserialize)]
struct ConfirmRequest {
    secret: String,
    code: String,
    recovery_codes: Vec<String>,
}

async fn confirm_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<ConfirmRequest>,
) -> impl IntoResponse {
    let claims = match auth::current_user(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };
    if !verify_totp(&body.secret, &body.code) {
        return (StatusCode::BAD_REQUEST, "invalid code").into_response();
    }
    if body.recovery_codes.len() != RECOVERY_CODE_COUNT {
        return (StatusCode::BAD_REQUEST, "wrong recovery code count").into_response();
    }
    // Encrypt sensitive columns at rest so a DB-only backup leak
    // doesn't expose TOTP secrets or recovery-code hashes.
    let encrypted_secret = match crate::crypto::encrypt(&body.secret) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "failed to encrypt TOTP secret");
            return (StatusCode::INTERNAL_SERVER_ERROR, "credential encryption unavailable")
                .into_response();
        }
    };
    let hashes = hashes_for_codes(&body.recovery_codes);
    let hashes_json = serde_json::to_string(&hashes).unwrap_or_else(|_| "[]".into());
    let encrypted_hashes = match crate::crypto::encrypt(&hashes_json) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "failed to encrypt recovery hashes");
            return (StatusCode::INTERNAL_SERVER_ERROR, "credential encryption unavailable")
                .into_response();
        }
    };
    if let Err(e) =
        crate::db::set_user_totp(&state.db, &claims.sub, &encrypted_secret, &encrypted_hashes).await
    {
        tracing::error!(error = %e, "failed to enable totp");
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }
    crate::db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&claims.sub),
        None,
        "auth.mfa.enabled",
        true,
        None,
    )
    .await;
    (StatusCode::OK, "ok").into_response()
}

#[derive(Deserialize)]
struct VerifyRequest {
    code: String,
}

#[derive(Serialize)]
struct VerifyResponse {
    ok: bool,
    used_recovery: bool,
}

async fn verify_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<VerifyRequest>,
) -> impl IntoResponse {
    if auth::is_dev_mode() {
        return Json(VerifyResponse {
            ok: true,
            used_recovery: false,
        })
        .into_response();
    }
    let pending = match auth::pending_mfa_user(&jar, &state.db).await {
        Ok(c) => c,
        Err(error) => return error.into_response(),
    };

    // Brute-force defence: lock the per-login MFA throttle after
    // MAX_FAILS bad codes. The pending-MFA cookie has a 10-minute TTL,
    // so the only way to attempt this many guesses is sustained access
    // to the victim's browser session — and we'd much rather force a
    // re-login at that point.
    let now = crate::now_unix();
    if let crate::throttle::CheckResult::Locked { retry_after_secs } =
        state.mfa_throttle.check(&pending.sub, now)
    {
        crate::db::record_audit(
            &state.db,
            now,
            Some(&pending.sub),
            None,
            "auth.mfa.locked",
            false,
            Some(&format!("retry_after={retry_after_secs}s")),
        )
        .await;
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(
                axum::http::header::RETRY_AFTER,
                retry_after_secs.to_string(),
            )],
            "too many failed attempts; try again later",
        )
            .into_response();
    }
    let row = match crate::db::get_user(&state.db, &pending.sub).await {
        Ok(Some(r)) if r.totp_enabled != 0 => r,
        _ => return (StatusCode::BAD_REQUEST, "totp not enabled").into_response(),
    };
    // Decrypt the AES-GCM-protected secret. None means the ciphertext
    // is corrupt or was encrypted with a different key (e.g. JWT_SECRET
    // rotated without a TOTP re-enroll). Treat that like a missing
    // secret — the user has to re-enroll out-of-band.
    let secret_plain = row.totp_secret.as_deref().and_then(crate::crypto::decrypt);
    let secret = match secret_plain.as_deref() {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, "totp not enabled").into_response(),
    };

    let mut used_recovery = false;
    let totp_ok = verify_totp(secret, &body.code);
    let recovery_ok = if !totp_ok {
        // Decrypt the recovery-codes ciphertext, then JSON-parse the
        // inner array of hashes.
        let hashes_plain = crate::crypto::decrypt(&row.totp_recovery_hashes).unwrap_or_default();
        let mut hashes: Vec<String> = serde_json::from_str(&hashes_plain).unwrap_or_default();
        let target = hash_recovery(&body.code);
        // Constant-time compare against EVERY stored hash so the loop
        // takes the same time regardless of which (if any) matches.
        // String `==` would short-circuit and leak match position via
        // timing.
        let target_bytes = target.as_bytes();
        let mut found_idx: Option<usize> = None;
        for (i, h) in hashes.iter().enumerate() {
            let eq: bool = h.as_bytes().ct_eq(target_bytes).into();
            if eq && found_idx.is_none() {
                found_idx = Some(i);
            }
        }
        if let Some(idx) = found_idx {
            hashes.remove(idx);
            let new_json = serde_json::to_string(&hashes).unwrap_or_else(|_| "[]".into());
            match crate::crypto::encrypt(&new_json) {
                Ok(new_encrypted) => {
                    if let Err(e) = crate::db::update_user_recovery_hashes(
                        &state.db,
                        &pending.sub,
                        &new_encrypted,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "failed to burn recovery code");
                    }
                }
                Err(error) => tracing::error!(%error, "failed to encrypt recovery hashes"),
            }
            used_recovery = true;
            true
        } else {
            false
        }
    } else {
        false
    };

    if !totp_ok && !recovery_ok {
        state.mfa_throttle.record_failure(&pending.sub, now);
        crate::db::record_audit(
            &state.db,
            now,
            Some(&pending.sub),
            None,
            "auth.mfa.fail",
            false,
            None,
        )
        .await;
        return (StatusCode::UNAUTHORIZED, "invalid code").into_response();
    }

    state.mfa_throttle.record_success(&pending.sub);
    let role = auth::Role::parse(&row.role);
    let token = match auth::issue_jwt(&pending.sub, role, true, 24 * 3600) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to issue post-mfa jwt");
            return (StatusCode::INTERNAL_SERVER_ERROR, "jwt error").into_response();
        }
    };
    crate::db::record_audit(
        &state.db,
        crate::now_unix(),
        Some(&pending.sub),
        None,
        if used_recovery {
            "auth.mfa.recovery"
        } else {
            "auth.mfa.ok"
        },
        true,
        None,
    )
    .await;

    let cookie = auth::build_session_cookie(token);
    (
        jar.add(cookie),
        Json(VerifyResponse {
            ok: true,
            used_recovery,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct DisableRequest {
    code: String,
}

async fn disable_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<DisableRequest>,
) -> impl IntoResponse {
    let claims = match auth::current_user(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };
    let row = match crate::db::get_user(&state.db, &claims.sub).await {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "no such user").into_response(),
    };
    if row.totp_enabled == 0 {
        return (StatusCode::OK, "already disabled").into_response();
    }
    let now = crate::now_unix();
    if let crate::throttle::CheckResult::Locked { retry_after_secs } =
        state.mfa_disable_throttle.check(&claims.sub, now)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(
                axum::http::header::RETRY_AFTER,
                retry_after_secs.to_string(),
            )],
            "too many failed attempts; try again later",
        )
            .into_response();
    }
    // The stored `totp_secret` is AES-GCM ciphertext (`v1:...`) since
    // the at-rest encryption rolled in. Decrypt before handing to
    // `verify_totp`, which expects a base32 string. Without this the
    // call would silently fail every code and lock legitimate users
    // out of disabling 2FA.
    let secret_plain = row.totp_secret.as_deref().and_then(crate::crypto::decrypt);
    let secret = match secret_plain.as_deref() {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, "totp not enabled").into_response(),
    };
    if !verify_totp(secret, &body.code) {
        state.mfa_disable_throttle.record_failure(&claims.sub, now);
        return (StatusCode::UNAUTHORIZED, "invalid code").into_response();
    }
    state.mfa_disable_throttle.record_success(&claims.sub);
    if let Err(e) = crate::db::clear_user_totp(&state.db, &claims.sub).await {
        tracing::error!(error = %e, "failed to disable totp");
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }
    // Bump session_epoch so that disabling 2FA also invalidates any
    // other sessions the user has open — defence-in-depth against the
    // case where one of them was leaked and the 2FA-removal is part
    // of an attacker's lock-down sequence.
    let _ = crate::db::bump_session_epoch(&state.db, &claims.sub, now).await;
    crate::db::record_audit(
        &state.db,
        now,
        Some(&claims.sub),
        None,
        "auth.mfa.disabled",
        true,
        None,
    )
    .await;
    (StatusCode::OK, "ok").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_totp_accepts_current_code_and_rejects_others() {
        let secret = "JBSWY3DPEHPK3PXP"; // RFC-style base32 secret
        let secret_bytes = BASE32_NOPAD.decode(secret.as_bytes()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let step = current_step(now);
        let code = format!(
            "{:0width$}",
            hotp(&secret_bytes, step),
            width = TOTP_DIGITS as usize
        );
        assert!(verify_totp(secret, &code), "the current TOTP must verify");
        // A code many steps away is outside the ±1 acceptance window.
        let far = format!(
            "{:0width$}",
            hotp(&secret_bytes, step + 100),
            width = TOTP_DIGITS as usize
        );
        assert!(
            !verify_totp(secret, &far),
            "out-of-window code must be rejected"
        );
    }

    #[test]
    fn verify_totp_rejects_malformed_inputs() {
        assert!(!verify_totp("JBSWY3DPEHPK3PXP", "not-a-number"));
        assert!(!verify_totp("JBSWY3DPEHPK3PXP", ""));
        assert!(!verify_totp("!!! not base32 !!!", "123456"));
    }

    #[test]
    fn totp_skew_window_accepts_adjacent_steps() {
        let secret = "JBSWY3DPEHPK3PXP";
        let secret_bytes = BASE32_NOPAD.decode(secret.as_bytes()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let step = current_step(now);
        // Previous and next step codes must both be accepted (±1 skew).
        let prev = format!(
            "{:0width$}",
            hotp(&secret_bytes, step - 1),
            width = TOTP_DIGITS as usize
        );
        let next = format!(
            "{:0width$}",
            hotp(&secret_bytes, step + 1),
            width = TOTP_DIGITS as usize
        );
        assert!(verify_totp(secret, &prev), "previous step within skew");
        assert!(verify_totp(secret, &next), "next step within skew");
    }
}
