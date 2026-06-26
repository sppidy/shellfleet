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
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;

use crate::AppState;

const OAUTH_STATE_COOKIE: &str = "oauth_state";
/// Pending-MFA tokens are short-lived. Anything past 10 min and the
/// user has to re-OAuth — minimal annoyance, large blast-radius
/// reduction if a pending-MFA cookie ever leaks.
const MFA_PENDING_TTL_SECS: i64 = 600;
const FULL_SESSION_TTL_SECS: i64 = 24 * 3600;
const OAUTH_STATE_TTL_SECS: i64 = 600;

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
    /// Issued-at timestamp. Compared against `users.session_epoch` on
    /// every check; tokens issued before the user's epoch (logout,
    /// role change, MFA disable) are rejected.
    #[serde(default)]
    pub iat: i64,
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
    // GitHub returns HTTP 200 with an {error, error_description} body on
    // failures (bad/reused code, redirect_uri mismatch), so access_token is
    // optional and we surface the error rather than a generic parse-500.
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
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

/// Whether auth/CSRF cookies carry the `Secure` attribute. Default on; set
/// COOKIE_SECURE to 0/false/no/off (case-insensitive) to disable for plain-HTTP
/// local dev. Shared with the CSRF cookie in csrf.rs so the session and CSRF
/// cookies never disagree on the flag.
pub(crate) fn cookie_secure() -> bool {
    !matches!(
        env::var("COOKIE_SECURE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn jwt_secret() -> String {
    // Production safety: any non-`dev` value is fine; an unset value
    // is fatal. `assert_jwt_secret_present` is called from `main`
    // before the server starts listening, so this fallback string is
    // only a defence-in-depth — control should never reach it.
    env::var("JWT_SECRET").unwrap_or_else(|_| {
        panic!(
            "JWT_SECRET environment variable is not set. Refusing to sign tokens \
             with a hard-coded default. Set JWT_SECRET=dev for local development \
             or to a 64-char random hex string for production."
        )
    })
}

/// Called from `main` at startup. Aborts the process if JWT_SECRET is
/// unset or trivially short (< 32 chars), unless we're in `dev` mode.
pub fn assert_jwt_secret_present() {
    let val = env::var("JWT_SECRET").unwrap_or_default();
    if val.is_empty() {
        eprintln!(
            "FATAL: JWT_SECRET is not set. Refusing to start.\n\
             Set JWT_SECRET=dev for local development or a 64+ char random \
             hex string for production."
        );
        std::process::exit(2);
    }
    if val == "dev" {
        if !dev_flag_set() {
            eprintln!(
                "FATAL: JWT_SECRET=dev disables auth/RBAC/MFA/CSRF and is for local \
                 development only. Refusing to start with a stray 'dev' secret. Set \
                 SHELLFLEET_DEV=1 to intentionally enable dev mode, or use a real \
                 32+ char secret (`openssl rand -hex 32`)."
            );
            std::process::exit(2);
        }
        tracing::warn!(
            "JWT_SECRET=dev + SHELLFLEET_DEV — auth, RBAC, MFA, and CSRF are all \
             disabled. Local development only; never deploy this configuration."
        );
        return;
    }
    if val.len() < 32 {
        eprintln!(
            "FATAL: JWT_SECRET is {} chars; refusing to start with anything \
             shorter than 32. Generate a fresh secret: \
             `openssl rand -hex 32`.",
            val.len()
        );
        std::process::exit(2);
    }
    if val == "supersecretkey" {
        eprintln!(
            "FATAL: JWT_SECRET is the historical placeholder \
             'supersecretkey'. Refusing to start."
        );
        std::process::exit(2);
    }
}

fn dev_flag_set() -> bool {
    matches!(
        env::var("SHELLFLEET_DEV")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn dev_mode() -> bool {
    // Dev mode (disables auth/RBAC/MFA/CSRF) requires BOTH JWT_SECRET=dev AND
    // an explicit SHELLFLEET_DEV opt-in, so a stray production JWT_SECRET=dev
    // can't silently turn off every protection.
    env::var("JWT_SECRET").unwrap_or_default() == "dev" && dev_flag_set()
}

fn random_oauth_state() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const INVITE_COOKIE: &str = "pending_invite";

#[derive(Deserialize)]
struct LoginQuery {
    #[serde(default)]
    invite: Option<String>,
}

async fn login_handler(jar: CookieJar, Query(q): Query<LoginQuery>) -> impl IntoResponse {
    let mut jar = jar;
    if let Some(ref code) = q.invite {
        let cookie = Cookie::build((INVITE_COOKIE, code.clone()))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(cookie_secure())
            .max_age(time::Duration::seconds(600))
            .build();
        jar = jar.add(cookie);
    }

    if crate::ee::ee_active()
        && env::var("EE_OIDC_ISSUER")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
    {
        let ee_public = env::var("EE_PUBLIC_URL")
            .or_else(|_| env::var("UI_URL"))
            .unwrap_or_else(|_| "https://dashboard.example.com".to_string());
        let ui_url =
            env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string());
        let sso_url = format!(
            "{}/auth/sso/login?redirect_uri={}",
            ee_public.trim_end_matches('/'),
            urlencoding::encode(&ui_url),
        );
        return Redirect::temporary(&sso_url).into_response();
    }

    let client_id = env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| "dummy_id".to_string());
    let redirect_uri = env::var("OAUTH_REDIRECT_URL")
        .unwrap_or_else(|_| "https://dashboard.example.com/auth/callback".to_string());

    // Random per-flow state cookie. The callback rejects any state
    // value that doesn't match the cookie set here, defeating the
    // classic OAuth CSRF where an attacker tricks a victim into hitting
    // /auth/callback with the attacker's authorization code. Cookie is
    // HttpOnly + SameSite=Lax + Secure so it travels back from GitHub
    // unmodified but isn't readable from JS.
    let state = random_oauth_state();
    let cookie = Cookie::build((OAUTH_STATE_COOKIE, state.clone()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .max_age(time::Duration::seconds(OAUTH_STATE_TTL_SECS))
        .build();

    let redirect_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&state={}&scope=read:user",
        client_id,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&state),
    );

    (jar.add(cookie), Redirect::temporary(&redirect_url)).into_response()
}

async fn logout_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Best-effort: bump the session epoch so the JWT we just dropped
    // can't be replayed. Reads the cookie before clearing it.
    if let Some(cookie) = jar.get("auth_token") {
        if let Some(claims) = claims_from_token_no_epoch_check(cookie.value()) {
            let _ = crate::db::bump_session_epoch(&state.db, &claims.sub, crate::now_unix()).await;
        }
    }
    let mut cookie = Cookie::build(("auth_token", ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .build();
    cookie.make_removal();

    let ui_url =
        env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string());
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
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let exp = (now_secs + ttl_secs) as usize;

    let claims = Claims {
        sub: sub.to_string(),
        exp,
        iat: now_secs,
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

pub fn issue_internal_jwt(login: &str, role: &str, mfa: bool) -> String {
    let r = Role::parse(role);
    issue_jwt(login, r, mfa, 86400).unwrap_or_default()
}

async fn callback_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthRequest>,
) -> Response {
    tracing::info!("github oauth callback");

    // CSRF protection on the callback. The login_handler set
    // `oauth_state` to a random 24-byte value; GitHub must echo it
    // back via the `state` query param. Any mismatch (or missing
    // cookie) is treated as a forgery attempt.
    let cookie_state = jar.get(OAUTH_STATE_COOKIE).map(|c| c.value().to_string());
    match &cookie_state {
        Some(s) if s == &query.state => { /* ok */ }
        _ => {
            tracing::warn!(
                got = %query.state,
                expected = ?cookie_state,
                "oauth state mismatch — rejecting callback"
            );
            return (
                StatusCode::FORBIDDEN,
                "OAuth state mismatch — start the sign-in flow over.",
            )
                .into_response();
        }
    }

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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get access token",
            )
                .into_response();
        }
    };

    let token_data = match token_res.json::<GithubTokenResponse>().await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse oauth token response");
            return (StatusCode::BAD_GATEWAY, "Failed to parse access token").into_response();
        }
    };

    let access_token = match token_data.access_token {
        Some(t) => t,
        None => {
            // GitHub signalled an error in a 200 body — surface the real
            // cause (commonly a misconfigured GITHUB_CLIENT_SECRET or
            // OAUTH_REDIRECT_URL) instead of a generic 500.
            tracing::error!(
                error = token_data.error.as_deref().unwrap_or("unknown"),
                description = token_data.error_description.as_deref().unwrap_or(""),
                "GitHub OAuth token endpoint returned an error"
            );
            return (StatusCode::BAD_GATEWAY, "GitHub rejected the OAuth code").into_response();
        }
    };

    let user_res = match client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "shellfleet")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch github user");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get user profile",
            )
                .into_response();
        }
    };

    if !user_res.status().is_success() {
        let status = user_res.status();
        tracing::error!(%status, "github /user returned non-success");
        return (StatusCode::BAD_GATEWAY, "GitHub user lookup failed").into_response();
    }

    let user_data = match user_res.json::<GithubUser>().await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse github user");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to parse user profile",
            )
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

    // CE bootstrap rule: if no users exist yet, the first allowlisted
    // login becomes admin. Subsequent allowlisted logins default to
    // viewer. An operator can also pin a specific login as the
    // bootstrap admin via BOOTSTRAP_ADMIN — useful when the allowlist
    // changes order or the first sign-in is mistakenly someone else.
    //
    // Seat-cap enforcement is now atomic: `upsert_login_with_seat_check`
    // wraps the COUNT + INSERT in a single SQLite transaction so two
    // concurrent first-time sign-ins can't both squeeze past the cap.
    let now = crate::now_unix();
    let bootstrap_admin = env::var("BOOTSTRAP_ADMIN").ok();
    let user_count = crate::db::count_users(&state.db).await.unwrap_or(0);

    // Check for a pending invite — overrides the default role assignment
    let invite_code = jar.get(INVITE_COOKIE).map(|c| c.value().to_string());
    let invite_role = if let Some(ref code) = invite_code {
        match crate::db::get_invite(&state.db, code).await {
            Ok(Some(inv)) if inv.used_by.is_none() && now <= inv.expires_at => {
                Some(inv.role.clone())
            }
            _ => None,
        }
    } else {
        None
    };

    let default_role =
        if user_count == 0 || bootstrap_admin.as_deref() == Some(user_data.login.as_str()) {
            "admin"
        } else if let Some(ref r) = invite_role {
            r.as_str()
        } else {
            "viewer"
        };

    let seat_cap = crate::db::seat_limit(&state.db).await;
    let upsert = match crate::db::upsert_login_with_seat_check(
        &state.db,
        &user_data.login,
        default_role,
        now,
        seat_cap,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to upsert user row");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user lookup failed").into_response();
        }
    };

    let user_row = match upsert {
        crate::db::SeatedUpsert::Existing(r) | crate::db::SeatedUpsert::Created(r) => r,
        crate::db::SeatedUpsert::SeatCapReached => {
            tracing::warn!(
                login = %user_data.login,
                limit = seat_cap,
                "rejecting new sign-in: seat cap reached"
            );
            crate::db::record_audit(
                &state.db,
                now,
                Some(&user_data.login),
                None,
                "auth.login.seat_cap_reached",
                false,
                Some(&format!("limit={seat_cap}")),
            )
            .await;
            return (
                StatusCode::FORBIDDEN,
                format!(
                    "This ShellFleet instance is at its {seat_cap}-user seat cap. \
                     Ask an existing admin to remove a seat at /admin, or upgrade to EE.",
                ),
            )
                .into_response();
        }
    };

    let role = Role::parse(&user_row.role);
    let mfa_required = user_row.totp_enabled != 0;

    // Pending-MFA cookies expire after MFA_PENDING_TTL_SECS; full
    // sessions get FULL_SESSION_TTL_SECS.
    let (mfa_verified, ttl, redirect_path) = if mfa_required {
        (false, MFA_PENDING_TTL_SECS, "mfa")
    } else {
        (true, FULL_SESSION_TTL_SECS, "")
    };

    let token = match issue_jwt(&user_data.login, role, mfa_verified, ttl) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to encode jwt");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to issue session token",
            )
                .into_response();
        }
    };

    let cookie = build_session_cookie(token);
    let ui_url =
        env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string());
    let dest = format!("{ui_url}{redirect_path}");

    // Burn the OAuth state cookie now that we've consumed it.
    let mut clear_state = Cookie::build((OAUTH_STATE_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .build();
    clear_state.make_removal();

    // Redeem invite if present
    let mut clear_invite = None;
    if let Some(ref code) = invite_code {
        if crate::db::redeem_invite(&state.db, code, &user_data.login)
            .await
            .unwrap_or(false)
        {
            tracing::info!(login = %user_data.login, %code, "invite redeemed");
            crate::db::record_audit(
                &state.db,
                now,
                Some(&user_data.login),
                None,
                "invite.redeemed",
                true,
                Some(&format!("code={code}")),
            )
            .await;
        }
        let mut c = Cookie::build((INVITE_COOKIE, ""))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(cookie_secure())
            .build();
        c.make_removal();
        clear_invite = Some(c);
    }

    crate::db::record_audit(
        &state.db,
        now,
        Some(&user_data.login),
        None,
        if mfa_required {
            "auth.login.pending_mfa"
        } else {
            "auth.login"
        },
        true,
        Some(&format!("role={}", role.as_str())),
    )
    .await;

    let mut final_jar = jar.add(cookie).add(clear_state);
    if let Some(c) = clear_invite {
        final_jar = final_jar.add(c);
    }
    (final_jar, Redirect::temporary(&dest)).into_response()
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

/// Decode without checking the user's session_epoch. Used by the
/// logout handler so we can identify *who* to bump even though we're
/// about to invalidate the token. Other call sites must use
/// `claims_from_token` (which performs the epoch check) instead.
pub fn claims_from_token_no_epoch_check(token: &str) -> Option<Claims> {
    decode_claims(token).filter(|c| is_user_allowed(&c.sub))
}

/// Returns true if the token decodes, the subject is on the allowlist,
/// AND the MFA challenge has been completed. Pending-MFA tokens are
/// treated as unauthenticated for everything except the verify endpoint.
///
/// NB: this *does not* check the session_epoch — for a fast pre-DB
/// path used by code that doesn't carry the pool. The DB-backed
/// `current_user` / RBAC middleware path is the authoritative one and
/// re-validates against `users.session_epoch`.
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
/// `claims.mfa`. **Does not** check session_epoch — most call sites
/// follow up with a DB lookup that does.
pub fn claims_from_token(token: &str) -> Option<Claims> {
    decode_claims(token).filter(|c| is_user_allowed(&c.sub))
}

/// CE: dev-mode shortcut returning a synthetic admin claim. Used by all
/// the gating helpers below so a single env-var flip continues to make
/// local development frictionless.
fn dev_claims() -> Claims {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    Claims {
        sub: "dev".to_string(),
        exp: (now as usize) + 24 * 3600,
        iat: now,
        role: "admin".to_string(),
        mfa: true,
    }
}

/// Resolve the current session from a CookieJar. Returns the verified
/// claims, or a `(StatusCode, &'static str)` suitable for direct
/// IntoResponse use in handlers.
///
/// In addition to the JWT signature + expiry checks, this:
///   - Re-resolves `role` from the `users` table so a freshly-demoted
///     admin is treated as viewer immediately on the next request.
///   - Rejects tokens whose `iat` predates the user's `session_epoch`,
///     which lets logout / role-change / MFA-disable invalidate
///     in-flight cookies without waiting for the 24h JWT expiry.
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
    let mut claims =
        claims_from_token(cookie.value()).ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))?;
    if !claims.mfa {
        return Err((StatusCode::FORBIDDEN, "MFA required"));
    }
    if let Ok(Some(row)) = crate::db::get_user(db, &claims.sub).await {
        if claims.iat < row.session_epoch {
            return Err((
                StatusCode::UNAUTHORIZED,
                "session revoked — please sign in again",
            ));
        }
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
    if claims.mfa { None } else { Some(claims) }
}

pub fn is_dev_mode() -> bool {
    dev_mode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_parse_maps_admin_and_defaults_to_viewer() {
        assert_eq!(Role::parse("admin"), Role::Admin);
        assert_eq!(Role::parse("viewer"), Role::Viewer);
        // Exact match only — anything else is the safe default.
        assert_eq!(Role::parse("ADMIN"), Role::Viewer);
        assert_eq!(Role::parse("root"), Role::Viewer);
        assert_eq!(Role::parse(""), Role::Viewer);
    }

    #[test]
    fn role_as_str_roundtrips_through_parse() {
        assert_eq!(Role::parse(Role::Admin.as_str()), Role::Admin);
        assert_eq!(Role::parse(Role::Viewer.as_str()), Role::Viewer);
    }
}
