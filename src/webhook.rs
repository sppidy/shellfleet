//! Outbound notification fan-out on update-window result.
//!
//! Up to four sinks can be configured via env. Each sink is
//! independent — set the ones you want, leave the rest unset. All
//! configured sinks are dispatched in parallel from a single
//! background task; failures are audited individually as
//! `webhook.update_result` rows so a Discord outage doesn't hide a
//! successful Telegram delivery.
//!
//! Supported sinks:
//!
//!   UPDATE_WEBHOOK_URL     — generic POST target. The body shape is
//!   UPDATE_WEBHOOK_FORMAT     controlled by `UPDATE_WEBHOOK_FORMAT`:
//!                             "json" (default, structured event) or
//!                             "slack" (Slack-attachment text). Useful
//!                             for Mattermost, n8n, custom receivers.
//!
//!   UPDATE_SLACK_WEBHOOK_URL  — alias for the generic + format=slack
//!                             pair. Set this if you only need Slack.
//!
//!   UPDATE_DISCORD_WEBHOOK_URL — Discord-native `content` payload.
//!                             Posts plain Markdown that renders
//!                             cleanly in Discord (Slack-style
//!                             payloads at Discord URLs lose the code
//!                             block and emoji). Append /slack to the
//!                             URL only if you intentionally want
//!                             Slack-compat.
//!
//!   UPDATE_TELEGRAM_BOT_TOKEN  — bot token from @BotFather
//!   UPDATE_TELEGRAM_CHAT_ID    — chat / channel / user id (numeric or
//!                             "@channelname"). Posted via the Bot API
//!                             with HTML parse_mode so log tails ride
//!                             inside <pre> without escaping every
//!                             markdown char.

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

fn slack_text(agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if status == "success" { ":white_check_mark:" } else { ":x:" };
    let mut text = format!("{icon} *sys-manager apt upgrade* on `{agent_id}` → *{status}*");
    if let Some(e) = error.filter(|s| !s.is_empty()) {
        text.push_str(&format!("\n> error: {e}"));
    }
    let tail = last_n_lines(log, 6);
    if !tail.is_empty() {
        text.push_str(&format!("\n```\n{tail}\n```"));
    }
    text
}

fn discord_text(agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if status == "success" { "✅" } else { "❌" };
    let mut text = format!("{icon} **sys-manager apt upgrade** on `{agent_id}` → **{status}**");
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

fn telegram_text(agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if status == "success" { "✅" } else { "❌" };
    let mut text = format!(
        "{icon} <b>sys-manager apt upgrade</b> on <code>{}</code> → <b>{}</b>",
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
    /// Generic JSON event — used by the legacy UPDATE_WEBHOOK_URL with
    /// UPDATE_WEBHOOK_FORMAT=json.
    GenericJson { url: String },
    /// Slack-format text. Backed by either UPDATE_WEBHOOK_URL with
    /// UPDATE_WEBHOOK_FORMAT=slack, or the dedicated
    /// UPDATE_SLACK_WEBHOOK_URL.
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

fn configured_sinks() -> Vec<Sink> {
    let mut sinks = Vec::new();

    // Legacy generic webhook. Format chooses between structured JSON
    // (good for Mattermost / n8n / custom) and Slack-style text.
    if let Ok(url) = std::env::var("UPDATE_WEBHOOK_URL") {
        if !url.is_empty() {
            let format = std::env::var("UPDATE_WEBHOOK_FORMAT")
                .unwrap_or_else(|_| "json".to_string());
            if format == "slack" {
                sinks.push(Sink::Slack { url });
            } else {
                sinks.push(Sink::GenericJson { url });
            }
        }
    }
    // Dedicated Slack URL (only added if distinct from the generic).
    if let Ok(url) = std::env::var("UPDATE_SLACK_WEBHOOK_URL") {
        if !url.is_empty() {
            sinks.push(Sink::Slack { url });
        }
    }
    if let Ok(url) = std::env::var("UPDATE_DISCORD_WEBHOOK_URL") {
        if !url.is_empty() {
            sinks.push(Sink::Discord { url });
        }
    }
    if let (Ok(token), Ok(chat_id)) = (
        std::env::var("UPDATE_TELEGRAM_BOT_TOKEN"),
        std::env::var("UPDATE_TELEGRAM_CHAT_ID"),
    ) {
        if !token.is_empty() && !chat_id.is_empty() {
            sinks.push(Sink::Telegram { bot_token: token, chat_id });
        }
    }

    sinks
}

/// Fire all configured webhooks (if any). Spawns ONE task that
/// dispatches to every sink in parallel and audits each result
/// independently.
pub fn fire_update_result(
    db: SqlitePool,
    agent_id: String,
    status: String,
    log: String,
    error: Option<String>,
    at: i64,
) {
    let sinks = configured_sinks();
    if sinks.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let truncated = truncate(&log, LOG_CAP);
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "webhook: client build failed");
                let _ = db::record_audit(
                    &db,
                    crate::now_unix(),
                    Some("webhook"),
                    Some(&agent_id),
                    "webhook.update_result",
                    false,
                    Some(&format!("client: {e}")),
                )
                .await;
                return;
            }
        };

        // Dispatch every sink as its own task; await them all so we
        // don't drop the pool reference before the requests finish.
        let mut handles = Vec::with_capacity(sinks.len());
        for sink in sinks {
            let client = client.clone();
            let db = db.clone();
            let agent_id = agent_id.clone();
            let status = status.clone();
            let error = error.clone();
            let truncated = truncated.clone();
            handles.push(tokio::spawn(async move {
                deliver(
                    &client,
                    db,
                    sink,
                    agent_id,
                    status,
                    error,
                    truncated,
                    at,
                )
                .await
            }));
        }
        for h in handles {
            let _ = h.await;
        }
    });
}

async fn deliver(
    client: &reqwest::Client,
    db: SqlitePool,
    sink: Sink,
    agent_id: String,
    status: String,
    error: Option<String>,
    log: String,
    at: i64,
) {
    let label = sink.label();
    let req = match &sink {
        Sink::GenericJson { url } => {
            let body = JsonPayload {
                event: "update_window.result",
                agent_id: &agent_id,
                status: &status,
                log: &log,
                error: error.as_deref(),
                at,
            };
            client.post(url).json(&body)
        }
        Sink::Slack { url } => {
            let body = SlackPayload {
                text: slack_text(&agent_id, &status, error.as_deref(), &log),
            };
            client.post(url).json(&body)
        }
        Sink::Discord { url } => {
            let body = DiscordPayload {
                content: discord_text(&agent_id, &status, error.as_deref(), &log),
            };
            client.post(url).json(&body)
        }
        Sink::Telegram { bot_token, chat_id } => {
            let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
            let body = TelegramPayload {
                chat_id,
                text: telegram_text(&agent_id, &status, error.as_deref(), &log),
                parse_mode: "HTML",
                disable_web_page_preview: true,
            };
            client.post(url).json(&body)
        }
    };

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                %agent_id, %status, sink = %label, code = resp.status().as_u16(),
                "webhook delivered"
            );
            db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&agent_id),
                "webhook.update_result",
                true,
                Some(&format!("sink={label} code={}", resp.status().as_u16())),
            )
            .await;
        }
        Ok(resp) => {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let snippet = body.chars().take(200).collect::<String>();
            tracing::warn!(%agent_id, sink = %label, code, body = %snippet, "webhook non-2xx");
            db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&agent_id),
                "webhook.update_result",
                false,
                Some(&format!("sink={label} code={code} body={snippet}")),
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(%agent_id, sink = %label, error = %e, "webhook send failed");
            db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&agent_id),
                "webhook.update_result",
                false,
                Some(&format!("sink={label} send: {e}")),
            )
            .await;
        }
    }
}
