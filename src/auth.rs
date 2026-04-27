//! GitHub OAuth + JWT session, extended with the CE 2FA / RBAC story.
//!
//! Session lifecycle:
//!
//! 1. User clicks "Sign in with GitHub" → `/auth/login` redirects to GitHub.
//! 2. GitHub redirects back to `/auth/callback`. We exchange the code,
//!    fetch the GitHub login, and persist a `users` row (creating it on
//!    first contact). The very first user — or a login matching
//!    `BOOTSTRAP_ADMIN` — becomes admin; everyone else defaults to
//!    viewer.
//! 3. If the user has TOTP enabled we issue a *pending* JWT
//!    (`mfa = false`) and redirect to the MFA challenge page. The
//!    pending JWT is short-lived and only valid against
//!    `/api/auth/mfa/verify` (enforced in the handler, not the JWT
//!    layer — claims always include `mfa: bool`).
//! 4. After `/api/auth/mfa/verify` succeeds we re-issue a full JWT with
//!    `mfa = true`.
//!
//! `JWT_SECRET=dev` keeps the historical local-development backdoor:
//! every check passes and the implicit user is `dev` with role `admin`
//! and `mfa = true`. CSRF is similarly disabled in main.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub code: String,
    #[allow(dead_code)]
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Viewer => "viewer",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "admin" => Role::Admin,
            _ => Role::Viewer,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    /// CE: role at the moment the JWT was issued. Re-checked against
    /// the `users` row by `require_admin` so a demotion takes effect on
    /// the next request, not just the next login.
    #[serde(default = "default_role")]
    pub role: String,
    /// CE: `false` for a pending-MFA token, `true` for a fully verified
    /// session. Endpoints that mutate state require `true`.
    #[serde(default = "default_mfa")]
    pub mfa: bool,
}

fn default_role() -> String {
    "viewer".to_string()
}
fn default_mfa() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct GithubTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
}

pub fn auth_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/login", get(login_handler))
        .route("/callback", get(callback_handler))
        .route("/logout", get(logout_handler))
        .with_state(state)
}

fn allowed_users() -> Vec<String> {
    env::var("ALLOWED_GITHUB_USERS")
        .unwrap_or_else(|_| "sppidy".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_user_allowed(login: &str) -> bool {
    allowed_users().iter().any(|u| u == login)
}

fn cookie_secure() -> bool {
    match env::var("COOKIE_SECURE").ok().as_deref() {
        Some("0") | Some("false") | Some("no") => false,
        _ => true,
    }
}

fn jwt_secret() -> String {
    env::var("JWT_SECRET").unwrap_or_else(|_| "supersecretkey".to_string())
}

fn dev_mode() -> bool {
    env::var("JWT_SECRET").unwrap_or_default() == "dev"
}

async fn login_handler() -> impl IntoResponse {
    let client_id = env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| "dummy_id".to_string());
    let redirect_uri = env::var("OAUTH_REDIRECT_URL")
        .unwrap_or_else(|_| "https://dashboard.example.com/auth/callback".to_string());

    let redirect_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&state=sysmanager&scope=read:user",
        client_id,
        urlencoding::encode(&redirect_uri)
    );

    Redirect::temporary(&redirect_url)
}

async fn logout_handler(jar: CookieJar) -> impl IntoResponse {
    let mut cookie = Cookie::build(("auth_token", ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .build();
    cookie.make_removal();

    let ui_url = env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string());
    let login_url = format!("{}login", ui_url);
    (jar.add(cookie), Redirect::temporary(&login_url)).into_response()
}

/// Build a session cookie for the given JWT. Centralized so the post-MFA
/// reissue path uses the same flags as the OAuth callback.
pub fn build_session_cookie(token: String) -> Cookie<'static> {
    Cookie::build(("auth_token", token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .build()
}

/// Issue a JWT with the given role + MFA-verified bit. `ttl_secs`
/// controls the cookie lifetime; pending-MFA tokens use a much shorter
/// TTL than fully-verified ones.
pub fn issue_jwt(sub: &str, role: Role, mfa: bool, ttl_secs: i64) -> Result<String, String> {
    let exp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + ttl_secs) as usize;

    let claims = Claims {
        sub: sub.to_string(),
        exp,
        role: role.as_str().to_string(),
        mfa,
    };

    let secret = jwt_secret();
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

async fn callback_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthRequest>,
) -> Response {
    tracing::info!("github oauth callback");

    let client_id = env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| "dummy_id".to_string());
    let client_secret =
        env::var("GITHUB_CLIENT_SECRET").unwrap_or_else(|_| "dummy_secret".to_string());

    let client = reqwest::Client::new();

    let token_res = match client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", query.code),
        ])
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e, "failed to exchange oauth code");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get access token")
                .into_response();
        }
    };

    let token_data = match token_res.json::<GithubTokenResponse>().await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse oauth token response");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to parse access token")
                .into_response();
        }
    };

    let user_res = match client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token_data.access_token))
        .header("User-Agent", "sys-manager")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch github user");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get user profile")
                .into_response();
        }
    };

    let user_data = match user_res.json::<GithubUser>().await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse github user");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to parse user profile")
                .into_response();
        }
    };

    if !is_user_allowed(&user_data.login) {
        tracing::warn!(login = %user_data.login, "github login not on allowlist");
        return (
            StatusCode::UNAUTHORIZED,
            "Unauthorized user. Your GitHub account is not on the allowlist.",
        )
            .into_response();
    }

    // CE seat cap: a fresh sign-in (login not already in `users`) is
    // rejected once `users` has reached `CE_USER_LIMIT` rows. Existing
    // users always get through — the cap is on *adding seats*, not on
    // sign-in volume. EE will lift this with a license-keyed cap.
    let now = crate::now_unix();
    let bootstrap_admin = env::var("BOOTSTRAP_ADMIN").ok();
    let user_count = crate::db::count_users(&state.db).await.unwrap_or(0);
    let already_seated = matches!(
        crate::db::get_user(&state.db, &user_data.login).await,
        Ok(Some(_))
    );
    if !already_seated && user_count >= crate::CE_USER_LIMIT as i64 {
        tracing::warn!(
            login = %user_data.login,
            user_count,
            limit = crate::CE_USER_LIMIT,
            "rejecting new sign-in: CE seat cap reached"
        );
        crate::db::record_audit(
            &state.db,
            now,
            Some(&user_data.login),
            None,
            "auth.login.seat_cap_reached",
            false,
            Some(&format!("limit={}", crate::CE_USER_LIMIT)),
        )
        .await;
        return (
            StatusCode::FORBIDDEN,
            format!(
                "This sys-manager Community Edition is at its {}-user seat cap. \
                 Ask an existing admin to remove a seat at /admin, or upgrade to EE.",
                crate::CE_USER_LIMIT
            ),
        )
            .into_response();
    }

    // CE bootstrap rule: if no users exist yet, the first allowlisted
    // login becomes admin. Subsequent allowlisted logins default to
    // viewer. An operator can also pin a specific login as the
    // bootstrap admin via BOOTSTRAP_ADMIN — useful when the allowlist
    // changes order or the first sign-in is mistakenly someone else.
    let default_role = if user_count == 0 || bootstrap_admin.as_deref() == Some(user_data.login.as_str()) {
        "admin"
    } else {
        "viewer"
    };

    let user_row = match crate::db::upsert_login(
        &state.db,
        &user_data.login,
        default_role,
        now,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to upsert user row");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user lookup failed").into_response();
        }
    };

    let role = Role::parse(&user_row.role);
    let mfa_required = user_row.totp_enabled != 0;

    // Pending-MFA cookies expire after 10 minutes; full sessions get the
    // historical 24h.
    let (mfa_verified, ttl, redirect_path) = if mfa_required {
        (false, 600_i64, "mfa")
    } else {
        (true, 24 * 3600_i64, "")
    };

    let token = match issue_jwt(&user_data.login, role, mfa_verified, ttl) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to encode jwt");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to issue session token")
                .into_response();
        }
    };

    let cookie = build_session_cookie(token);
    let ui_url = env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string());
    let dest = format!("{ui_url}{redirect_path}");

    crate::db::record_audit(
        &state.db,
        now,
        Some(&user_data.login),
        None,
        if mfa_required { "auth.login.pending_mfa" } else { "auth.login" },
        true,
        Some(&format!("role={}", role.as_str())),
    )
    .await;

    (jar.add(cookie), Redirect::temporary(&dest)).into_response()
}

fn decode_claims(token: &str) -> Option<Claims> {
    let secret = jwt_secret();
    let mut validation = Validation::default();
    validation.validate_exp = true;
    validation.validate_nbf = false;

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

/// Returns true if the token decodes, the subject is on the allowlist,
/// AND the MFA challenge has been completed. Pending-MFA tokens are
/// treated as unauthenticated for everything except the verify endpoint.
pub fn verify_token(token: &str) -> bool {
    decode_claims(token)
        .map(|c| c.mfa && is_user_allowed(&c.sub))
        .unwrap_or(false)
}

/// Returns the GitHub login from a fully-verified session cookie, or
/// `None` if the token is missing, expired, not on the allowlist, or
/// still pending MFA.
pub fn user_from_token(token: &str) -> Option<String> {
    decode_claims(token).and_then(|c| {
        if c.mfa && is_user_allowed(&c.sub) {
            Some(c.sub)
        } else {
            None
        }
    })
}

/// Returns the full Claims (role + mfa flag) regardless of MFA state.
/// Callers that need to distinguish pending-vs-verified must inspect
/// `claims.mfa`.
pub fn claims_from_token(token: &str) -> Option<Claims> {
    decode_claims(token).filter(|c| is_user_allowed(&c.sub))
}

/// CE: dev-mode shortcut returning a synthetic admin claim. Used by all
/// the gating helpers below so a single env-var flip continues to make
/// local development frictionless.
fn dev_claims() -> Claims {
    Claims {
        sub: "dev".to_string(),
        exp: (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize)
            + 24 * 3600,
        role: "admin".to_string(),
        mfa: true,
    }
}

/// Resolve the current session from a CookieJar. Returns the verified
/// claims, or a `(StatusCode, &'static str)` suitable for direct
/// IntoResponse use in handlers. The role string in the returned claims
/// is *re-resolved* against the `users` table so a freshly-demoted user
/// is treated as viewer immediately, even if their JWT still says admin.
pub async fn current_user(
    jar: &CookieJar,
    db: &sqlx::SqlitePool,
) -> Result<Claims, (StatusCode, &'static str)> {
    if dev_mode() {
        return Ok(dev_claims());
    }
    let cookie = jar
        .get("auth_token")
        .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))?;
    let mut claims = claims_from_token(cookie.value())
        .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))?;
    if !claims.mfa {
        return Err((StatusCode::FORBIDDEN, "MFA required"));
    }
    if let Ok(Some(row)) = crate::db::get_user(db, &claims.sub).await {
        claims.role = row.role;
    }
    Ok(claims)
}

/// CE: gate for write endpoints. Requires `current_user` to succeed and
/// the role to be `admin`.
pub async fn require_admin(
    jar: &CookieJar,
    db: &sqlx::SqlitePool,
) -> Result<Claims, (StatusCode, &'static str)> {
    let claims = current_user(jar, db).await?;
    if Role::parse(&claims.role) == Role::Admin {
        Ok(claims)
    } else {
        Err((StatusCode::FORBIDDEN, "viewer role: read-only"))
    }
}

/// CE: pending-MFA escape hatch — used by `/api/auth/mfa/verify` to
/// look up the pre-MFA token without bouncing on the `mfa=false`
/// state. Does NOT short-circuit dev mode (dev mode never has
/// pending-MFA, so the caller checks `dev_mode()` itself).
pub fn pending_mfa_claims(jar: &CookieJar) -> Option<Claims> {
    let cookie = jar.get("auth_token")?;
    let claims = claims_from_token(cookie.value())?;
    if claims.mfa {
        None
    } else {
        Some(claims)
    }
}

pub fn is_dev_mode() -> bool {
    dev_mode()
}
