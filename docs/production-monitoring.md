# Production synthetic monitoring

`scripts/production-synthetic.mjs` exercises the public dashboard path instead
of probing the containers directly. Each run creates a 60-second JWT in memory
from the control plane's existing signing secret and checks:

- authenticated `GET /api/core/v1/fleet`, including an online-host floor and a
  fresh system snapshot;
- authenticated `/api/core/v1/events`, waiting for a real fleet event through
  the complete SSE stream;
- authenticated `/ui/ws`, including the Cloudflare/ingress upgrade and the
  initial live agent list;
- passwordless passkey challenge creation, including the configured RP id.

The token and signing secret are never printed or stored by the probe. systemd
delivers the signing secret through its protected credentials directory rather
than an environment variable. The service runs as an isolated dynamic user,
records only health state, alerts once when a failure starts, re-alerts hourly
while it persists, and sends a recovery notice. Telegram is used when its two
optional systemd credentials are installed.

## Install on the control-plane host

Install the script and units, then provide a root-readable environment file:

```sh
sudo install -D -m 0755 scripts/production-synthetic.mjs \
  /usr/local/libexec/shellfleet-production-synthetic.mjs
sudo install -D -m 0644 deploy/systemd/shellfleet-production-synthetic.service \
  /etc/systemd/system/shellfleet-production-synthetic.service
sudo install -D -m 0644 deploy/systemd/shellfleet-production-synthetic.timer \
  /etc/systemd/system/shellfleet-production-synthetic.timer
sudo install -d -m 0755 /etc/shellfleet
sudoedit /etc/shellfleet/synthetic.env
sudoedit /etc/shellfleet/synthetic.jwt-secret
```

Put only the raw JWT signing-secret value in `synthetic.jwt-secret`, without a
variable name or quotes, and set its mode to `0600`. Required environment
settings are `SHELLFLEET_SYNTHETIC_BASE_URL` and
`SHELLFLEET_SYNTHETIC_LOGIN`. `UI_URL` and the first entry in
`ALLOWED_GITHUB_USERS` are accepted as deployment-compatible fallbacks.

```dotenv
SHELLFLEET_SYNTHETIC_BASE_URL=https://fleet.example.com/
SHELLFLEET_SYNTHETIC_LOGIN=monitor
SHELLFLEET_SYNTHETIC_ROLE=viewer
SHELLFLEET_SYNTHETIC_MIN_ONLINE=1
SHELLFLEET_SYNTHETIC_MAX_SNAPSHOT_AGE_SECS=60
```

The selected login must already be allowed and present in the server database.
Use a viewer where possible. If Enterprise ACLs intentionally hide every agent
from that viewer, the WebSocket transport is still validated; its visible-agent
count is reported but is not used as the fleet availability floor.

```sh
sudo chmod 0600 /etc/shellfleet/synthetic.env \
  /etc/shellfleet/synthetic.jwt-secret
sudo systemctl daemon-reload
sudo systemctl enable --now shellfleet-production-synthetic.timer
sudo systemctl start shellfleet-production-synthetic.service
sudo systemctl status shellfleet-production-synthetic.service
sudo journalctl -u shellfleet-production-synthetic.service -n 20 --no-pager
```

For Telegram alerts, put the raw bot token and chat ID in separate root-only
files, then install the supplied credential drop-in:

```sh
sudo install -D -m 0644 \
  deploy/systemd/shellfleet-production-synthetic-telegram.conf \
  /etc/systemd/system/shellfleet-production-synthetic.service.d/telegram.conf
```

The drop-in contains:

```ini
# /etc/systemd/system/shellfleet-production-synthetic.service.d/telegram.conf
[Service]
LoadCredential=telegram-bot-token:/etc/shellfleet/synthetic.telegram-bot-token
LoadCredential=telegram-chat-id:/etc/shellfleet/synthetic.telegram-chat-id
```

Direct `JWT_SECRET`, `TELEGRAM_BOT_TOKEN`, and `TELEGRAM_CHAT_ID` environment
variables remain supported for one-shot/manual runs, but the shipped systemd
unit intentionally keeps secrets out of its ordinary environment.

Tune `SHELLFLEET_SYNTHETIC_TIMEOUT_MS` (default `25000`) or
`SHELLFLEET_SYNTHETIC_ALERT_COOLDOWN_SECS` (default `3600`) only when the edge
has known longer latency. A failed run exits non-zero, so the service is also
compatible with systemd or external failure collectors.
