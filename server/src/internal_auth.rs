use axum::{
    body::{Body, to_bytes},
    extract::{OriginalUri, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use shared::internal_auth::{
    Direction, HEADER_KEY_ID, HEADER_NONCE, HEADER_RESPONSE_SIGNATURE, HEADER_SIGNATURE,
    HEADER_TIMESTAMP, InternalKey, SignedRequest, SignedResponse,
};
use sqlx::SqlitePool;
use std::{fmt, sync::Arc, time::Duration};

use crate::AppState;

const MAX_INTERNAL_BODY: usize = 8 * 1024 * 1024;
const NONCE_TTL_SECS: i64 = 300;

#[derive(Clone)]
struct Config {
    ce_to_ee: Vec<InternalKey>,
    ce_to_ee_active: String,
    ee_to_ce: Vec<InternalKey>,
}

impl Config {
    fn from_env() -> Result<Self, Error> {
        Self::from_values(
            &required_env("CE_TO_EE_HMAC_KEYS_JSON")?,
            &required_env("CE_TO_EE_HMAC_ACTIVE_KEY_ID")?,
            &required_env("EE_TO_CE_HMAC_KEYS_JSON")?,
            &required_env("EE_TO_CE_HMAC_ACTIVE_KEY_ID")?,
        )
    }

    fn from_values(
        ce_to_ee_json: &str,
        ce_to_ee_active: &str,
        ee_to_ce_json: &str,
        ee_to_ce_active: &str,
    ) -> Result<Self, Error> {
        let ce_to_ee = shared::internal_auth::parse_keyring(ce_to_ee_json)?;
        let ee_to_ce = shared::internal_auth::parse_keyring(ee_to_ce_json)?;
        shared::internal_auth::active_key(&ce_to_ee, ce_to_ee_active)?;
        shared::internal_auth::active_key(&ee_to_ce, ee_to_ce_active)?;
        Ok(Self {
            ce_to_ee,
            ce_to_ee_active: ce_to_ee_active.to_owned(),
            ee_to_ce,
        })
    }

    fn ce_active(&self) -> Result<&InternalKey, Error> {
        Ok(shared::internal_auth::active_key(
            &self.ce_to_ee,
            &self.ce_to_ee_active,
        )?)
    }
}

pub(crate) fn validate_config() -> Result<(), Error> {
    Config::from_env().map(|_| ())
}

fn required_env(name: &'static str) -> Result<String, Error> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(Error::MissingConfig(name))
}

#[derive(Debug)]
pub(crate) enum Error {
    MissingConfig(&'static str),
    Auth(shared::internal_auth::AuthError),
    Database(sqlx::Error),
    Replay,
    BadHeader,
    BodyTooLarge,
    InvalidUrl,
    Transport(reqwest::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConfig(name) => write!(formatter, "missing {name}"),
            Self::Auth(error) => error.fmt(formatter),
            Self::Database(error) => error.fmt(formatter),
            Self::Replay => formatter.write_str("internal request replayed"),
            Self::BadHeader => formatter.write_str("invalid internal authentication header"),
            Self::BodyTooLarge => formatter.write_str("internal response body too large"),
            Self::InvalidUrl => formatter.write_str("invalid internal URL"),
            Self::Transport(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {}

impl From<shared::internal_auth::AuthError> for Error {
    fn from(value: shared::internal_auth::AuthError) -> Self {
        Self::Auth(value)
    }
}

impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Transport(value)
    }
}

pub(crate) async fn init_nonce_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS internal_request_nonces (
            direction TEXT NOT NULL,
            key_id TEXT NOT NULL,
            nonce TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            PRIMARY KEY (direction, key_id, nonce)
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_internal_request_nonces_expiry ON internal_request_nonces(expires_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::CeToEe => "ce-to-ee",
        Direction::EeToCe => "ee-to-ce",
    }
}

async fn consume_nonce(
    pool: &SqlitePool,
    direction: Direction,
    key_id: &str,
    nonce: &str,
    now: i64,
) -> Result<(), Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM internal_request_nonces WHERE expires_at < ?1")
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    let inserted = sqlx::query(
        r#"
        INSERT OR IGNORE INTO internal_request_nonces
            (direction, key_id, nonce, expires_at)
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind(direction_label(direction))
    .bind(key_id)
    .bind(nonce)
    .bind(now + NONCE_TTL_SECS)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    if inserted.rows_affected() == 1 {
        Ok(())
    } else {
        Err(Error::Replay)
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &'static str) -> Result<&'a str, Error> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or(Error::BadHeader)
}

fn optional_header<'a>(headers: &'a HeaderMap, name: &'static str) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

fn signed_error(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

async fn sign_axum_response(
    config: &Config,
    nonce: &str,
    response: Response,
) -> Result<Response, Error> {
    let (mut parts, body) = response.into_parts();
    let body = to_bytes(body, MAX_INTERNAL_BODY)
        .await
        .map_err(|_| Error::BodyTooLarge)?;
    let signed = shared::internal_auth::sign_response(
        config.ce_active()?,
        Direction::CeToEe,
        nonce,
        parts.status.as_u16(),
        &body,
    )?;
    parts.headers.insert(
        HeaderName::from_static(HEADER_KEY_ID),
        HeaderValue::from_str(&signed.key_id).map_err(|_| Error::BadHeader)?,
    );
    parts.headers.insert(
        HeaderName::from_static(HEADER_RESPONSE_SIGNATURE),
        HeaderValue::from_str(&signed.signature).map_err(|_| Error::BadHeader)?,
    );
    Ok(Response::from_parts(parts, Body::from(body)))
}

pub(crate) async fn require_ee_signature(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(error = %error, "internal authentication is not configured");
            return signed_error(StatusCode::SERVICE_UNAVAILABLE, "internal auth unavailable");
        }
    };
    let (mut parts, body) = request.into_parts();
    let key_id = match header(&parts.headers, HEADER_KEY_ID) {
        Ok(value) => value.to_owned(),
        Err(_) => return signed_error(StatusCode::UNAUTHORIZED, "unauthorized"),
    };
    let timestamp = match header(&parts.headers, HEADER_TIMESTAMP)
        .and_then(|value| value.parse::<i64>().map_err(|_| Error::BadHeader))
    {
        Ok(value) => value,
        Err(_) => return signed_error(StatusCode::UNAUTHORIZED, "unauthorized"),
    };
    let nonce = match header(&parts.headers, HEADER_NONCE) {
        Ok(value) => value.to_owned(),
        Err(_) => return signed_error(StatusCode::UNAUTHORIZED, "unauthorized"),
    };
    let signature = match header(&parts.headers, HEADER_SIGNATURE) {
        Ok(value) => value.to_owned(),
        Err(_) => return signed_error(StatusCode::UNAUTHORIZED, "unauthorized"),
    };
    let content_type = optional_header(&parts.headers, "content-type").to_owned();
    let login = optional_header(&parts.headers, "x-shellfleet-login").to_owned();
    let role = optional_header(&parts.headers, "x-shellfleet-role").to_owned();
    let canonical_uri = parts
        .extensions
        .get::<OriginalUri>()
        .map(|value| &value.0)
        .unwrap_or(&parts.uri);
    let path = canonical_uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or_else(|| canonical_uri.path())
        .to_owned();
    let method = parts.method.as_str().to_owned();
    let body = match to_bytes(body, MAX_INTERNAL_BODY).await {
        Ok(body) => body,
        Err(_) => return signed_error(StatusCode::PAYLOAD_TOO_LARGE, "body too large"),
    };
    let signed = SignedRequest {
        key_id,
        timestamp,
        nonce: nonce.clone(),
        signature,
        direction: Direction::EeToCe,
    };
    if let Err(error) = shared::internal_auth::verify_request(
        &config.ee_to_ce,
        Direction::EeToCe,
        &signed,
        crate::now_unix(),
        &method,
        &path,
        &body,
        &content_type,
        &login,
        &role,
    ) {
        tracing::warn!(error = %error, "rejected unauthenticated EE request");
        return signed_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if let Err(error) = consume_nonce(
        &state.db,
        Direction::EeToCe,
        &signed.key_id,
        &signed.nonce,
        crate::now_unix(),
    )
    .await
    {
        tracing::warn!(error = %error, "rejected replayed EE request");
        let response = signed_error(StatusCode::CONFLICT, "replayed request");
        return sign_axum_response(&config, &nonce, response)
            .await
            .unwrap_or_else(|_| signed_error(StatusCode::INTERNAL_SERVER_ERROR, "auth failure"));
    }

    for name in [
        HEADER_KEY_ID,
        HEADER_TIMESTAMP,
        HEADER_NONCE,
        HEADER_SIGNATURE,
    ] {
        parts.headers.remove(name);
    }
    let response = next.run(Request::from_parts(parts, Body::from(body))).await;
    sign_axum_response(&config, &nonce, response)
        .await
        .unwrap_or_else(|error| {
            tracing::error!(error = %error, "failed to sign internal response");
            signed_error(StatusCode::INTERNAL_SERVER_ERROR, "auth failure")
        })
}

pub(crate) struct VerifiedResponse {
    pub status: reqwest::StatusCode,
    pub body: Vec<u8>,
}

impl VerifiedResponse {
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    body: Vec<u8>,
    content_type: &str,
    login: &str,
    role: &str,
    timeout: Duration,
) -> Result<VerifiedResponse, Error> {
    let config = Config::from_env()?;
    let parsed = reqwest::Url::parse(url).map_err(|_| Error::InvalidUrl)?;
    let mut path = parsed.path().to_owned();
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    let now = crate::now_unix();
    let signed = shared::internal_auth::sign_request(
        config.ce_active()?,
        Direction::CeToEe,
        now,
        &shared::internal_auth::new_nonce(),
        method.as_str(),
        &path,
        &body,
        content_type,
        login,
        role,
    )?;
    let mut builder = client
        .request(method, url)
        .header(HEADER_KEY_ID, &signed.key_id)
        .header(HEADER_TIMESTAMP, signed.timestamp.to_string())
        .header(HEADER_NONCE, &signed.nonce)
        .header(HEADER_SIGNATURE, &signed.signature)
        .timeout(timeout);
    if !content_type.is_empty() {
        builder = builder.header("content-type", content_type);
    }
    if !login.is_empty() {
        builder = builder.header("x-shellfleet-login", login);
    }
    if !role.is_empty() {
        builder = builder.header("x-shellfleet-role", role);
    }
    if !body.is_empty() {
        builder = builder.body(body);
    }
    let response = builder.send().await?;
    let status = response.status();
    let response_key = header(response.headers(), HEADER_KEY_ID)?.to_owned();
    let response_signature = header(response.headers(), HEADER_RESPONSE_SIGNATURE)?.to_owned();
    let mut response_body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if response_body.len().saturating_add(chunk.len()) > MAX_INTERNAL_BODY {
            return Err(Error::BodyTooLarge);
        }
        response_body.extend_from_slice(&chunk);
    }
    let signed_response = SignedResponse {
        key_id: response_key,
        request_nonce: signed.nonce,
        signature: response_signature,
        direction: Direction::EeToCe,
    };
    shared::internal_auth::verify_response(
        &config.ee_to_ce,
        Direction::EeToCe,
        &signed_response,
        status.as_u16(),
        &response_body,
    )?;
    Ok(VerifiedResponse {
        status,
        body: response_body,
    })
}

pub(crate) async fn send_json<T: serde::Serialize + ?Sized>(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    value: &T,
    login: &str,
    role: &str,
    timeout: Duration,
) -> Result<VerifiedResponse, Error> {
    let body = serde_json::to_vec(value).map_err(|_| Error::BadHeader)?;
    send(
        client,
        method,
        url,
        body,
        "application/json",
        login,
        role,
        timeout,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn keyring(byte: u8) -> String {
        let secret = data_encoding::BASE64.encode(&[byte; 32]);
        format!(r#"{{"active":"{secret}"}}"#)
    }

    #[test]
    fn config_requires_distinct_directional_keyrings_and_valid_active_ids() {
        let ce = keyring(1);
        let ee = keyring(2);
        let config = Config::from_values(&ce, "active", &ee, "active").unwrap();
        let nonce = [3; 32];
        let ce_signature = shared::internal_auth::sign_request(
            &config.ce_to_ee[0],
            Direction::CeToEe,
            1,
            &nonce,
            "GET",
            "/",
            b"",
            "",
            "",
            "",
        )
        .unwrap();
        let ee_signature = shared::internal_auth::sign_request(
            &config.ee_to_ce[0],
            Direction::CeToEe,
            1,
            &nonce,
            "GET",
            "/",
            b"",
            "",
            "",
            "",
        )
        .unwrap();
        assert_ne!(ce_signature.signature, ee_signature.signature);
        assert!(Config::from_values(&ce, "missing", &ee, "active").is_err());
    }

    #[tokio::test]
    async fn nonce_is_consumed_once_and_persists_in_sqlite() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_nonce_schema(&pool).await.unwrap();

        assert!(
            consume_nonce(&pool, Direction::EeToCe, "k1", "nonce", 100)
                .await
                .is_ok()
        );
        assert!(matches!(
            consume_nonce(&pool, Direction::EeToCe, "k1", "nonce", 101).await,
            Err(Error::Replay)
        ));
    }

    #[tokio::test]
    async fn expired_nonce_is_pruned_before_insert() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_nonce_schema(&pool).await.unwrap();
        consume_nonce(&pool, Direction::EeToCe, "k1", "nonce", 100)
            .await
            .unwrap();
        assert!(
            consume_nonce(
                &pool,
                Direction::EeToCe,
                "k1",
                "nonce",
                100 + NONCE_TTL_SECS + 1,
            )
            .await
            .is_ok()
        );
    }
}
