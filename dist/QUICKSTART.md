# sys-manager — public testing quickstart

**Audience:** cloud engineer who has Docker + a host with a public DNS name.
**Target time:** 10 minutes from zero to a working dashboard with one paired host.

This guide does not need access to the GitHub source. Everything you need is
on the public Harbor project at `hrbr.sppidy.in/sys-manager`.

---

## 0 · what you'll have at the end

- A web dashboard at `https://your-host.example.com/` signed in via GitHub.
- A `sys-manager-agent` running on at least one Linux host, paired through
  the dashboard, surfacing systemd services + Docker containers + apt updates.

```
        you (browser, GitHub OAuth)
                │
                ▼
   ┌────────────────────────┐    wss://…/ui/ws    ┌──────────────────────┐
   │ web (Next.js, Harbor)  │ ──────────────────► │ server (axum,        │
   │ 3000                   │                     │ Harbor)  8080        │
   └────────────────────────┘                     └──────────┬───────────┘
                                                             │  wss://…/agent/ws
                                                             ▼
                                                  ┌──────────────────────┐
                                                  │ sys-manager-agent    │
                                                  │ (one per managed     │
                                                  │  host, .deb or       │
                                                  │  Docker)             │
                                                  └──────────────────────┘
```

---

## 1 · prerequisites

On a small Linux VM (1 vCPU, 1 GB RAM is plenty):

- Docker 25+ with the `compose` plugin.
- A DNS A/AAAA record pointing `your-host.example.com` at the VM.
- Some way to terminate TLS in front of the stack — Caddy / nginx / Cloudflare
  Tunnel / Traefik. The stack itself listens HTTP on `:3000` (web) and `:8080`
  (server). Public hosting also needs to forward two WebSocket paths:
  `/ui/ws` (browser ⇄ server) and `/agent/ws` (agent ⇄ server). Same TLS
  origin for both is fine.
- A GitHub OAuth app — register one at
  <https://github.com/settings/developers> with:
  - **Homepage URL:** `https://your-host.example.com/`
  - **Authorization callback URL:** `https://your-host.example.com/auth/callback`

  Save the **Client ID** and **Client Secret**.

---

## 2 · drop these two files on the VM

`docker-compose.yml`:

```yaml
services:
  server:
    image: hrbr.sppidy.in/sys-manager/server:v21
    ports:
      - "8080:8080"
    environment:
      - GITHUB_CLIENT_ID=${GITHUB_CLIENT_ID}
      - GITHUB_CLIENT_SECRET=${GITHUB_CLIENT_SECRET}
      - OAUTH_REDIRECT_URL=${OAUTH_REDIRECT_URL}
      - UI_URL=${UI_URL}
      - "JWT_SECRET=${JWT_SECRET:?JWT_SECRET must be set, run openssl rand -hex 32}"
      - "ALLOWED_GITHUB_USERS=${ALLOWED_GITHUB_USERS:?ALLOWED_GITHUB_USERS must be set, comma-separated GitHub logins}"
      - TOKENS_PATH=/data/approved_tokens.json
      # Optional knobs:
      # - AGENT_SECRET=
      # - UPDATE_WEBHOOK_URL=
      # - UPDATE_WEBHOOK_FORMAT=json
      # - WS_ALLOWED_ORIGINS=
      # - BACKUPS_ENABLED=false
    volumes:
      - server_data:/data
    healthcheck:
      test: ["CMD-SHELL", "wget -q -O- http://127.0.0.1:8080/healthz || exit 1"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s
    restart: unless-stopped

  web:
    image: hrbr.sppidy.in/sys-manager/web:v21
    ports:
      - "3000:3000"
    environment:
      - NEXT_PUBLIC_WS_URL=${NEXT_PUBLIC_WS_URL}
    restart: unless-stopped
    depends_on:
      - server

volumes:
  server_data:
```

`.env`:

```bash
# DNS / TLS — replace with your hostname
OAUTH_REDIRECT_URL=https://your-host.example.com/auth/callback
UI_URL=https://your-host.example.com/
NEXT_PUBLIC_WS_URL=wss://your-host.example.com/ui/ws

# GitHub OAuth app from step 1
GITHUB_CLIENT_ID=Iv1.aaaaaaaaaaaaaaaa
GITHUB_CLIENT_SECRET=ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# Generate fresh — anyone who knows this signs valid sessions
JWT_SECRET=$(openssl rand -hex 32)

# Comma-separated GitHub logins permitted to sign in.
# Default refuses to start because publishing one would make every
# fresh deploy publicly accessible to that user.
ALLOWED_GITHUB_USERS=your-github-login
```

**Generate `JWT_SECRET` properly** — the `$(openssl rand -hex 32)` shown above
won't expand inside `.env`; replace with the literal output of running
`openssl rand -hex 32` in your shell.

---

## 3 · TLS in front (one of these)

### Option A — Caddy (simplest, auto Let's Encrypt)

`Caddyfile`:

```caddyfile
your-host.example.com {
    @ws path /ui/ws /agent/ws
    reverse_proxy @ws server:8080
    reverse_proxy /api/* server:8080
    reverse_proxy /auth/* server:8080
    reverse_proxy server:8080 web:3000
}
```

Add Caddy to your `docker-compose.yml`:

```yaml
  caddy:
    image: caddy:2-alpine
    ports: ["80:80", "443:443"]
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
      - caddy_config:/config
    restart: unless-stopped
volumes:
  caddy_data:
  caddy_config:
```

### Option B — Cloudflare Tunnel

`cloudflared tunnel create sys-manager`, then map both `/ui/ws` and
`/agent/ws` to `http://server:8080` and the rest to `http://web:3000`.
Cloudflare Free's 100 MB body limit doesn't matter for this stack.

### Option C — your existing nginx

WebSocket-capable forwarding for `/ui/ws` and `/agent/ws` — check that
the upstream config has:

```nginx
proxy_http_version 1.1;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection "upgrade";
proxy_read_timeout 1d;
```

---

## 4 · bring it up

```bash
docker compose pull         # grabs server / web from Harbor (no login needed)
docker compose up -d
docker compose logs -f server
```

Open `https://your-host.example.com/` → "Continue with GitHub" → you should
land on the Fleet overview with zero agents.

---

## 5 · pair your first agent

The agent is its own daemon installed on each Linux host you want to manage.
Two paths:

### Path A — apt repo (production-grade, signed packages)

```bash
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://sys-mgr-repo.sppidy.in/sys-manager.gpg \
  | sudo tee /etc/apt/keyrings/sys-manager.asc > /dev/null
echo 'deb [signed-by=/etc/apt/keyrings/sys-manager.asc] https://sys-mgr-repo.sppidy.in stable main' \
  | sudo tee /etc/apt/sources.list.d/sys-manager.list
sudo apt-get update
sudo apt-get install -y sys-manager-agent
```

The .deb installs a systemd unit that reads its config from
`/etc/sys-manager/env` (a key-value file, one `KEY=value` per line). Set
the server URL there:

```bash
sudo tee /etc/sys-manager/env > /dev/null <<EOF
SERVER_API_URL=https://your-host.example.com
SERVER_WS_URL=wss://your-host.example.com/agent/ws
EOF

sudo systemctl restart sys-manager-agent
sudo journalctl -u sys-manager-agent -n 20
```

### Path B — Docker (quick test, single host)

```bash
docker run -d \
  --name sys-manager-agent \
  --restart unless-stopped \
  -e SERVER_API_URL=https://your-host.example.com \
  -e SERVER_WS_URL=wss://your-host.example.com/agent/ws \
  -v /var/run/dbus/system_bus_socket:/var/run/dbus/system_bus_socket \
  -v /var/lib/sys-manager:/var/lib/sys-manager \
  hrbr.sppidy.in/sys-manager/agent:v21
docker logs sys-manager-agent | tail -20
```

You only get the host's systemd if you mount the DBus socket; running the
agent on bare metal via Path A is the supported route for fleet operation.

### approve the pairing

The agent prints an 8-character code in its journal/log:

```
[INFO] device pairing required — code ABCD-1234
```

In the dashboard, **Connect agent** → paste the code → approve. The agent
caches its bearer token at `/etc/sys-manager/agent-token.txt` and will
reconnect automatically across reboots.

---

## 6 · what each env var does

| Var                          | Required | What it controls                                                |
|------------------------------|----------|-----------------------------------------------------------------|
| `GITHUB_CLIENT_ID`           | yes      | OAuth app                                                        |
| `GITHUB_CLIENT_SECRET`       | yes      | OAuth app                                                        |
| `OAUTH_REDIRECT_URL`         | yes      | Must match the OAuth app's callback URL exactly                 |
| `UI_URL`                     | yes      | Used in OAuth redirects + the device-pairing page link          |
| `NEXT_PUBLIC_WS_URL`         | yes      | Browser → server WebSocket. Must use `wss://` if site is TLS    |
| `JWT_SECRET`                 | yes      | Signs session cookies. **Stack refuses to start without it.**   |
| `ALLOWED_GITHUB_USERS`       | yes      | Comma list of GitHub logins permitted to sign in                |
| `AGENT_SECRET`               | optional | Bare-token bootstrap path; leave empty to require device-pairing|
| `BACKUPS_ENABLED`            | optional | `true` mounts `/api/backups/*` and the backup scheduler         |
| `WS_ALLOWED_ORIGINS`         | optional | Extra origins for `/ui/ws` (UI_URL is always allowed)           |
| `UPDATE_WEBHOOK_URL`         | optional | Outbound webhook on apt-update window completion                |
| `UPDATE_WEBHOOK_FORMAT`      | optional | `json` (default) or `slack` — Slack/Discord-shaped              |

---

## 7 · pull-only quick test (no compose)

If you just want to see whether the images pull:

```bash
docker pull hrbr.sppidy.in/sys-manager/server:v21
docker pull hrbr.sppidy.in/sys-manager/web:v21
docker pull hrbr.sppidy.in/sys-manager/agent:v21
```

No login required — the Harbor project is public.

---

## 8 · where to get help

- Issues / questions → open one in `sppidy/sys-manager` on GitHub once the
  repo flips public, or reach the maintainer through the public dashboard
  contact email.
- Architecture deep-dive → see the README in the GitHub repo (currently
  private; ask the maintainer for access).

---

## what's in v21

- Phosphor-green terminal redesign of the dashboard (panel/btn/tbl design
  system, JetBrains Mono + Inter, command palette ⌘K, theme + density
  toggles, every per-agent feature panel restyled).
- Next.js `output: 'standalone'` — web image dropped from 1.16 GB → 272 MB.
- `JWT_SECRET` and `ALLOWED_GITHUB_USERS` are now required (no defaults)
  so a fresh deploy can't accidentally start with predictable creds.
- All three published images target `linux/amd64` only at v21. Agent
  multi-arch (amd64 + arm64) ships through the apt repo via the GitHub
  Actions matrix.
