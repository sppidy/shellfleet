use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::{delete, get},
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use std::sync::Arc;

use crate::{AppState, auth, db};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_invites).post(create_invite))
        .route("/{code}", delete(delete_invite))
}

pub fn public_routes() -> Router<Arc<AppState>> {
    Router::new().route("/invite/{code}", get(accept_invite))
}

fn generate_code() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
struct CreateRequest {
    #[serde(default = "default_role")]
    role: String,
    #[serde(default = "default_ttl")]
    ttl_hours: i64,
}

fn default_role() -> String {
    "viewer".into()
}
fn default_ttl() -> i64 {
    24
}

fn invite_created_audit_detail(role: &str, expires_at: i64) -> String {
    format!("role={role} expires_at={expires_at}")
}

async fn create_invite(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<CreateRequest>,
) -> impl IntoResponse {
    if !crate::ee::ee_active() {
        return (StatusCode::NOT_FOUND, "requires Enterprise Edition").into_response();
    }
    let claims = match auth::require_admin(&jar, &state.db).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let role = match body.role.as_str() {
        "admin" | "viewer" => body.role.as_str(),
        _ => "viewer",
    };

    let code = generate_code();
    let now = crate::now_unix();
    let expires_at = now + body.ttl_hours * 3600;

    match db::create_invite(&state.db, &code, role, &claims.sub, expires_at).await {
        Ok(_) => {
            db::record_audit(
                &state.db,
                now,
                Some(&claims.sub),
                None,
                "invite.created",
                true,
                Some(&invite_created_audit_detail(role, expires_at)),
            )
            .await;
            (
                StatusCode::CREATED,
                axum::Json(serde_json::json!({
                    "code": code,
                    "role": role,
                    "expires_at": expires_at,
                    "url": format!("/invite/{code}"),
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "create invite failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed").into_response()
        }
    }
}

async fn list_invites(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if !crate::ee::ee_active() {
        return (StatusCode::NOT_FOUND, "requires Enterprise Edition").into_response();
    }
    if let Err(e) = auth::require_admin(&jar, &state.db).await {
        return e.into_response();
    }
    match db::list_invites(&state.db).await {
        Ok(invites) => axum::Json(invites).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list invites failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed").into_response()
        }
    }
}

async fn delete_invite(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = auth::require_admin(&jar, &state.db).await {
        return e.into_response();
    }
    match db::delete_invite(&state.db, &code).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete invite failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed").into_response()
        }
    }
}

async fn accept_invite(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    let invite = match db::get_invite(&state.db, &code).await {
        Ok(Some(inv)) => inv,
        _ => return (StatusCode::NOT_FOUND, "invalid or expired invite link").into_response(),
    };

    let now = crate::now_unix();
    if now > invite.expires_at {
        return (StatusCode::GONE, "this invite link has expired").into_response();
    }
    if invite.used_by.is_some() {
        return (StatusCode::GONE, "this invite has already been used").into_response();
    }

    // If the user is already logged in, redeem immediately
    if let Ok(claims) = auth::current_user(&jar, &state.db).await {
        // Redeem the invite for this user
        let _ = db::redeem_invite(&state.db, &code, &claims.sub).await;
        db::record_audit(
            &state.db,
            now,
            Some(&claims.sub),
            None,
            "invite.redeemed",
            true,
            None,
        )
        .await;
        let ui_url = std::env::var("UI_URL").unwrap_or_else(|_| "/".into());
        return Redirect::temporary(&ui_url).into_response();
    }

    // Not logged in — redirect to login with invite code in a cookie/param
    // After login, the callback will check for a pending invite
    let login_url = format!("/auth/login?invite={code}");
    Redirect::temporary(&login_url).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn invite_audit_detail_contains_metadata_not_bearer_code() {
        let detail = super::invite_created_audit_detail("admin", 12345);
        assert_eq!(detail, "role=admin expires_at=12345");
        assert!(!detail.contains("invite-secret"));
    }
}
