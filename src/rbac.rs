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

use crate::{AppState, auth};

/// Exact-or-`/`-bounded match for the api-keys route, post-`/api`-strip form.
/// Never a bare `starts_with("/ee/keys")` (would swallow `/ee/keys-extra`).
fn is_api_keys_path(path: &str) -> bool {
    path == "/ee/keys" || path.starts_with("/ee/keys/")
}

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

    // This middleware is mounted *inside* the `/api` nest, so axum
    // strips the prefix before the request reaches us — `path` is
    // `/me`, `/auth/mfa/verify`, `/device/request`, etc., NOT
    // `/api/me`. The whitelist below must therefore match the
    // post-strip form. (Earlier versions used the `/api/...` form
    // and silently let RBAC gate /api/me + the MFA verify path,
    // which broke the MFA flow.)
    //
    // Whitelist:
    //   /me                       — session probe; needed during the
    //                               pending-MFA window.
    //   /auth/mfa/...             — the MFA endpoints themselves
    //                               handle pending-MFA cookies.
    //   /device/request, /token   — agent pairing handshake; the
    //                               agent has no cookie at this
    //                               point and shouldn't need one.
    //                               /device/approve is NOT in the
    //                               whitelist — that's admin-only.
    if path == "/me"
        || path.starts_with("/auth/mfa/")
        || path.starts_with("/auth/passkey/")
        || path == "/device/request"
        || path == "/device/token"
    {
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
    let user_row = crate::db::get_user(&state.db, &claims.sub)
        .await
        .ok()
        .flatten();
    if let Some(ref row) = user_row {
        if claims.iat < row.session_epoch {
            return unauthorized("session revoked — please sign in again");
        }
    }

    // Per-user API key self-service: viewers may create/revoke/update their OWN
    // keys. EE scopes every mutation by the CE-injected login, so this only
    // lets a viewer manage their own keys — never escalation. All other guards
    // (auth, session-epoch, MFA above) still applied.
    if is_mutating(&method) && !is_api_keys_path(&path) {
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

#[cfg(test)]
mod tests {
    use super::is_api_keys_path;

    #[test]
    fn matches_keys_routes_segment_bounded() {
        assert!(is_api_keys_path("/ee/keys"));
        assert!(is_api_keys_path("/ee/keys/1"));
        assert!(!is_api_keys_path("/ee/keys-extra"));
        assert!(!is_api_keys_path("/ee/keysX"));
        assert!(!is_api_keys_path("/ee/metrics/panels"));
    }
}
