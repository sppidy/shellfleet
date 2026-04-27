//! CE RBAC: a Tower middleware that enforces admin-only writes on
//! `/api/*` and authenticated reads everywhere else.
//!
//! Layered AFTER the CSRF middleware, so the order of checks for an
//! API request is: CSRF → RBAC → handler. The MFA endpoints under
//! `/api/auth/mfa/*` are skipped here because they need to accept a
//! pending-MFA cookie (and they enforce their own auth in-handler).
//!
//! Dev mode (`JWT_SECRET=dev`) short-circuits the entire layer so
//! local tooling continues to work.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{Method, Response, StatusCode},
    middleware::Next,
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::{auth, AppState};

fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    )
}

fn forbidden(reason: &'static str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(Body::from(reason))
        .unwrap()
}

fn unauthorized(reason: &'static str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from(reason))
        .unwrap()
}

pub async fn middleware(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response<Body> {
    if auth::is_dev_mode() {
        return next.run(req).await;
    }

    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // The MFA endpoints handle their own auth (verify accepts a
    // pending-MFA cookie that this layer would otherwise reject).
    // /api/me is also whitelisted so the dashboard can render
    // "you are <login>" during the MFA challenge.
    if path.starts_with("/api/auth/mfa/") || path == "/api/me" {
        return next.run(req).await;
    }

    let cookie = match jar.get("auth_token") {
        Some(c) => c,
        None => return unauthorized("Unauthorized"),
    };
    let claims = match auth::claims_from_token(cookie.value()) {
        Some(c) => c,
        None => return unauthorized("Unauthorized"),
    };
    if !claims.mfa {
        return forbidden("MFA required");
    }

    // Single DB lookup that handles both the session-epoch invalidation
    // check and the role re-resolution. Avoids a second hit for the
    // mutating-method branch.
    let user_row = crate::db::get_user(&state.db, &claims.sub).await.ok().flatten();
    if let Some(ref row) = user_row {
        if claims.iat < row.session_epoch {
            return unauthorized("session revoked — please sign in again");
        }
    }

    if is_mutating(&method) {
        let role_str = user_row
            .as_ref()
            .map(|r| r.role.clone())
            .unwrap_or_else(|| claims.role.clone());
        if auth::Role::parse(&role_str) != auth::Role::Admin {
            return forbidden("viewer role: read-only");
        }
    }

    next.run(req).await
}
