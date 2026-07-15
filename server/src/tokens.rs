use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AppState, auth, db};

#[derive(Serialize)]
struct TokenRow {
    token_preview: String,
    hostname: Option<String>,
    created_at: i64,
    last_seen: i64,
    /// Access-token expiry (unix seconds). 0 for pre-rotation rows.
    expires_at: i64,
    /// Refresh-token expiry (unix seconds). 0 for pre-rotation rows.
    refresh_expires_at: i64,
    /// 1 if the token has been revoked, else 0.
    revoked: i64,
}

#[derive(Deserialize)]
struct RevokeRequest {
    /// Full token string. Takes priority over `hostname` if both are sent.
    #[serde(default)]
    token: Option<String>,
    /// Hostname previously announced by an agent. Useful from the dashboard
    /// where the operator only sees the token preview.
    #[serde(default)]
    hostname: Option<String>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_tokens))
        .route("/revoke", post(revoke_token))
}

async fn require_auth(jar: &CookieJar, state: &AppState) -> Option<String> {
    auth::current_user(jar, &state.db).await.ok().map(|claims| claims.sub)
}

fn preview(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 12 {
        return "*".repeat(chars.len());
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

async fn list_tokens(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if require_auth(&jar, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let rows = match db::list_tokens(&state.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to list tokens");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let out: Vec<TokenRow> = rows
        .into_iter()
        .map(|r| TokenRow {
            token_preview: preview(&r.token),
            hostname: r.hostname,
            created_at: r.created_at,
            last_seen: r.last_seen,
            expires_at: r.expires_at,
            refresh_expires_at: r.refresh_expires_at,
            revoked: r.revoked,
        })
        .collect();
    Json(out).into_response()
}

async fn revoke_token(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(req): Json<RevokeRequest>,
) -> impl IntoResponse {
    let Some(actor) = require_auth(&jar, &state).await else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    if req.token.is_none() && req.hostname.is_none() {
        return (StatusCode::BAD_REQUEST, "Provide either token or hostname").into_response();
    }

    let removed_any = if let Some(t) = req.token.as_ref() {
        match db::revoke_token(&state.db, t).await {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "revoke by token failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    } else if let Some(h) = req.hostname.as_ref() {
        match db::revoke_by_hostname(&state.db, h).await {
            Ok(n) => n > 0,
            Err(e) => {
                tracing::error!(error = %e, "revoke by hostname failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    } else {
        false
    };

    if removed_any {
        tracing::info!(
            token = ?req.token.as_deref().map(preview),
            hostname = ?req.hostname,
            actor = %actor,
            "token revoked",
        );
        let detail = match (&req.token, &req.hostname) {
            (Some(t), _) => format!("token={}", preview(t)),
            (_, Some(h)) => format!("hostname={h}"),
            _ => String::new(),
        };
        db::record_audit(
            &state.db,
            crate::now_unix(),
            Some(&actor),
            req.hostname.as_deref(),
            "token.revoke",
            true,
            Some(&detail),
        )
        .await;
        return (StatusCode::OK, "Revoked").into_response();
    }
    (StatusCode::NOT_FOUND, "No matching token").into_response()
}
