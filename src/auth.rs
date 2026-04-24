use axum::{
    extract::Query,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Debug, Deserialize)]
struct GithubTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
}

pub fn auth_routes() -> Router {
    Router::new()
        .route("/login", get(login_handler))
        .route("/callback", get(callback_handler))
}

async fn login_handler() -> impl IntoResponse {
    let client_id = env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| "dummy_id".to_string());
    let redirect_uri = env::var("OAUTH_REDIRECT_URL").unwrap_or_else(|_| "https://dashboard.example.com/auth/callback".to_string());
    
    let redirect_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&state=sysmanager",
        client_id, urlencoding::encode(&redirect_uri)
    );

    Redirect::temporary(&redirect_url)
}

async fn callback_handler(jar: CookieJar, Query(query): Query<AuthRequest>) -> Response {
    println!("Received GitHub OAuth callback");

    let client_id = env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| "dummy_id".to_string());
    let client_secret = env::var("GITHUB_CLIENT_SECRET").unwrap_or_else(|_| "dummy_secret".to_string());
    
    let client = reqwest::Client::new();
    
    let token_res = match client.post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", query.code),
        ])
        .send()
        .await {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Failed to exchange token: {}", e);
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to get access token").into_response();
            }
        };

    let token_data = match token_res.json::<GithubTokenResponse>().await {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to parse token response: {}", e);
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to parse access token").into_response();
        }
    };

    let user_res = match client.get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token_data.access_token))
        .header("User-Agent", "sys-manager")
        .send()
        .await {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Failed to fetch user profile: {}", e);
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to get user profile").into_response();
            }
        };

    let user_data = match user_res.json::<GithubUser>().await {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to parse user profile: {}", e);
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to parse user profile").into_response();
        }
    };

    if user_data.login != "sppidy" {
        println!("Unauthorized user attempted login: {}", user_data.login);
        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized user. Only 'sppidy' is allowed.").into_response();
    }

    let expiration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize + 24 * 3600;

    let claims = Claims {
        sub: user_data.login,
        exp: expiration,
    };

    let secret = env::var("JWT_SECRET").unwrap_or_else(|_| "supersecretkey".to_string());
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap();

    let cookie = Cookie::build(("auth_token", token))
        .path("/")
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        // .secure(true) // Set to true if served exclusively over HTTPS
        .build();

    let ui_url = env::var("UI_URL").unwrap_or_else(|_| "https://dashboard.example.com/".to_string());
    
    (jar.add(cookie), Redirect::temporary(&ui_url)).into_response()
}

pub fn verify_token(token: &str) -> bool {
    let secret = env::var("JWT_SECRET").unwrap_or_else(|_| "supersecretkey".to_string());
    let mut validation = jsonwebtoken::Validation::default();
    validation.validate_exp = true;
    validation.validate_nbf = false;
    
    match jsonwebtoken::decode::<Claims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        Ok(token_data) => token_data.claims.sub == "sppidy",
        Err(_) => false,
    }
}
