use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
use axum_extra::extract::cookie::CookieJar;
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AppState, auth, db};

/// Access-token lifetime. Short enough that a stolen access token
/// is useless within an hour; long enough that a healthy agent
/// reconnecting within the hour reuses it without a refresh round-trip.
const ACCESS_TOKEN_TTL_SECS: i64 = 3600;
/// Refresh-token lifetime. A stolen refresh token is single-use (rotated
/// on every refresh), so the window is one use; the 30-day cap bounds how
/// long an agent can stay paired without an operator re-approving it.
const REFRESH_TOKEN_TTL_SECS: i64 = 30 * 86400;

fn random_token() -> String {
    let mut rng = rand::thread_rng();
    (&mut rng)
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

#[derive(Serialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
pub struct DeviceTokenRequest {
    pub device_code: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum DeviceTokenResponse {
    Token {
        access_token: String,
        refresh_token: String,
        token_type: String,
        expires_in: i64,
        refresh_expires_in: i64,
    },
    Error {
        error: String,
    },
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Deserialize)]
pub struct ApproveDeviceRequest {
    pub user_code: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/request", post(request_device_auth))
        .route("/token", post(poll_device_token))
        .route("/refresh", post(refresh_device_token))
        .route("/approve", post(approve_device))
}

async fn request_device_auth(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (device_code, user_code) = {
        let mut rng = rand::thread_rng();
        let d_code: String = (&mut rng)
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        let raw_user_code: String = (&mut rng)
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let u_code = format!("{}-{}", &raw_user_code[0..4], &raw_user_code[4..8]).to_uppercase();
        (d_code, u_code)
    };

    let expires_in = 300; // 5 minutes
    let expires_at = crate::now_unix() + expires_in as i64;

    if let Err(e) = db::insert_pending_device(&state.db, &device_code, &user_code, expires_at).await
    {
        tracing::error!(error = %e, "insert pending device failed");
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }

    let verification_uri = std::env::var("UI_URL")
        .unwrap_or_else(|_| "https://dashboard.example.com/".to_string())
        + "device";

    Json(DeviceAuthResponse {
        device_code,
        user_code,
        verification_uri,
        expires_in,
        interval: 5,
    })
    .into_response()
}

async fn poll_device_token(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeviceTokenRequest>,
) -> impl IntoResponse {
    let now = crate::now_unix();
    let row = match db::pending_device(&state.db, &payload.device_code).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "pending_device lookup failed");
            return Json(DeviceTokenResponse::Error {
                error: "server_error".to_string(),
            })
            .into_response();
        }
    };

    let Some(row) = row else {
        return Json(DeviceTokenResponse::Error {
            error: "invalid_grant".to_string(),
        })
        .into_response();
    };

    if now > row.expires_at {
        let _ = db::delete_pending_device(&state.db, &row.device_code).await;
        return Json(DeviceTokenResponse::Error {
            error: "expired_token".to_string(),
        })
        .into_response();
    }

    if row.approved == 0 {
        return Json(DeviceTokenResponse::Error {
            error: "authorization_pending".to_string(),
        })
        .into_response();
    }

    // Approved — mint a short-lived access token + a single-use refresh
    // token. The access token expires in ACCESS_TOKEN_TTL_SECS; the
    // agent rotates both via /api/device/refresh before the access token
    // expires, and the refresh token is single-use (rotation reuses the
    // same row, so a replayed refresh token is rejected).
    let access_token = random_token();
    let refresh_token = random_token();
    let access_expires_at = now + ACCESS_TOKEN_TTL_SECS;
    let refresh_expires_at = now + REFRESH_TOKEN_TTL_SECS;

    if let Err(e) = db::insert_token(
        &state.db,
        &access_token,
        access_expires_at,
        &refresh_token,
        refresh_expires_at,
        now,
    )
    .await
    {
        tracing::error!(error = %e, "insert token failed");
        return Json(DeviceTokenResponse::Error {
            error: "server_error".to_string(),
        })
        .into_response();
    }
    let _ = db::delete_pending_device(&state.db, &row.device_code).await;

    db::record_audit(
        &state.db,
        now,
        None,
        None,
        "device.token.issued",
        true,
        Some(&format!("user_code={}", row.user_code)),
    )
    .await;

    Json(DeviceTokenResponse::Token {
        access_token,
        refresh_token,
        token_type: "bearer".to_string(),
        expires_in: ACCESS_TOKEN_TTL_SECS,
        refresh_expires_in: REFRESH_TOKEN_TTL_SECS,
    })
    .into_response()
}

/// Rotate an agent's access + refresh tokens. The agent calls this
/// before its access token expires (and reactively on a 401). The
/// presented refresh token is single-use: on success the old row is
/// deleted and a fresh pair is issued, so a replayed refresh token fails
/// the next lookup. A failed/expired/revoked refresh token returns
/// `invalid_grant`, which tells the agent to re-pair.
async fn refresh_device_token(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RefreshRequest>,
) -> impl IntoResponse {
    let now = crate::now_unix();
    let new_access = random_token();
    let new_refresh = random_token();
    let access_expires_at = now + ACCESS_TOKEN_TTL_SECS;
    let refresh_expires_at = now + REFRESH_TOKEN_TTL_SECS;

    match db::replace_token_on_refresh(
        &state.db,
        &payload.refresh_token,
        &new_access,
        &new_refresh,
        access_expires_at,
        refresh_expires_at,
        now,
    )
    .await
    {
        Ok(Some(_hostname)) => {
            db::record_audit(
                &state.db,
                now,
                None,
                None,
                "device.token.refreshed",
                true,
                None,
            )
            .await;
            Json(DeviceTokenResponse::Token {
                access_token: new_access,
                refresh_token: new_refresh,
                token_type: "bearer".to_string(),
                expires_in: ACCESS_TOKEN_TTL_SECS,
                refresh_expires_in: REFRESH_TOKEN_TTL_SECS,
            })
            .into_response()
        }
        Ok(None) => Json(DeviceTokenResponse::Error {
            error: "invalid_grant".to_string(),
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "refresh token rotation failed");
            Json(DeviceTokenResponse::Error {
                error: "server_error".to_string(),
            })
            .into_response()
        }
    }
}

async fn approve_device(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ApproveDeviceRequest>,
) -> impl IntoResponse {
    let actor = match auth::current_user(&jar, &state.db).await {
        Ok(claims) => claims.sub,
        Err(err) => return err.into_response(),
    };

    let now = crate::now_unix();

    // Brute-force defence on user_code (only ~36^8 ≈ 2.8e12 raw, but
    // the operator-approved attack surface is one approve per
    // 5-minute window per pending device — still cheap to guess
    // without a throttle).
    if let crate::throttle::CheckResult::Locked { retry_after_secs } =
        state.device_approve_throttle.check(&actor, now)
    {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            [(
                axum::http::header::RETRY_AFTER,
                retry_after_secs.to_string(),
            )],
            "too many failed attempts; try again later",
        )
            .into_response();
    }

    let user_code = payload.user_code.to_uppercase();
    let approved = match db::approve_user_code(&state.db, &user_code, now).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "approve_user_code failed");
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    if approved {
        state.device_approve_throttle.record_success(&actor);
        db::record_audit(
            &state.db,
            now,
            Some(&actor),
            None,
            "device.approve",
            true,
            Some(&format!("user_code={user_code}")),
        )
        .await;
        (axum::http::StatusCode::OK, "Approved").into_response()
    } else {
        state.device_approve_throttle.record_failure(&actor, now);
        db::record_audit(
            &state.db,
            now,
            Some(&actor),
            None,
            "device.approve.fail",
            false,
            None,
        )
        .await;
        (
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid or expired code",
        )
            .into_response()
    }
}
