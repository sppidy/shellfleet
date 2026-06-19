//! Outbound notification fan-out for operationally-significant events.
//!
//! Up to four sink types per event can be configured via env. Each
//! sink is independent — set the ones you want, leave the rest unset.
//! All configured sinks are dispatched in parallel from a single
//! background task; failures are audited individually as `webhook.<kind>`
//! rows so a Discord outage doesn't hide a successful Telegram delivery.
//!
//! ## Per-event env prefixes — with a default-prefix fallback
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
//! **Want one config for everything?** Set the prefix-less vars
//! (`WEBHOOK_URL`, `WEBHOOK_FORMAT`, `SLACK_WEBHOOK_URL`,
//! `DISCORD_WEBHOOK_URL`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`).
//! Every event falls back to those when its own `<PREFIX>_*` is unset.
//! Per-event vars still override when both are set, so you can route
//! one event differently while leaving the rest on the default.
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

use std::sync::{Arc, OnceLock};

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, Semaphore};

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
        return s.to_string();
    }
    // Slicing at `s.len() - cap` blind would panic if the index
    // lands inside a multi-byte UTF-8 character. Walk forward to
    // the next char boundary (guaranteed to exist at s.len()).
    let mut start = s.len() - cap;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    let cut = start;
    format!("…[{cut} bytes truncated]…\n{}", &s[start..])
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
    let icon = if matches!(status, "success" | "green" | "connected") {
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
    let icon = if matches!(status, "success" | "green" | "connected") {
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

/// HTML escape for fields we drop into Telegram message bodies AND
/// for any upstream-sourced text we persist in audit rows. Telegram's
/// HTML parser only treats `<`, `>`, `&` as special, but we cover
/// `'` and `"` too so the same helper is safe for any HTML-rendered
/// audit-log viewer that might escape attributes differently.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&#39;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Strip ANSI CSI escape sequences, BiDi overrides, and other C0/C1
/// control characters (keeping `\t`, `\n`, `\r`) from `s`. Used on
/// upstream-sourced text before it lands in audit rows or logs:
/// without this, a malicious sink response could embed cursor
/// movements, color codes, or right-to-left overrides that confuse
/// an operator reading `/activity`.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI: ESC `[` … <final byte in 0x40..=0x7E>. Drop the
            // entire sequence. A bare ESC (no `[`) is also dropped.
            if matches!(chars.peek(), Some(&'[')) {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if matches!(n, '\u{40}'..='\u{7e}') {
                        break;
                    }
                }
            }
            continue;
        }
        // BiDi overrides — Trojan-Source-style attacks.
        if matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') {
            continue;
        }
        if c.is_control() && c != '\t' && c != '\n' && c != '\r' {
            continue;
        }
        out.push(c);
    }
    out
}

/// A bot token that intentionally redacts itself in `Debug` output
/// so an accidental `tracing::debug!(?sink)` somewhere in the
/// dispatcher can't leak it into structured logs. The reqwest
/// trace target (`RUST_LOG=reqwest=trace`) still sees the URL we
/// build — operators must filter that target out in production.
#[derive(Clone)]
struct RedactedToken(String);

impl std::fmt::Debug for RedactedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RedactedToken(<redacted>)")
    }
}

impl RedactedToken {
    fn expose(&self) -> &str {
        &self.0
    }
}

fn telegram_text(headline: &str, agent_id: &str, status: &str, error: Option<&str>, log: &str) -> String {
    let icon = if matches!(status, "success" | "green" | "connected") {
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
    /// Telegram Bot API. The token is wrapped in `RedactedToken` so
    /// it never lands in `?sink`-style debug prints; only `expose()`
    /// hands it back as a `&str` for URL construction.
    Telegram { bot_token: RedactedToken, chat_id: String },
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

/// Read a prefixed var, falling back to the bare (default) var when
/// the prefixed one is unset or empty. Lets operators set one
/// `WEBHOOK_URL` for everything and selectively override per event.
fn env_with_fallback(prefix: &str, suffix: &str) -> String {
    let prefixed = env_or_empty(&format!("{prefix}{suffix}"));
    if !prefixed.is_empty() {
        return prefixed;
    }
    env_or_empty(suffix)
}

fn configured_sinks(prefix: &str) -> Vec<Sink> {
    let mut sinks = Vec::new();

    // Generic webhook. Format chooses between structured JSON and
    // Slack-style text. Format also falls back to the prefix-less var.
    let generic_url = env_with_fallback(prefix, "WEBHOOK_URL");
    if !generic_url.is_empty() {
        let format_specific = env_or_empty(&format!("{prefix}WEBHOOK_FORMAT"));
        let format = if !format_specific.is_empty() {
            format_specific
        } else {
            std::env::var("WEBHOOK_FORMAT").unwrap_or_else(|_| "json".to_string())
        };
        if format == "slack" {
            sinks.push(Sink::Slack { url: generic_url });
        } else {
            sinks.push(Sink::GenericJson { url: generic_url });
        }
    }
    let slack_url = env_with_fallback(prefix, "SLACK_WEBHOOK_URL");
    if !slack_url.is_empty() {
        sinks.push(Sink::Slack { url: slack_url });
    }
    let discord_url = env_with_fallback(prefix, "DISCORD_WEBHOOK_URL");
    if !discord_url.is_empty() {
        sinks.push(Sink::Discord { url: discord_url });
    }
    let token = env_with_fallback(prefix, "TELEGRAM_BOT_TOKEN");
    let chat_id = env_with_fallback(prefix, "TELEGRAM_CHAT_ID");
    if !token.is_empty() && !chat_id.is_empty() {
        sinks.push(Sink::Telegram {
            bot_token: RedactedToken(token),
            chat_id,
        });
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

/// One job pushed onto the dispatch queue. Held briefly while the
/// worker acquires a concurrency permit; cheap to enqueue.
struct DispatchJob {
    db: SqlitePool,
    prefix: &'static str,
    event: Event,
}

/// Bounded queue and concurrency cap for outbound HTTP. The previous
/// implementation `tokio::spawn`d unboundedly per-event, which let a
/// burst of probe transitions or a slow upstream (Telegram timeout
/// queueing behind reqwest's connection pool) accumulate tasks
/// without limit. With a bounded mpsc + a permit-gated worker,
/// backpressure is visible: try_send returns Err on full and we
/// drop-with-audit instead of growing memory.
const QUEUE_CAP: usize = 1_000;
const MAX_CONCURRENT_DELIVERIES: usize = 20;

static DISPATCH_TX: OnceLock<mpsc::Sender<DispatchJob>> = OnceLock::new();

fn dispatcher() -> &'static mpsc::Sender<DispatchJob> {
    DISPATCH_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<DispatchJob>(QUEUE_CAP);
        tokio::spawn(dispatcher_loop(rx));
        tx
    })
}

async fn dispatcher_loop(mut rx: mpsc::Receiver<DispatchJob>) {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DELIVERIES));
    while let Some(job) = rx.recv().await {
        // Wait for a permit. If acquire_owned fails, the semaphore
        // was closed which means we're shutting down — bail.
        let permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };
        tokio::spawn(async move {
            let _permit = permit;
            fire_inner(job.db, job.prefix, job.event).await;
        });
    }
}

/// Fire all sinks configured under `prefix` for `event`. No-op when
/// no sinks are configured. Enqueues a single dispatch job onto the
/// global bounded queue; on full queue, records a `webhook.<kind>`
/// failure audit row and drops the event rather than blocking the
/// caller (the WS receive path) or spawning unboundedly.
fn fire(db: SqlitePool, prefix: &'static str, event: Event) {
    if configured_sinks(prefix).is_empty() {
        return;
    }
    let tx = dispatcher();
    let job = DispatchJob {
        db: db.clone(),
        prefix,
        event: event.clone(),
    };
    if let Err(e) = tx.try_send(job) {
        let kind = event.kind;
        tracing::warn!(
            error = %e,
            kind = %kind,
            agent_id = %event.agent_id,
            "webhook: dispatch queue full, dropping event"
        );
        let agent_id = event.agent_id;
        tokio::spawn(async move {
            let _ = db::record_audit(
                &db,
                crate::now_unix(),
                Some("webhook"),
                Some(&agent_id),
                &format!("webhook.{kind}"),
                false,
                Some("dispatch queue full, event dropped"),
            )
            .await;
        });
    }
}

async fn fire_inner(db: SqlitePool, prefix: &'static str, event: Event) {
    let sinks = configured_sinks(prefix);
    if sinks.is_empty() {
        return;
    }
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
            let url = format!(
                "https://api.telegram.org/bot{}/sendMessage",
                bot_token.expose(),
            );
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
            // Sanitize before logging or auditing: an upstream sink
            // could return ANSI control sequences, BiDi overrides,
            // or HTML-special chars that confuse a `/activity`
            // viewer. Strip controls first, then HTML-escape what's
            // left. Cap length AFTER sanitize so escapes don't blow
            // the budget by 6× (`&#39;` per quote).
            let snippet: String = strip_ansi(&body).chars().take(200).collect();
            let snippet = html_escape(&snippet);
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
            headline: "shellfleet apt upgrade".into(),
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
            headline: format!("shellfleet health probe `{probe_name}`"),
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
            headline: format!("shellfleet backup `{name}`"),
            agent_id,
            status: if success { "success".into() } else { "failed".into() },
            log,
            error,
            at,
        },
    );
}

/// `agent.disconnect` — fired when an agent's WS read loop exits and
/// the server removes it from the live agents map *and the
/// reconnect grace window elapses without a re-register*. Reads
/// `DISCONNECT_*` env. No log body; the chat formatters will skip
/// the code block. The grace window is owned by `handle_agent_socket`
/// in `main.rs`; this helper just posts the webhook.
pub fn fire_agent_disconnect(db: SqlitePool, agent_id: String, at: i64) {
    fire(
        db,
        "DISCONNECT_",
        Event {
            kind: "agent.disconnect",
            headline: "shellfleet agent".into(),
            agent_id,
            status: "disconnected".into(),
            log: String::new(),
            error: None,
            at,
        },
    );
}

/// `agent.connect` — fired when an agent registers AFTER its
/// `agent.disconnect` webhook had already gone out (i.e. it was
/// offline past the 50s grace window so the operator was told it
/// left; now they want to know it's back). Transient blips that
/// reconnect within the grace window do NOT fire this — neither
/// disconnect nor connect goes out for those. First-time
/// registrations also stay silent (no prior disconnect to clear).
/// Reads `CONNECT_*` env (falling back to the bare prefix-less
/// defaults).
pub fn fire_agent_connect(db: SqlitePool, agent_id: String, at: i64) {
    fire(
        db,
        "CONNECT_",
        Event {
            kind: "agent.connect",
            headline: "shellfleet agent".into(),
            agent_id,
            status: "connected".into(),
            log: String::new(),
            error: None,
            at,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_returns_short_strings_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_cuts_on_utf8_boundary_without_panic() {
        // Mixed 1/2/3-byte chars; a small cap forces a cut that, taken
        // blindly, would land inside a multi-byte character.
        let s = "héllo wörld ☃ über alles";
        let out = truncate(s, 8);
        assert!(out.contains("truncated"), "must carry the truncation marker");
        let tail = out.rsplit('\n').next().unwrap();
        assert!(s.ends_with(tail), "kept tail must be a valid char-boundary suffix");
    }

    #[test]
    fn strip_ansi_drops_escapes_bidi_and_control() {
        assert_eq!(strip_ansi("\u{1b}[31mhi\u{1b}[0m"), "hi");
        // RTL override (Trojan-Source-style) is dropped.
        assert_eq!(strip_ansi("a\u{202e}b"), "ab");
        // Tabs / newlines / CR are preserved.
        assert_eq!(strip_ansi("a\tb\nc\rd"), "a\tb\nc\rd");
    }
}
