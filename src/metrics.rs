//! Prometheus metrics plugin (CE).
//!
//! ShellFleet doesn't store time-series — that's Prometheus's job.
//! This module turns the dashboard into a thin renderer for the
//! operator's existing Prometheus: configure named panel templates
//! in YAML, and a per-agent `Metrics` tab in the web UI calls
//! `/api/metrics/query` with `{ panel, agent_id, range }` to get a
//! JSON time-series back.
//!
//! ## CE / EE split
//!
//! - **CE**: one Prometheus URL. All panel features (templates,
//!   `{instance}` substitution, range picker, viewer-readable).
//! - **EE** (separate sidecar, future): multiple Prometheus
//!   instances, federated/per-tenant routing, plus the SaaS
//!   observability vendors (Datadog, New Relic, Grafana Cloud).
//!
//! ## Configuration
//!
//! Path resolved from `METRICS_CONFIG_PATH` (default
//! `/etc/shellfleet/metrics.yaml`). If the file is missing or
//! invalid the plugin stays disabled — the `/api/metrics/panels`
//! endpoint returns an empty list and the web UI hides the tab.
//! See `metrics.example.yaml` in the repo root for the schema.
//!
//! ## Auth
//!
//! Both endpoints require an authenticated session (RBAC middleware
//! enforces this). Both are READ-ONLY and explicitly viewer-allowed
//! — the value of "see graphs without write power" is real, and
//! Prometheus is read-only by definition. There is no free-form
//! PromQL from the client; the dashboard sends a panel ID and the
//! server interpolates a server-side template, so a viewer can't
//! craft an expensive query.
//!
//! ## Throttle
//!
//! Per-actor query throttle reuses the existing `Throttle` shape
//! (10 fails / 15 min lock). Each Prometheus call has a hard 10 s
//! upstream timeout and a 1000-row response cap on data points.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::{AppState, auth};

async fn proxy_get_to_ee(ee_url: &str, path: &str) -> axum::response::Response {
    let url = format!("{}{}", ee_url.trim_end_matches('/'), path);
    let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
    match reqwest::Client::new()
        .get(&url)
        .bearer_auth(&secret)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "EE metrics proxy failed");
            (StatusCode::BAD_GATEWAY, "EE sidecar unavailable").into_response()
        }
    }
}

async fn proxy_post_json_to_ee(
    ee_url: &str,
    path: &str,
    body: &impl Serialize,
) -> axum::response::Response {
    let url = format!("{}{}", ee_url.trim_end_matches('/'), path);
    let secret = std::env::var("EE_INTERNAL_SECRET").unwrap_or_default();
    match reqwest::Client::new()
        .post(&url)
        .bearer_auth(&secret)
        .json(body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "EE metrics proxy failed");
            (StatusCode::BAD_GATEWAY, "EE sidecar unavailable").into_response()
        }
    }
}

const DEFAULT_CONFIG_PATH: &str = "/etc/shellfleet/metrics.yaml";
const PROM_TIMEOUT_SECS: u64 = 10;
const MAX_POINTS: usize = 5_000;

// ---------------------------------------------------------------------
// Config schema (YAML on disk)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct RawConfig {
    prometheus: Option<RawPrometheus>,
    #[serde(default)]
    panels: Vec<RawPanel>,
    /// Optional explicit map from agent_id (with the `-id` suffix) to
    /// the Prometheus `instance` label, when the convention isn't
    /// just "strip `-id`". e.g. node_exporter on `host:9100`.
    #[serde(default)]
    agent_instance_map: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPrometheus {
    url: String,
    #[serde(default)]
    bearer_token: Option<String>,
    #[serde(default)]
    basic_auth: Option<RawBasicAuth>,
    #[serde(default)]
    tls: Option<RawTls>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawBasicAuth {
    username: String,
    password: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawTls {
    #[serde(default)]
    insecure_skip_verify: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPanel {
    id: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    /// PromQL query template. `{agent_id}` and `{instance}` get
    /// substituted before the request goes upstream.
    query: String,
    #[serde(default = "default_unit")]
    unit: String,
}

fn default_unit() -> String {
    "raw".to_string()
}

// ---------------------------------------------------------------------
// Resolved (post-env-substitution) config
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Config {
    prometheus: PrometheusConfig,
    panels: HashMap<String, Panel>,
    agent_instance_map: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct PrometheusConfig {
    base_url: String, // e.g. https://prom.internal/api/v1
    bearer_token: Option<String>,
    basic_auth: Option<(String, String)>,
    insecure_skip_verify: bool,
    timeout_secs: u64,
}

#[derive(Debug, Clone)]
struct Panel {
    id: String,
    title: String,
    description: Option<String>,
    query: String,
    unit: String,
}

/// Expand `${VAR}` references against the process env. Operators put
/// secrets like `${PROMETHEUS_TOKEN}` in the YAML; we don't want
/// them sitting in plaintext alongside the rest of the config.
fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut name = String::new();
            let mut closed = false;
            while let Some(&n) = chars.peek() {
                if n == '}' {
                    chars.next();
                    closed = true;
                    break;
                }
                name.push(n);
                chars.next();
            }
            if closed {
                if let Ok(v) = std::env::var(&name) {
                    out.push_str(&v);
                } else {
                    // Unknown env: leave the original token in place
                    // so a misconfiguration is visible at runtime.
                    out.push_str("${");
                    out.push_str(&name);
                    out.push('}');
                }
            } else {
                out.push('$');
                out.push('{');
                out.push_str(&name);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn load_config_from_path(path: &str) -> Option<Config> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::info!(
                error = %e,
                path = %path,
                "metrics: config not loaded; plugin disabled"
            );
            return None;
        }
    };
    let raw = expand_env(&raw);
    let cfg: RawConfig = match serde_yaml::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %path, "metrics: YAML parse failed; plugin disabled");
            return None;
        }
    };
    let prom = match cfg.prometheus {
        Some(p) => PrometheusConfig {
            base_url: p.url.trim_end_matches('/').to_string(),
            bearer_token: p.bearer_token.filter(|s| !s.is_empty()),
            basic_auth: p.basic_auth.map(|b| (b.username, b.password)),
            insecure_skip_verify: p.tls.unwrap_or_default().insecure_skip_verify,
            timeout_secs: p.timeout_secs.unwrap_or(PROM_TIMEOUT_SECS),
        },
        None => {
            tracing::warn!(path = %path, "metrics: no `prometheus:` block; plugin disabled");
            return None;
        }
    };
    let mut panels = HashMap::with_capacity(cfg.panels.len());
    for p in cfg.panels {
        if panels.contains_key(&p.id) {
            tracing::warn!(panel = %p.id, "metrics: duplicate panel id; later definition wins");
        }
        panels.insert(
            p.id.clone(),
            Panel {
                id: p.id,
                title: p.title,
                description: p.description,
                query: p.query,
                unit: p.unit,
            },
        );
    }
    if panels.is_empty() {
        tracing::info!(path = %path, "metrics: no panels configured; plugin loaded but empty");
    } else {
        tracing::info!(path = %path, panels = panels.len(), "metrics: plugin enabled");
    }
    Some(Config {
        prometheus: prom,
        panels,
        agent_instance_map: cfg.agent_instance_map,
    })
}

fn config() -> Option<&'static Config> {
    static CFG: OnceLock<Option<Config>> = OnceLock::new();
    CFG.get_or_init(|| {
        let path = std::env::var("METRICS_CONFIG_PATH")
            .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        load_config_from_path(&path)
    })
    .as_ref()
}

// ---------------------------------------------------------------------
// HTTP client (built once, reused)
// ---------------------------------------------------------------------

fn build_client(prom: &PrometheusConfig) -> Result<reqwest::Client, reqwest::Error> {
    let mut b =
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(prom.timeout_secs));
    if prom.insecure_skip_verify {
        b = b.danger_accept_invalid_certs(true);
    }
    b.build()
}

// ---------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/panels", get(panels_handler))
        .route("/query", post(query_handler))
}

#[derive(Serialize)]
struct PanelInfo<'a> {
    id: &'a str,
    title: &'a str,
    description: Option<&'a str>,
    unit: &'a str,
}

#[derive(Serialize)]
struct PanelsResponse<'a> {
    enabled: bool,
    panels: Vec<PanelInfo<'a>>,
}

async fn panels_handler(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Err(err) = auth::current_user(&jar, &state.db).await {
        return err.into_response();
    }
    if let Some(ee_url) = crate::ee::ee_sidecar_url() {
        return proxy_get_to_ee(&ee_url, "/api/ee/metrics/panels").await;
    }
    let Some(cfg) = config() else {
        return Json(PanelsResponse {
            enabled: false,
            panels: Vec::new(),
        })
        .into_response();
    };
    let mut panels: Vec<PanelInfo> = cfg
        .panels
        .values()
        .map(|p| PanelInfo {
            id: &p.id,
            title: &p.title,
            description: p.description.as_deref(),
            unit: &p.unit,
        })
        .collect();
    panels.sort_by(|a, b| a.title.cmp(b.title));
    Json(PanelsResponse {
        enabled: true,
        panels,
    })
    .into_response()
}

#[derive(Deserialize, Serialize)]
struct QueryRequest {
    panel: String,
    agent_id: String,
    /// One of "1h" | "6h" | "24h" | "7d". Defaults to "1h".
    #[serde(default = "default_range")]
    range: String,
}

fn default_range() -> String {
    "1h".to_string()
}

#[derive(Serialize)]
struct QueryResponse {
    panel_id: String,
    title: String,
    unit: String,
    /// One series per Prometheus result row. Each series carries a
    /// human-readable label (built from the metric labels minus
    /// `__name__`/`instance`/`job`) and `(unix_secs, value_f64)`
    /// points.
    series: Vec<Series>,
    /// PromQL we actually sent — useful for "why is this empty?"
    /// debugging from the dashboard.
    expanded_query: String,
    /// Prometheus's own status. Mirrors "success" / "error".
    upstream_status: String,
    /// If upstream returned an error, surfaced for display.
    upstream_error: Option<String>,
}

#[derive(Serialize)]
struct Series {
    label: String,
    points: Vec<(i64, f64)>,
}

fn step_for_range(range: &str) -> (i64, &'static str) {
    // (window_secs, step_str_for_prometheus)
    match range {
        "6h" => (6 * 3600, "60s"),
        "24h" => (24 * 3600, "5m"),
        "7d" => (7 * 24 * 3600, "30m"),
        _ => (3600, "30s"), // 1h default
    }
}

fn resolve_instance(cfg: &Config, agent_id: &str) -> String {
    if let Some(mapped) = cfg.agent_instance_map.get(agent_id) {
        return mapped.clone();
    }
    // Default convention: strip the trailing `-id` the server tacks
    // onto every agent. Operators using node_exporter typically have
    // the host's bare name as the `instance` label.
    agent_id.strip_suffix("-id").unwrap_or(agent_id).to_string()
}

fn build_label(metric: &serde_json::Map<String, serde_json::Value>) -> String {
    // Exclude noisy boilerplate labels; show what's distinctive.
    let mut bits: Vec<String> = metric
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "__name__" | "instance" | "job"))
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
        .collect();
    bits.sort();
    if bits.is_empty() {
        // Fall back to whatever's there.
        if let Some(name) = metric.get("__name__").and_then(|v| v.as_str()) {
            return name.to_string();
        }
        return "value".to_string();
    }
    bits.join(", ")
}

async fn query_handler(
    jar: CookieJar,
    State(state): State<Arc<AppState>>,
    Json(body): Json<QueryRequest>,
) -> impl IntoResponse {
    let claims = match auth::current_user(&jar, &state.db).await {
        Ok(c) => c,
        Err(err) => return err.into_response(),
    };

    if let Some(ee_url) = crate::ee::ee_sidecar_url() {
        return proxy_post_json_to_ee(&ee_url, "/api/ee/metrics/query", &body).await;
    }

    let Some(cfg) = config() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics plugin not configured",
        )
            .into_response();
    };
    let Some(panel) = cfg.panels.get(&body.panel) else {
        return (StatusCode::NOT_FOUND, "no such panel").into_response();
    };

    // Per-actor throttle. Reuses the existing MFA-class throttle so
    // a misbehaving dashboard doesn't hammer Prometheus.
    let now = crate::now_unix();
    if let crate::throttle::CheckResult::Locked { retry_after_secs } = state
        .mfa_throttle
        .check(&format!("metrics:{}", claims.sub), now)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(
                axum::http::header::RETRY_AFTER,
                retry_after_secs.to_string(),
            )],
            "metrics queries: too many failures, try again later",
        )
            .into_response();
    }

    let instance = resolve_instance(cfg, &body.agent_id);
    let expanded = panel
        .query
        .replace("{agent_id}", &body.agent_id)
        .replace("{instance}", &instance)
        .replace("{hostname}", &instance);

    let (window_secs, step) = step_for_range(&body.range);
    let end = now;
    let start = end - window_secs;

    let url = format!("{}/query_range", cfg.prometheus.base_url);
    let client = match build_client(&cfg.prometheus) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "metrics: client build failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "client build failed").into_response();
        }
    };

    let mut req = client.post(&url).form(&[
        ("query", expanded.as_str()),
        ("start", &start.to_string()),
        ("end", &end.to_string()),
        ("step", step),
    ]);
    if let Some(ref tok) = cfg.prometheus.bearer_token {
        req = req.bearer_auth(tok);
    }
    if let Some((ref u, ref p)) = cfg.prometheus.basic_auth {
        req = req.basic_auth(u, Some(p));
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            state
                .mfa_throttle
                .record_failure(&format!("metrics:{}", claims.sub), now);
            tracing::warn!(error = %e, panel = %body.panel, "metrics: prom request failed");
            return (StatusCode::BAD_GATEWAY, format!("prometheus: {e}")).into_response();
        }
    };
    let code = resp.status();
    let raw = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, format!("prometheus body: {e}")).into_response();
        }
    };
    if !code.is_success() {
        let snippet = raw.chars().take(300).collect::<String>();
        return (
            StatusCode::BAD_GATEWAY,
            format!("prometheus {}: {snippet}", code.as_u16()),
        )
            .into_response();
    }

    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, format!("prometheus json: {e}")).into_response();
        }
    };

    let upstream_status = parsed
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();
    let upstream_error = parsed
        .get("error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut series: Vec<Series> = Vec::new();
    if upstream_status == "success" {
        if let Some(result) = parsed
            .get("data")
            .and_then(|d| d.get("result"))
            .and_then(|r| r.as_array())
        {
            for entry in result {
                let label = entry
                    .get("metric")
                    .and_then(|m| m.as_object())
                    .map(build_label)
                    .unwrap_or_else(|| "value".into());
                let mut points: Vec<(i64, f64)> = Vec::new();
                if let Some(values) = entry.get("values").and_then(|v| v.as_array()) {
                    for pair in values {
                        if let Some(arr) = pair.as_array() {
                            if arr.len() == 2 {
                                let ts = arr[0].as_f64().unwrap_or(0.0) as i64;
                                let val = arr[1]
                                    .as_str()
                                    .and_then(|s| s.parse::<f64>().ok())
                                    .unwrap_or(f64::NAN);
                                points.push((ts, val));
                                if points.len() >= MAX_POINTS {
                                    break;
                                }
                            }
                        }
                    }
                }
                series.push(Series { label, points });
            }
        }
    }

    state
        .mfa_throttle
        .record_success(&format!("metrics:{}", claims.sub));

    Json(QueryResponse {
        panel_id: panel.id.clone(),
        title: panel.title.clone(),
        unit: panel.unit.clone(),
        series,
        expanded_query: expanded,
        upstream_status,
        upstream_error,
    })
    .into_response()
}
