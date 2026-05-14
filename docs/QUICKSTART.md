# ShellFleet — quickstart

**Audience:** cloud engineer who has Docker + a host with a public DNS name.
**Target time:** 10 minutes from zero to a working dashboard with one paired host.

This guide does not need access to the GitHub source. Everything you need is
on the public GHCR packages at `ghcr.io/sppidy/shellfleet` and the public
apt repo at `shellfleet-repo.sppidy.in`.

---

## 0 · what you'll have at the end

- A web dashboard at `https://your-host.example.com/` signed in via GitHub.
- A `shellfleet-agent` running on at least one Linux host, paired through
  the dashboard.
- The dashboard auto-hides tabs the agent can't serve (no docker on the host
  → no Docker tab, host doesn't run k8s → no Kubernetes tab, etc.).

```
        you (browser, GitHub OAuth)
                │
                ▼
   ┌────────────────────────┐    wss://…/ui/ws    ┌──────────────────────┐
   │ web (Next.js, GHCR)  │ ──────────────────► │ server (axum,        │
   │ 3000                   │                     │ GHCR)  8080        │
   └────────────────────────┘                     └──────────┬───────────┘
                                                             │  wss://…/agent/ws
                                                             ▼
                                                  ┌──────────────────────┐
                                                  │ shellfleet-agent     │
                                                  │  • host shape (.deb  │
                                                  │    on any Linux)     │
                                                  │  • k8s shape (Helm   │
                                                  │    Pod or .deb-k8s)  │
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
    image: ghcr.io/sppidy/shellfleet/server:latest
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
      # Optional knobs — see §6 for all of them.
      # - AGENT_SECRET=
      # - WS_ALLOWED_ORIGINS=
      # - BACKUPS_ENABLED=false
      # - METRICS_CONFIG_PATH=/etc/shellfleet/metrics.yaml
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
    image: ghcr.io/sppidy/shellfleet/web:latest
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
JWT_SECRET=replace-me-with-openssl-rand-hex-32

# Comma-separated GitHub logins permitted to sign in.
# Default refuses to start because publishing one would make every
# fresh deploy publicly accessible to that user.
ALLOWED_GITHUB_USERS=your-github-login
```

> **JWT_SECRET** must be a real random value. Generate it with:
> ```bash
> openssl rand -hex 32
> ```
> and paste the output into `.env`.

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

`cloudflared tunnel create shellfleet`, then map both `/ui/ws` and
`/agent/ws` to `http://server:8080` and the rest to `http://web:3000`.
See [`docs/CLOUDFLARE.md`](CLOUDFLARE.md) for the WAF rate-limit rules
recommended in front of the stack.

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
docker compose pull         # grabs server / web from GHCR (no login needed)
docker compose up -d
docker compose logs -f server
```

Open `https://your-host.example.com/` → "Continue with GitHub" → you should
land on the Fleet overview with zero agents.

---

## 5 · pair your first agent

Pick the install shape that matches what the host runs.

### Path A — apt repo, host agent (most common)

For Linux VMs / bare metal that host systemd + (optionally) Docker:

```bash
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://shellfleet-repo.sppidy.in/shellfleet.gpg \
  | sudo tee /etc/apt/keyrings/shellfleet.asc > /dev/null
echo 'deb [signed-by=/etc/apt/keyrings/shellfleet.asc] https://shellfleet-repo.sppidy.in stable main' \
  | sudo tee /etc/apt/sources.list.d/shellfleet.list
sudo apt-get update
sudo apt-get install -y shellfleet-agent
```

The .deb installs a systemd unit that reads its config from
`/etc/shellfleet/env`:

```bash
sudo tee /etc/shellfleet/env > /dev/null <<EOF
SERVER_API_URL=https://your-host.example.com
SERVER_WS_URL=wss://your-host.example.com/agent/ws
EOF

sudo systemctl restart shellfleet-agent
sudo journalctl -u shellfleet-agent -n 20
```

### Path B — apt repo, k8s flavor (host that talks to a kube-apiserver)

If the host has a `KUBECONFIG`, install the k8s-flavor package instead.
It Conflicts/Replaces the standard package, so you can't have both:

```bash
# (same apt repo setup as Path A, then…)
sudo apt-get install -y shellfleet-agent-k8s

# Add the kubeconfig path to the agent's env file:
sudo tee -a /etc/shellfleet/env > /dev/null <<EOF
KUBECONFIG=/etc/rancher/k3s/k3s.yaml
EOF
sudo systemctl restart shellfleet-agent
```

The agent advertises an additional `"k8s"` capability, and the dashboard
reveals the **Kubernetes** top-level tab when you select that agent.

### Path C — Helm, in-cluster Pod

Install the agent as a Pod inside any kube-apiserver-reachable cluster.
Read [`docs/KUBERNETES.md`](KUBERNETES.md) for the full operator
walkthrough and [`docs/HELM.md`](HELM.md) for every chart value.

```bash
helm install sysmgr ./helm/shellfleet-agent \
  --namespace shellfleet --create-namespace \
  --set server.apiUrl=https://your-host.example.com \
  --set server.wsUrl=wss://your-host.example.com/agent/ws \
  --set rbac.exec=true \
  --set rbac.write=true        # opt-in: lets the agent apply / scale / delete
```

Tail the Pod's logs to read the pairing code, same as Path A.

### approve the pairing

The agent prints an 8-character code in its journal / Pod log:

```
[INFO] device pairing required — code ABCD-1234
```

In the dashboard, **Connect agent** → paste the code → approve. The agent
caches its bearer token at `/etc/shellfleet/agent-token.txt` and will
reconnect automatically across reboots.

---

## 6 · what each env var does

### Required server vars

| Var                          | What it controls                                                |
|------------------------------|------------------------------------------------------------------|
| `GITHUB_CLIENT_ID`           | OAuth app                                                        |
| `GITHUB_CLIENT_SECRET`       | OAuth app                                                        |
| `OAUTH_REDIRECT_URL`         | Must match the OAuth app's callback URL exactly                 |
| `UI_URL`                     | Used in OAuth redirects + the device-pairing page link          |
| `NEXT_PUBLIC_WS_URL`         | Browser → server WebSocket. Must use `wss://` if site is TLS    |
| `JWT_SECRET`                 | Signs session cookies. **Stack refuses to start without it.**   |
| `ALLOWED_GITHUB_USERS`       | Comma list of GitHub logins permitted to sign in                |

### Optional server features

| Var                          | What it controls                                                |
|------------------------------|------------------------------------------------------------------|
| `AGENT_SECRET`               | Bare-token bootstrap path; leave empty to require device-pairing|
| `BACKUPS_ENABLED`            | `true` mounts `/api/backups/*` and the backup scheduler         |
| `WS_ALLOWED_ORIGINS`         | Extra origins for `/ui/ws` (UI_URL is always allowed)           |
| `METRICS_CONFIG_PATH`        | YAML at this path enables the Prometheus metrics plugin         |

### Outbound webhook fan-out

Set the **prefix-less** vars below to route every event (apt update result,
health-probe transition, backup result, agent disconnect) through the same
sink. Or pin a per-event prefix to override a single event type.

| Suffix                       | Maps to                                                         |
|------------------------------|------------------------------------------------------------------|
| `WEBHOOK_URL`                | Generic POST. `WEBHOOK_FORMAT=json` (default) or `slack`.       |
| `SLACK_WEBHOOK_URL`          | Slack-shaped text payload                                       |
| `DISCORD_WEBHOOK_URL`        | Discord-native `content` payload                                |
| `TELEGRAM_BOT_TOKEN`         | Bot token; pair with `TELEGRAM_CHAT_ID`                         |
| `TELEGRAM_CHAT_ID`           | Numeric chat / channel id, or `@channelname`                    |

Per-event override prefixes (each takes the same five suffixes):
`UPDATE_*` (apt scheduler), `HEALTH_*` (probe transitions),
`BACKUP_*` (backup job result), `DISCONNECT_*` (agent dropped off).

See [`WEBHOOKS.md`](WEBHOOKS.md) for the full reference: when each
event fires, what each sink renders, the audit-row format, and
worked examples for Slack / Discord / Telegram / generic JSON.

### Per-agent S3 backup destination

When a backup job's `dest` is `s3://bucket/prefix`, the agent uploads via
the AWS SDK — no `awscli` install needed. Standard AWS env vars work, plus
`AWS_ENDPOINT_URL` for any S3-compatible backend (MinIO, Cloudflare R2,
Backblaze B2, Wasabi, …). Drop the relevant block into `/etc/shellfleet/env`
on each agent. Recipes for each backend are in
[`agent/debian/env.example`](https://github.com/sppidy/shellfleet-agent/blob/main/debian/env.example)
or your installed copy at `/etc/shellfleet/env.example`.

---

## 7 · pull-only quick test (no compose)

If you just want to see whether the images pull:

```bash
docker pull ghcr.io/sppidy/shellfleet/server:latest
docker pull ghcr.io/sppidy/shellfleet/web:latest
docker pull ghcr.io/sppidy/shellfleet/agent:latest
docker pull ghcr.io/sppidy/shellfleet/agent-k8s:latest
```

No login required — the GHCR project is public.

---

## 8 · tabs that auto-hide

The agent advertises a capability set on connect (`systemd`, `docker`,
`swarm`, `k8s`). The dashboard hides tabs for capabilities the agent
didn't claim, so a k8s-only Pod agent shows just `overview · k8s · metrics
· health · config` and a no-docker host hides the Docker top-level tab.
This is automatic — nothing to configure.

---

## 9 · where to get help

- Issues / questions → open one in `sppidy/shellfleet` on GitHub.
- Architecture deep-dive → [README.md](../README.md) at the repo root.
- Topic-specific docs:
  - [`KUBERNETES.md`](KUBERNETES.md) — k8s install paths, RBAC posture
  - [`HELM.md`](HELM.md) — chart reference + every value
  - [`METRICS.md`](METRICS.md) — Prometheus plugin schema + worked example
  - [`WEBHOOKS.md`](WEBHOOKS.md) — outbound notification fan-out reference
  - [`CLOUDFLARE.md`](CLOUDFLARE.md) — WAF rate-limit rules in front of the stack
