use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::AppState;

pub async fn middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let Some(ref token) = auth_header else {
        return next.run(req).await;
    };

    if !token.starts_with("sf_live_") {
        return next.run(req).await;
    }

    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    };
    let now = crate::now_unix();

    #[derive(sqlx::FromRow)]
    struct KeyRow {
        id: i64,
        login: String,
        policy_id: Option<i64>,
        expires_at: Option<i64>,
    }

    let row: Option<KeyRow> = sqlx::query_as(
        "SELECT id, login, policy_id, expires_at FROM ee_api_keys WHERE key_hash = ?1 AND revoked = 0",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let Some(row) = row else {
        return (StatusCode::UNAUTHORIZED, "invalid API key").into_response();
    };

    if let Some(exp) = row.expires_at {
        if exp < now {
            return (StatusCode::UNAUTHORIZED, "API key expired").into_response();
        }
    }

    let _ = sqlx::query("UPDATE ee_api_keys SET last_used_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(row.id)
        .execute(&state.db)
        .await;

    let role = match crate::db::get_user(&state.db, &row.login).await {
        Ok(Some(user)) => user.role,
        _ => "viewer".to_string(),
    };

    let headers = req.headers_mut();
    if let Ok(v) = axum::http::HeaderValue::from_str(&row.login) {
        headers.insert(
            axum::http::HeaderName::from_static("x-api-key-login"),
            v,
        );
    }
    if let Ok(v) = axum::http::HeaderValue::from_str(&role) {
        headers.insert(
            axum::http::HeaderName::from_static("x-api-key-role"),
            v,
        );
    }
    if let Some(pid) = row.policy_id {
        if let Ok(v) = axum::http::HeaderValue::from_str(&pid.to_string()) {
            headers.insert(
                axum::http::HeaderName::from_static("x-api-key-policy-id"),
                v,
            );
        }
    }

    next.run(req).await
}
