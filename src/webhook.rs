//! Outbound notification fan-out for operationally-significant events.
//!
//! Up to four sink types per event can be configured via env. Each
//! sink is independent — set the ones you want, leave the rest unset.
//! All configured sinks are dispatched in parallel from a single
//! background task; failures are audited individually as `webhook.<kind>`
//! rows so a Discord outage doesn't hide a successful Telegram delivery.
//!
//! ## Per-event env prefixes
//!
//! Each event type reads its own `<PREFIX>_*` env vars so operators can
//! route them differently (e.g. nightly apt updates to a Slack channel,
//! red probe transitions to a more urgent Telegram bot, agent
//! disconnects to a phone-pinging webhook).
//!
//! | Event                       | Prefix       | Triggered by                 |
//! |-----------------------------|--------------|------------------------------|
//! | apt update window result    | `UPDATE_`    | `Message::AptUpgradeResponse`|
//! | health probe transition     | `HEALTH_`    | `Message::HealthProbeReport` |
//! | backup job result           | `BACKUP_`    | `Message::BackupRunResponse` |
//! | agent disconnect            | `DISCONNECT_`| WS receive-loop tear-down    |
//!
//! ## Sink suffixes (per prefix)
//!
//! | Suffix                      | Format                                |
//! |-----------------------------|---------------------------------------|
//! | `WEBHOOK_URL`               | Generic POST. `<PREFIX>_WEBHOOK_FORMAT`|
//! |                             | picks `json` (default, structured) or  |
//! |                             | `slack` (Slack-attachment text). Good  |
//! |                             | for Mattermost, n8n, custom receivers. |
//! | `SLACK_WEBHOOK_URL`         | Slack-format alias.                    |
//! | `DISCORD_WEBHOOK_URL`       | Discord-native `content` payload.      |
//! | `TELEGRAM_BOT_TOKEN` +      | Bot API `sendMessage` with HTML        |
//! | `TELEGRAM_CHAT_ID`          | parse_mode and `<pre>` log tail.       |

use serde::Serialize;
use sqlx::SqlitePool;

use crate::db;

const LOG_CAP: usize = 3_000;
/// Telegram caps message text at 4096 chars including markup. Stay
/// well under to leave room for the title + code-block delimiters.
const TELEGRAM_LOG_CAP: usize = 3_500;

#[derive(Serialize)]
struct JsonPayload<'a> {
    event: &'a str,
    agent_id: &'a str,
    status: &'a str,
    log: &'a str,
    error: Option<&'a str>,
    at: i64,
}

#[derive(Serialize)]
struct SlackPayload {
    text: String,
}

#[derive(Serialize)]
struct DiscordPayload {
    content: String,
}

#[derive(Serialize)]
struct TelegramPayload<'a> {
    chat_id: &'a str,
    text: String,
    parse_mode: &'static str,
    disable_web_page_preview: bool,
}

fn truncate(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        s.to_string()
    } else {
        let cut = s.len() - cap;
        format!("…[{cut} bytes truncated]…\n{}", &s[s.len() - cap..])
    }
}

fn last_n_lines(log: &str, n: usize) -> String {
    log.lines()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn slack_text(headline: &str, agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if matches!(status, "success" | "green") {
        ":white_check_mark:"
    } else if status == "disconnected" {
        ":warning:"
    } else {
        ":x:"
    };
    let mut text = format!("{icon} *{headline}* on `{agent_id}` → *{status}*");
    if let Some(e) = error.filter(|s| !s.is_empty()) {
        text.push_str(&format!("\n> error: {e}"));
    }
    let tail = last_n_lines(log, 6);
    if !tail.is_empty() {
        text.push_str(&format!("\n```\n{tail}\n```"));
    }
    text
}

fn discord_text(headline: &str, agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if matches!(status, "success" | "green") {
        "✅"
    } else if status == "disconnected" {
        "⚠️"
    } else {
        "❌"
    };
    let mut text = format!("{icon} **{headline}** on `{agent_id}` → **{status}**");
    if let Some(e) = error.filter(|s| !s.is_empty()) {
        text.push_str(&format!("\n> error: {e}"));
    }
    let tail = last_n_lines(log, 6);
    if !tail.is_empty() {
        text.push_str(&format!("\n```\n{tail}\n```"));
    }
    // Discord caps message at 2000 chars. Truncate hard on the way out.
    if text.len() > 1900 {
        let cut = text.len() - 1900;
        text = format!(
            "…[{cut} bytes truncated]…\n{}",
            &text[text.len() - 1900..]
        );
    }
    text
}

/// Minimal HTML escape for the small fields we drop into Telegram.
/// Telegram's HTML parser only treats `<`, `>`, `&` as special; we
/// don't emit `"` inside attributes so leave that alone.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn telegram_text(headline: &str, agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if matches!(status, "success" | "green") {
        "✅"
    } else if status == "disconnected" {
        "⚠️"
    } else {
        "❌"
    };
    let mut text = format!(
        "{icon} <b>{}</b> on <code>{}</code> → <b>{}</b>",
        html_escape(headline),
        html_escape(agent_id),
        html_escape(status),
    );
    if let Some(e) = error.filter(|s| !s.is_empty()) {
        text.push_str(&format!("\n<i>error:</i> {}", html_escape(e)));
    }
    let tail = last_n_lines(log, 8);
    if !tail.is_empty() {
        let body = truncate(&tail, TELEGRAM_LOG_CAP);
        text.push_str(&format!("\n<pre>{}</pre>", html_escape(&body)));
    }
    text
}

/// One configured destination. Each variant carries everything the
/// task needs to POST without re-reading env later.
enum Sink {
    /// Generic JSON event.
    GenericJson { url: String },
    /// Slack-format text. Backed by either the generic `*_WEBHOOK_URL`
    /// with format=slack, or the dedicated `*_SLACK_WEBHOOK_URL`.
    Slack { url: String },
    /// Discord webhook with native `content` field.
    Discord { url: String },
    /// Telegram Bot API. URL is derived from the token at fire time
    /// so the secret never lives in the struct.
    Telegram { bot_token: String, chat_id: String },
}

impl Sink {
    fn label(&self) -> &'static str {
        match self {
            Sink::GenericJson { .. } => "generic",
            Sink::Slack { .. } => "slack",
            Sink::Discord { .. } => "discord",
            Sink::Telegram { .. } => "telegram",
        }
    }
}

fn env_or_empty(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn configured_sinks(prefix: &str) -> Vec<Sink> {
    let mut sinks = Vec::new();

    // Generic webhook. Format chooses between structured JSON and
    // Slack-style text.
    let generic_url = env_or_empty(&format!("{prefix}WEBHOOK_URL"));
    if !generic_url.is_empty() {
        let format = std::env::var(format!("{prefix}WEBHOOK_FORMAT"))
            .unwrap_or_else(|_| "json".to_string());
        if format == "slack" {
            sinks.push(Sink::Slack { url: generic_url });
        } else {
            sinks.push(Sink::GenericJson { url: generic_url });
        }
    }
    let slack_url = env_or_empty(&format!("{prefix}SLACK_WEBHOOK_URL"));
    if !slack_url.is_empty() {
        sinks.push(Sink::Slack { url: slack_url });
    }
    let discord_url = env_or_empty(&format!("{prefix}DISCORD_WEBHOOK_URL"));
    if !discord_url.is_empty() {
        sinks.push(Sink::Discord { url: discord_url });
    }
    let token = env_or_empty(&format!("{prefix}TELEGRAM_BOT_TOKEN"));
    let chat_id = env_or_empty(&format!("{prefix}TELEGRAM_CHAT_ID"));
    if !token.is_empty() && !chat_id.is_empty() {
        sinks.push(Sink::Telegram { bot_token: token, chat_id });
    }

    sinks
}

/// Per-event payload. Pre-rendered headline + status + optional log
/// gives every sink the same data; each sink formatter chooses how
/// to render it.
#[derive(Clone)]
struct Event {
    /// Audit-row kind suffix. The audit row will be
    /// `webhook.<kind>` so callers and `/activity` can filter by it.
    kind: &'static str,
    /// Used in the JSON sink as the `event` field, and templated into
    /// every chat-format body.
    headline: String,
    agent_id: String,
    status: String,
    log: String,
    error: Option<String>,
    at: i64,
}

/// Fire all sinks configured under `prefix` for `event`. No-op when
/// no sinks are configured. Spawns ONE task that dispatches to every
/// sink in parallel and audits each result independently.
fn fire(db: SqlitePool, prefix: &'static str, event: Event) {
    let sinks = configured_sinks(prefix);
    if sinks.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let truncated = truncate(&event.log, LOG_CAP);
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, kind = %event.kind, "webhook: client build failed");
                let _ = db::record_audit(
                    &db,
                    crate::now_unix(),
                    Some("webhook"),
                    Some(&event.agent_id),
                    &format!("webhook.{}", event.kind),
                    false,
                    Some(&format!("client: {e}")),
                )
                .await;
                return;
            }
        };

        let mut handles = Vec::with_capacity(sinks.len());
        for sink in sinks {
            let client = client.clone();
            let db = db.clone();
            let mut event = event.clone();
            event.log = truncated.clone();
            handles.push(tokio::spawn(async move {
                deliver(&client, db, sink, event).await
            }));
        }
        for h in handles {
            let _ = h.await;
        }
    });
}

async fn deliver(client: &reqwest::Client, db: SqlitePool, sink: Sink, event: Event) {
    let label = sink.label();
    let audit_kind = format!("webhook.{}", event.kind);
    let req = match &sink {
        Sink::GenericJson { url } => {
            let body = JsonPayload {
                event: event.kind,
                agent_id: &event.agent_id,
                status: &event.status,
                log: &event.log,
                error: event.error.as_deref(),
                at: event.at,
            };
            client.post(url).json(&body)
        }
        Sink::Slack { url } => {
            let body = SlackPayload {
                text: slack_text(
                    &event.headline,
                    &event.agent_id,
                    &event.status,
                    event.error.as_deref(),
                    &event.log,
                ),
            };
            client.post(url).json(&body)
        }
        Sink::Discord { url } => {
            let body = DiscordPayload {
                content: discord_text(
                    &event.headline,
                    &event.agent_id,
                    &event.status,
                    event.error.as_deref(),
                    &event.log,
                ),
            };
            client.post(url).json(&body)
        }
        Sink::Telegram { bot_token, chat_id } => {
            let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
            let body = TelegramPayload {
                chat_id,
                text: telegram_text(
                    &event.headline,
                    &event.agent_id,
                    &event.status,
                    event.error.as_deref(),
                    &event.log,
                ),
                parse_mode: "HTML",
                disable_web_page_preview: true,
            };
            client.post(url).json(&body)
        }
    };

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                agent_id = %event.agent_id,
                status = %event.status,
                kind = %event.kind,
                sink = %label,
                code = resp.status().as_u16(),
                "webhook delivered"
            );
            db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&event.agent_id),
                &audit_kind,
                true,
                Some(&format!("sink={label} code={}", resp.status().as_u16())),
            )
            .await;
        }
        Ok(resp) => {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let snippet = body.chars().take(200).collect::<String>();
            tracing::warn!(
                agent_id = %event.agent_id,
                kind = %event.kind,
                sink = %label,
                code,
                body = %snippet,
                "webhook non-2xx"
            );
            db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&event.agent_id),
                &audit_kind,
                false,
                Some(&format!("sink={label} code={code} body={snippet}")),
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(
                agent_id = %event.agent_id,
                kind = %event.kind,
                sink = %label,
                error = %e,
                "webhook send failed"
            );
            db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&event.agent_id),
                &audit_kind,
                false,
                Some(&format!("sink={label} send: {e}")),
            )
            .await;
        }
    }
}

// ─── public fires ──────────────────────────────────────────────

/// `update_window.result` — apt-upgrade scheduler outcome.
/// Reads `UPDATE_*` env. Original event, kept for backwards compat.
pub fn fire_update_result(
    db: SqlitePool,
    agent_id: String,
    status: String,
    log: String,
    error: Option<String>,
    at: i64,
) {
    fire(
        db,
        "UPDATE_",
        Event {
            kind: "update_result",
            headline: "sys-manager apt upgrade".into(),
            agent_id,
            status,
            log,
            error,
            at,
        },
    );
}

/// `health_probe.transition` — fired only on green↔red flips, not
/// every report. Reads `HEALTH_*` env. `probe_name` rides in the
/// headline so the chat preview shows which probe transitioned.
pub fn fire_health_probe_transition(
    db: SqlitePool,
    agent_id: String,
    probe_name: String,
    state: shared::HealthProbeState,
    detail: String,
    at: i64,
) {
    let status = match state {
        shared::HealthProbeState::Green => "green",
        shared::HealthProbeState::Red => "red",
    };
    fire(
        db,
        "HEALTH_",
        Event {
            kind: "health_probe.transition",
            headline: format!("sys-manager health probe `{probe_name}`"),
            agent_id,
            status: status.into(),
            log: detail,
            error: None,
            at,
        },
    );
}

/// `backup_job.result` — finished backup run. Reads `BACKUP_*` env.
/// `name` (the operator's job name) goes into the headline.
pub fn fire_backup_result(
    db: SqlitePool,
    agent_id: String,
    name: String,
    success: bool,
    log: String,
    error: Option<String>,
    at: i64,
) {
    fire(
        db,
        "BACKUP_",
        Event {
            kind: "backup_job.result",
            headline: format!("sys-manager backup `{name}`"),
            agent_id,
            status: if success { "success".into() } else { "failed".into() },
            log,
            error,
            at,
        },
    );
}

/// `agent.disconnect` — fired when an agent's WS read loop exits and
/// the server removes it from the live agents map. Reads `DISCONNECT_*`
/// env. No log body; the chat formatters will skip the code block.
pub fn fire_agent_disconnect(db: SqlitePool, agent_id: String, at: i64) {
    fire(
        db,
        "DISCONNECT_",
        Event {
            kind: "agent.disconnect",
            headline: "sys-manager agent".into(),
            agent_id,
            status: "disconnected".into(),
            log: String::new(),
            error: None,
            at,
        },
    );
}
