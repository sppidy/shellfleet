//! Device-login credentials for the native operator cockpit.
//!
//! Browser cookies are never copied to disk. The server issues a dedicated,
//! short-lived CLI token after the operator approves a code in the dashboard.

use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SESSION_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredSession {
    version: u32,
    #[serde(default)]
    dashboard_url: Option<String>,
    ws_url: String,
    access_token: String,
    expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub dashboard_url: String,
    pub ws_url: String,
    pub access_token: String,
}

#[derive(Deserialize)]
struct DeviceRequest {
    device_code: String,
    user_code: String,
    verification_uri: String,
    ws_url: String,
    expires_in: i64,
    interval: u64,
}

#[derive(Deserialize)]
struct DevicePoll {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    error: Option<String>,
}

fn now_unix() -> Result<i64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch")?
        .as_secs() as i64)
}

fn session_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/shellfleet/cli-session.json"))
}

fn base_url(arg: Option<&String>) -> Result<String, String> {
    let raw = arg
        .cloned()
        .or_else(|| std::env::var("SHELLFLEET_URL").ok())
        .ok_or("usage: shellfleet login https://dashboard.example.com")?;
    let url = reqwest::Url::parse(&raw).map_err(|_| "invalid ShellFleet URL")?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err("ShellFleet URL must be an absolute http(s) URL".into());
    }
    Ok(raw.trim_end_matches('/').to_string())
}

fn dashboard_url_from_ws(raw: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(raw).map_err(|_| "invalid ShellFleet WebSocket URL")?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return Err("ShellFleet WebSocket URL must use ws or wss".into()),
    };
    url.set_scheme(scheme)
        .map_err(|_| "invalid ShellFleet WebSocket scheme")?;
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn write_session(session: &StoredSession) -> Result<(), String> {
    let path = session_path()?;
    let parent = path.parent().ok_or("invalid session path")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(session).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn read_session() -> Result<StoredSession, String> {
    let path = session_path()?;
    let session: StoredSession = serde_json::from_slice(
        &std::fs::read(&path).map_err(|_| "not signed in; run shellfleet login <dashboard-url>")?,
    )
    .map_err(|_| "invalid CLI session file; run shellfleet login again")?;
    if session.version != SESSION_VERSION {
        return Err("unsupported CLI session file; run shellfleet login again".into());
    }
    if now_unix()? >= session.expires_at {
        return Err("CLI session expired; run shellfleet login again".into());
    }
    Ok(session)
}

pub async fn login(arg: Option<&String>) -> Result<(), String> {
    let base = base_url(arg)?;
    let client = reqwest::Client::builder()
        .https_only(base.starts_with("https://"))
        .build()
        .map_err(|error| error.to_string())?;
    let request: DeviceRequest = client
        .post(format!("{base}/api/cli-auth/request"))
        .send()
        .await
        .map_err(|error| format!("request CLI authorization: {error}"))?
        .error_for_status()
        .map_err(|error| format!("request CLI authorization: {error}"))?
        .json()
        .await
        .map_err(|error| format!("decode CLI authorization response: {error}"))?;

    println!(
        "Open {} and enter code: {}",
        request.verification_uri, request.user_code
    );
    println!(
        "Waiting for dashboard approval (expires in {} seconds)…",
        request.expires_in
    );

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(request.expires_in.max(1) as u64);
    let interval = Duration::from_secs(request.interval.clamp(2, 10));
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("CLI authorization expired; run shellfleet login again".into());
        }
        tokio::time::sleep(interval).await;
        let response: DevicePoll = client
            .post(format!("{base}/api/cli-auth/token"))
            .json(&serde_json::json!({ "device_code": &request.device_code }))
            .send()
            .await
            .map_err(|error| format!("poll CLI authorization: {error}"))?
            .error_for_status()
            .map_err(|error| format!("poll CLI authorization: {error}"))?
            .json()
            .await
            .map_err(|error| format!("decode CLI authorization response: {error}"))?;
        if let Some(token) = response.access_token {
            let expires_in = response
                .expires_in
                .ok_or("CLI token response missing expiry")?;
            let session = StoredSession {
                version: SESSION_VERSION,
                dashboard_url: Some(base.clone()),
                ws_url: request.ws_url,
                access_token: token,
                expires_at: now_unix()? + expires_in,
            };
            write_session(&session)?;
            println!("CLI authorized. Launch shellfleet to open the operator cockpit.");
            return Ok(());
        }
        match response.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("expired_token") => {
                return Err("CLI authorization expired; run login again".into());
            }
            Some("invalid_grant") => {
                return Err("CLI authorization was rejected; run login again".into());
            }
            Some("server_error") => return Err("server could not issue a CLI session".into()),
            Some(error) => return Err(format!("CLI authorization failed: {error}")),
            None => return Err("invalid CLI authorization response".into()),
        }
    }
}

pub fn logout() -> Result<(), String> {
    let path = session_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => {
            println!("CLI session removed.");
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("CLI is already signed out.");
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

pub fn connection() -> Result<Connection, String> {
    let env_ws = std::env::var("SHELLFLEET_WS_URL").ok();
    let env_token = std::env::var("SHELLFLEET_AUTH_TOKEN").ok();
    let env_dashboard = std::env::var("SHELLFLEET_URL")
        .ok()
        .map(|value| value.trim_end_matches('/').to_string());
    if let (Some(ws_url), Some(access_token)) = (&env_ws, &env_token) {
        let dashboard_url = env_dashboard
            .map(Ok)
            .unwrap_or_else(|| dashboard_url_from_ws(ws_url))?;
        return Ok(Connection {
            dashboard_url,
            ws_url: ws_url.clone(),
            access_token: access_token.clone(),
        });
    }
    let session = read_session()?;
    let ws_url = env_ws.unwrap_or(session.ws_url);
    let dashboard_url = env_dashboard
        .or(session.dashboard_url)
        .map(Ok)
        .unwrap_or_else(|| dashboard_url_from_ws(&ws_url))?;
    Ok(Connection {
        dashboard_url,
        ws_url,
        access_token: env_token.unwrap_or(session.access_token),
    })
}

#[cfg(test)]
mod tests {
    use super::dashboard_url_from_ws;

    #[test]
    fn derives_dashboard_origin_from_operator_websocket() {
        assert_eq!(
            dashboard_url_from_ws("wss://fleet.example/ui/ws").unwrap(),
            "https://fleet.example"
        );
        assert_eq!(
            dashboard_url_from_ws("ws://127.0.0.1:8080/ui/ws").unwrap(),
            "http://127.0.0.1:8080"
        );
    }
}
