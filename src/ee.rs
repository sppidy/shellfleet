use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{auth, db, AppState};

pub fn ee_sidecar_url() -> Option<String> {
    std::env::var("EE_SIDECAR_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn ee_active() -> bool {
    ee_sidecar_url().is_some()
}

fn internal_secret() -> Option<String> {
    std::env::var("EE_INTERNAL_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

fn verify_internal_auth(headers: &HeaderMap) -> bool {
    let Some(expected) = internal_secret() else {
        return false;
    };
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|t| t.trim() == expected)
        .unwrap_or(false)
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/resolve", post(auth_resolve_handler))
        .route("/seat-limit", post(seat_limit_handler))
        .route("/agents", get(agents_handler))
}

#[derive(Deserialize)]
struct AuthResolveRequest {
    login: String,
    #[serde(default = "default_role")]
    role: String,
    #[serde(default)]
    mfa: bool,
}

fn default_role() -> String {
    "viewer".into()
}

#[derive(Serialize)]
struct AuthResolveResponse {
    token: String,
}

async fn auth_resolve_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<AuthResolveRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let role = match body.role.as_str() {
        "admin" | "viewer" => body.role.as_str(),
        _ => "viewer",
    };

    let now = crate::now_unix();
    let seat_limit = db::seat_limit(&state.db).await;
    match db::upsert_login_with_seat_check(&state.db, &body.login, role, now, seat_limit).await {
        Ok(db::SeatedUpsert::SeatCapReached) => {
            return (StatusCode::FORBIDDEN, "seat cap reached").into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "internal auth resolve: db error");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
        Ok(_) => {}
    }

    let token = auth::issue_internal_jwt(&body.login, role, body.mfa);
    (StatusCode::OK, axum::Json(AuthResolveResponse { token })).into_response()
}

#[derive(Deserialize)]
struct SeatLimitRequest {
    seats: i64,
}

async fn seat_limit_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<SeatLimitRequest>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    match db::set_ee_seat_limit(&state.db, body.seats).await {
        Ok(_) => {
            tracing::info!(seats = body.seats, "EE seat limit updated");
            (StatusCode::OK, "ok").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to set EE seat limit");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

#[derive(Serialize)]
struct AgentInfo {
    agent_id: String,
    capabilities: Vec<String>,
}

async fn agents_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if !verify_internal_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let map = state.agents.lock().await;
    let agents: Vec<AgentInfo> = map
        .iter()
        .map(|(id, entry)| AgentInfo {
            agent_id: id.clone(),
            capabilities: entry.capabilities.clone(),
        })
        .collect();
    (StatusCode::OK, axum::Json(agents)).into_response()
}

pub async fn ee_proxy_handler(req: Request) -> impl IntoResponse {
    let Some(ee_url) = ee_sidecar_url() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "EE not active").into_response();
    };
    let secret = internal_secret().unwrap_or_default();

    let path = req.uri().path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.uri().path());
    let url = format!("{}{}", ee_url.trim_end_matches('/'), path);
    let method = req.method().clone();

    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "body too large").into_response(),
    };

    let client = reqwest::Client::new();
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    )
    .bearer_auth(&secret)
    .timeout(std::time::Duration::from_secs(30));

    if !body_bytes.is_empty() {
        builder = builder
            .header("content-type", "application/json")
            .body(body_bytes.to_vec());
    }

    match builder.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "EE proxy failed");
            (StatusCode::BAD_GATEWAY, "EE sidecar unavailable").into_response()
        }
    }
}
