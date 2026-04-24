use axum::{
    extract::State,
    response::IntoResponse,
    routing::post,
    Router,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::{SystemTime, UNIX_EPOCH}};

use crate::{AppState, auth::verify_token};

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
    Token { access_token: String, token_type: String },
    Error { error: String },
}

#[derive(Deserialize)]
pub struct ApproveDeviceRequest {
    pub user_code: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/request", post(request_device_auth))
        .route("/token", post(poll_device_token))
        .route("/approve", post(approve_device))
}

async fn request_device_auth(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (device_code, user_code) = {
        let mut rng = rand::thread_rng();
        
        // Generate codes
        let d_code: String = (&mut rng).sample_iter(&Alphanumeric).take(32).map(char::from).collect();
        
        // User code usually easy to read, e.g. 8 uppercase letters split by dash
        let raw_user_code: String = (&mut rng).sample_iter(&Alphanumeric).take(8).map(char::from).collect();
        let u_code = format!("{}-{}", &raw_user_code[0..4], &raw_user_code[4..8]).to_uppercase();
        
        (d_code, u_code)
    };

    let expires_in = 300; // 5 minutes
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() + expires_in;

    let pending = crate::PendingDevice {
        device_code: device_code.clone(),
        user_code: user_code.clone(),
        expires_at,
        approved: false,
    };

    state.pending_devices.write().await.insert(device_code.clone(), pending);
    state.user_codes.write().await.insert(user_code.clone(), device_code.clone());

    let verification_uri = std::env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string()) + "device";

    Json(DeviceAuthResponse {
        device_code,
        user_code,
        verification_uri,
        expires_in,
        interval: 5,
    })
}

async fn poll_device_token(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeviceTokenRequest>,
) -> impl IntoResponse {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let mut pending_devices = state.pending_devices.write().await;
    
    if let Some(device) = pending_devices.get_mut(&payload.device_code) {
        if now > device.expires_at {
            // Expired
            let uc = device.user_code.clone();
            pending_devices.remove(&payload.device_code);
            state.user_codes.write().await.remove(&uc);
            return Json(DeviceTokenResponse::Error { error: "expired_token".to_string() });
        }

        if device.approved {
            // Generate permanent token
            let token = {
                let mut rng = rand::thread_rng();
                let t: String = (&mut rng).sample_iter(&Alphanumeric).take(64).map(char::from).collect();
                t
            };
            
            // Save token
            state.approved_tokens.write().await.insert(token.clone(), true);
            let _ = crate::save_tokens(&*state.approved_tokens.read().await);

            // Cleanup pending
            let uc = device.user_code.clone();
            pending_devices.remove(&payload.device_code);
            state.user_codes.write().await.remove(&uc);

            return Json(DeviceTokenResponse::Token {
                access_token: token,
                token_type: "bearer".to_string(),
            });
        } else {
            return Json(DeviceTokenResponse::Error { error: "authorization_pending".to_string() });
        }
    }

    Json(DeviceTokenResponse::Error { error: "invalid_grant".to_string() })
}

async fn approve_device(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ApproveDeviceRequest>,
) -> impl IntoResponse {
    // Authenticate Web UI User
    let mut is_authenticated = false;
    if std::env::var("JWT_SECRET").unwrap_or_default() == "dev" {
        is_authenticated = true;
    } else if let Some(cookie) = jar.get("auth_token") {
        if verify_token(cookie.value()) {
            is_authenticated = true;
        }
    }

    if !is_authenticated {
        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let user_code = payload.user_code.to_uppercase();
    
    let device_code_opt = state.user_codes.read().await.get(&user_code).cloned();

    if let Some(device_code) = device_code_opt {
        let mut pending = state.pending_devices.write().await;
        if let Some(device) = pending.get_mut(&device_code) {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            if now <= device.expires_at {
                device.approved = true;
                return (axum::http::StatusCode::OK, "Approved").into_response();
            }
        }
    }

    (axum::http::StatusCode::BAD_REQUEST, "Invalid or expired code").into_response()
}
