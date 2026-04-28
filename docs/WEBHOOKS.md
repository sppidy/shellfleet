# Outbound webhooks

The server fires webhooks on four operationally-significant events.
Each delivery is independent: one configured sink failing doesn't
hide a successful one. Every attempt audits a row at `/activity` of
shape `webhook.<event_kind> · sink={generic|slack|discord|telegram}
· code=<status>`, so a Discord 503 mid-rollout is visible
immediately.

## When each event fires

| Event kind                   | Trigger                                                      | Status values            |
|------------------------------|--------------------------------------------------------------|--------------------------|
| `update_result`              | An apt update window completes (scheduled or run-now).       | `success` / `failed`     |
| `health_probe.transition`    | A health probe flips between green and red. First-sample     | `green` / `red`          |
|                              | spam is suppressed; only real transitions plus first-red.    |                          |
| `backup_job.result`          | A backup job finishes (local fs **or** S3).                  | `success` / `failed`     |
| `agent.disconnect`           | The agent's WS read loop exits and the server removes it     | `disconnected`           |
|                              | from the live agents map.                                    |                          |

The body shape is the same across events: a pre-rendered headline +
status + optional log tail + optional error string + the agent_id
the event came from.

## Configuring sinks

### One config for everything (recommended starting point)

Set the prefix-less env vars on the server. Every event lands in
the same sink:

```bash
# Pick whichever sinks you want. Configured ones fire; unset ones don't.
WEBHOOK_URL=https://...                          # generic; WEBHOOK_FORMAT=json|slack
WEBHOOK_FORMAT=json                              # default
SLACK_WEBHOOK_URL=https://hooks.slack.com/...
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...
TELEGRAM_BOT_TOKEN=...                           # bot token from @BotFather
TELEGRAM_CHAT_ID=...                             # numeric, or @channelname
```

### Per-event override (optional)

Each event reads `<PREFIX>_<SUFFIX>` first, then falls back to the
prefix-less version. Use this when one event matters more than the
others and should route to a louder channel:

| Prefix          | Use case                                                              |
|-----------------|-----------------------------------------------------------------------|
| `UPDATE_`       | Daily apt-upgrade noise → low-priority Slack channel.                 |
| `HEALTH_`       | Probe transitions → urgent Telegram bot that pings your phone.        |
| `BACKUP_`       | Backup result → ops-archive channel, separate from the noisy update.  |
| `DISCONNECT_`   | Agent dropped off → on-call escalation webhook.                       |

For example, to keep the daily noise quiet but escalate disconnects:

```bash
WEBHOOK_URL=https://hooks.slack.com/services/T/B/quiet-channel
DISCONNECT_TELEGRAM_BOT_TOKEN=…
DISCONNECT_TELEGRAM_CHAT_ID=…
```

`update_result`, `health_probe.transition`, `backup_job.result`
land at the Slack URL; `agent.disconnect` lands at the Telegram bot
*instead of* the Slack URL (per-event override wins, not adds).

### What each sink renders

| Sink                       | Body shape                                                         |
|----------------------------|--------------------------------------------------------------------|
| `WEBHOOK_URL` + `format=json`   | JSON: `{event, agent_id, status, log, error, at}` — the structured event. |
| `WEBHOOK_URL` + `format=slack`  | Slack-shaped `{text}` with `:white_check_mark:` / `:x:` icons.     |
| `SLACK_WEBHOOK_URL`        | Same Slack-shaped body, distinct URL.                              |
| `DISCORD_WEBHOOK_URL`      | `{content}` with `✅ / ❌ / ⚠️` icons + Discord-native code blocks.   |
| `TELEGRAM_BOT_TOKEN`+`CHAT_ID`  | Bot API `sendMessage`, `parse_mode=HTML`, log tail in `<pre>`.     |

All chat formats truncate the log to keep the message under each
provider's limit (Discord caps at 2000 chars, Telegram at 4096).
The structured-JSON sink doesn't truncate.

## Worked examples

### Slack

```bash
SLACK_WEBHOOK_URL=https://hooks.slack.com/services/T0/B0/xxxxx
```

Slack post on a probe transition:
```
:x: *sys-manager health probe `#7`* on `amd64-builder-id` → *red*
> error: …
```
```
nginx is dead 4/4 for 60s
```

### Discord

```bash
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/.../...
```

Discord post on backup success:
```
✅ **sys-manager backup `etc-nightly`** on `amd64-builder-id` → **success**
```
```
1234567 bytes → /var/backups/sys-manager/etc-nightly-1730000000.tar.gz
```

### Telegram

```bash
TELEGRAM_BOT_TOKEN=123:ABC...
TELEGRAM_CHAT_ID=-100123456789
```

(Use `@channelname` for public channels, the numeric `chat_id` for
private chats — DM your bot, then GET `/getUpdates` to find the id.)

### Generic JSON receiver

```bash
WEBHOOK_URL=https://n8n.example.com/webhook/xyz
WEBHOOK_FORMAT=json
```

Receives:
```json
{
  "event": "update_result",
  "agent_id": "amd64-builder-id",
  "status": "success",
  "log": "…tail of apt-get output…",
  "error": null,
  "at": 1777310512
}
```

Drop into n8n, Mattermost, your own bot, anything.

## Auditing

Every fire (success or failure) writes one row per sink at
`/activity`:

```
webhook.update_result · sink=slack · code=200
webhook.update_result · sink=discord · code=204
webhook.update_result · sink=telegram · code=200
webhook.health_probe.transition · sink=slack · code=400 body={"error":"channel_not_found"}
```

`code` is the upstream HTTP status. Bodies are captured for non-2xx
responses (truncated to 200 chars) so the operator can see why.
Failures don't retry — a missed delivery shows up as a single
`ok=false` audit row, and the next event will re-attempt all
configured sinks.

## What doesn't fire (yet)

- **Fan-out summary.** Per-host pings already cover it. A roll-up
  webhook for "100-host apt run finished, 96 success, 4 failed" is
  on the roadmap but not shipped.
- **Auth + admin events.** Sign-in, MFA enable/disable, role change,
  token revoke. The audit log at `/activity` covers them. Webhook
  spam for "sppidy logged in" is noise; the operational events
  above are intentionally the only ones that fan out.

If a non-listed event would be useful, file an issue — it's a
~20-line drop-in following the existing pattern.
