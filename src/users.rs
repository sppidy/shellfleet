//! CE user-management API for the admin UI.
//!
//! All routes require `admin` — the RBAC middleware on `/api/*`
//! guarantees that on mutating methods, but the GET handler also
//! re-checks because viewer-listing of users is not a CE feature
//! (avoids leaking the seat list to read-only operators).
//!
//! Endpoints:
//!   GET    /api/users          → list users + seat-cap headroom
//!   PUT    /api/users/:login   → { role: "admin" | "viewer" }
//!   DELETE /api/users/:login   → remove a seat (frees one up to the cap)
//!
//! Removing a user wipes their TOTP enrollment along with the row;
//! they'll be re-created as viewer on next OAuth (assuming there's
//! still a free seat).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{auth, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler))
        .route("/{login}", axum::routing::put(set_role_handler).delete(delete_handler))
}

#[derive(Serialize)]
struct ListResponse {
    users: Vec<crate::db::UserListRow>,
    seat_limit: i64,
    seats_used: usize,
}

async fn list_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let _ = match auth::require_admin(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };
    let users = match crate::db::list_users(&state.db).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "list users failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let seats_used = users.len();
    let seat_cap = crate::db::seat_limit(&state.db).await;
    Json(ListResponse {
        users,
        seat_limit: seat_cap,
        seats_used,
    })
    .into_response()
}

#[derive(Deserialize)]
struct SetRoleRequest {
    role: String,
}

async fn set_role_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(login): Path<String>,
    Json(body): Json<SetRoleRequest>,
) -> impl IntoResponse {
    let actor = match auth::require_admin(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };

    let new_role = match body.role.as_str() {
        "admin" | "viewer" => body.role.as_str(),
        _ => return (StatusCode::BAD_REQUEST, "role must be admin or viewer").into_response(),
    };

    // Guard: refuse to demote the last remaining admin so the operator
    // can't lock themselves out via the UI.
    if new_role == "viewer" {
        let admins = match crate::db::list_users(&state.db).await {
            Ok(u) => u.into_iter().filter(|r| r.role == "admin").count(),
            Err(e) => {
                tracing::error!(error = %e, "list users for demote check failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        };
        let target_is_admin = matches!(
            crate::db::get_user(&state.db, &login).await,
            Ok(Some(r)) if r.role == "admin"
        );
        if target_is_admin && admins <= 1 {
            return (
                StatusCode::CONFLICT,
                "cannot demote the last admin — promote someone else first",
            )
                .into_response();
        }
    }

    match crate::db::set_user_role(&state.db, &login, new_role).await {
        Ok(true) => {
            // Bump the target's session_epoch so any outstanding JWT
            // (still 24h valid) loses its role power on the next
            // request. Without this, a demoted admin would keep admin
            // until their cookie expires.
            let now = crate::now_unix();
            let _ = crate::db::bump_session_epoch(&state.db, &login, now).await;
            crate::db::record_audit(
                &state.db,
                now,
                Some(&actor.sub),
                None,
                "users.set_role",
                true,
                Some(&format!("login={login} role={new_role}")),
            )
            .await;
            (StatusCode::OK, "ok").into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "no such user").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_user_role failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn delete_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Path(login): Path<String>,
) -> impl IntoResponse {
    let actor = match auth::require_admin(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };

    if actor.sub == login {
        return (StatusCode::CONFLICT, "cannot remove your own seat").into_response();
    }

    // Prevent removing the last admin even if we somehow get here past
    // the demote guard.
    let target_is_admin = matches!(
        crate::db::get_user(&state.db, &login).await,
        Ok(Some(r)) if r.role == "admin"
    );
    if target_is_admin {
        let admins = match crate::db::list_users(&state.db).await {
            Ok(u) => u.into_iter().filter(|r| r.role == "admin").count(),
            Err(e) => {
                tracing::error!(error = %e, "list users for remove check failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        };
        if admins <= 1 {
            return (
                StatusCode::CONFLICT,
                "cannot remove the last admin — promote someone else first",
            )
                .into_response();
        }
    }

    match sqlx::query("DELETE FROM users WHERE login = ?")
        .bind(&login)
        .execute(&state.db)
        .await
    {
        Ok(res) if res.rows_affected() > 0 => {
            crate::db::record_audit(
                &state.db,
                crate::now_unix(),
                Some(&actor.sub),
                None,
                "users.delete",
                true,
                Some(&format!("login={login}")),
            )
            .await;
            (StatusCode::OK, "ok").into_response()
        }
        Ok(_) => (StatusCode::NOT_FOUND, "no such user").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete user failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}
