//! Native CLI device authorization.
//!
//! The CLI creates a short-lived request and shows its human-friendly code.
//! An already-authenticated dashboard user approves that code on `/device`.
//! The CLI then receives a purpose-restricted JWT that only works on `/ui/ws`.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AppState, auth, db};

const REQUEST_TTL_SECS: i64 = 5 * 60;
const CLI_TOKEN_TTL_SECS: i64 = 8 * 60 * 60;

#[derive(Serialize)]
pub struct CliAuthRequestResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub ws_url: String,
    pub expires_in: i64,
    pub interval: u64,
}

#[derive(Deserialize)]
pub struct CliAuthTokenRequest {
    pub device_code: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum CliAuthTokenResponse {
    Token {
        access_token: String,
        token_type: String,
        expires_in: i64,
    },
    Error {
        error: String,
    },
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/request", post(request_cli_auth))
        .route("/token", post(poll_cli_token))
}

fn random_code(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (&mut rng)
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn ui_url() -> String {
    std::env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string())
}

fn cli_ws_url(base: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base).map_err(|error| error.to_string())?;
    let scheme = match url.scheme() {
        "https" => "wss".to_string(),
        "http" => "ws".to_string(),
        "wss" | "ws" => url.scheme().to_string(),
        _ => return Err("UI_URL must use http or https".to_string()),
    };
    url.set_scheme(&scheme)
        .map_err(|_| "invalid UI_URL scheme".to_string())?;
    url.set_path("/ui/ws");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

async fn request_cli_auth(State(state): State<Arc<AppState>>) -> Response {
    let device_code = random_code(48);
    let raw_user_code = random_code(8);
    let user_code = format!("{}-{}", &raw_user_code[0..4], &raw_user_code[4..8]).to_uppercase();
    let now = crate::now_unix();
    let expires_at = now + REQUEST_TTL_SECS;

    if let Err(error) = db::insert_pending_device_for_purpose(
        &state.db,
        &device_code,
        &user_code,
        expires_at,
        "cli",
    )
    .await
    {
        tracing::error!(%error, "create CLI device authorization failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }

    let base = ui_url();
    let ws_url = match cli_ws_url(&base) {
        Ok(url) => url,
        Err(error) => {
            tracing::error!(%error, "invalid UI_URL for CLI authorization");
            let _ = db::delete_pending_device(&state.db, &device_code).await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "invalid UI_URL").into_response();
        }
    };
    let verification_uri = format!("{}/device?cli=1", base.trim_end_matches('/'));

    Json(CliAuthRequestResponse {
        device_code,
        user_code,
        verification_uri,
        ws_url,
        expires_in: REQUEST_TTL_SECS,
        interval: 5,
    })
    .into_response()
}

async fn poll_cli_token(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CliAuthTokenRequest>,
) -> Response {
    let now = crate::now_unix();
    let row = match db::pending_device(&state.db, &payload.device_code).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return Json(CliAuthTokenResponse::Error {
                error: "invalid_grant".to_string(),
            })
            .into_response();
        }
        Err(error) => {
            tracing::error!(%error, "CLI device authorization lookup failed");
            return Json(CliAuthTokenResponse::Error {
                error: "server_error".to_string(),
            })
            .into_response();
        }
    };

    if row.purpose != "cli" {
        return Json(CliAuthTokenResponse::Error {
            error: "invalid_grant".to_string(),
        })
        .into_response();
    }
    if now > row.expires_at {
        let _ = db::delete_pending_device(&state.db, &row.device_code).await;
        return Json(CliAuthTokenResponse::Error {
            error: "expired_token".to_string(),
        })
        .into_response();
    }
    if row.approved == 0 {
        return Json(CliAuthTokenResponse::Error {
            error: "authorization_pending".to_string(),
        })
        .into_response();
    }
    let Some(actor) = row.approved_by else {
        tracing::error!(device_code = %row.device_code, "CLI authorization missing approving user");
        return Json(CliAuthTokenResponse::Error {
            error: "server_error".to_string(),
        })
        .into_response();
    };
    let role = match db::get_user(&state.db, &actor).await {
        Ok(Some(user)) => auth::Role::parse(&user.role),
        Ok(None) => {
            return Json(CliAuthTokenResponse::Error {
                error: "invalid_grant".to_string(),
            })
            .into_response();
        }
        Err(error) => {
            tracing::error!(%error, %actor, "load CLI authorizer failed");
            return Json(CliAuthTokenResponse::Error {
                error: "server_error".to_string(),
            })
            .into_response();
        }
    };
    let consumed = match db::consume_pending_cli_device(&state.db, &row.device_code, &actor, now).await {
        Ok(consumed) => consumed,
        Err(error) => {
            tracing::error!(%error, "consume CLI device authorization failed");
            return Json(CliAuthTokenResponse::Error {
                error: "server_error".to_string(),
            })
            .into_response();
        }
    };
    if !consumed {
        return Json(CliAuthTokenResponse::Error {
            error: "invalid_grant".to_string(),
        })
        .into_response();
    }
    let token = match auth::issue_cli_jwt(&actor, role, CLI_TOKEN_TTL_SECS) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, %actor, "issue CLI session failed");
            return Json(CliAuthTokenResponse::Error {
                error: "server_error".to_string(),
            })
            .into_response();
        }
    };
    db::record_audit(
        &state.db,
        now,
        Some(&actor),
        None,
        "cli.device.token.issued",
        true,
        None,
    )
    .await;

    Json(CliAuthTokenResponse::Token {
        access_token: token,
        token_type: "cli_session".to_string(),
        expires_in: CLI_TOKEN_TTL_SECS,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::cli_ws_url;

    #[test]
    fn maps_dashboard_urls_to_operator_websocket() {
        assert_eq!(
            cli_ws_url("https://dashboard.example.com/").unwrap(),
            "wss://dashboard.example.com/ui/ws"
        );
        assert_eq!(
            cli_ws_url("http://localhost:3000/dashboard").unwrap(),
            "ws://localhost:3000/ui/ws"
        );
    }
}
