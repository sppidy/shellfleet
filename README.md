# sys-manager

A self-hosted, terminal-flavoured fleet dashboard. One Rust agent per host, one
axum/SQLite server, one Next.js dashboard. Manages systemd services, Docker
containers + swarm, apt updates, health probes, backups, fan-out commands, and
remote shells across every host you connect.

Live: https://dashboard.example.com/  ·  Apt repo: https://sys-mgr-repo.sppidy.in/

> Built because Prometheus node-exporter + Grafana + Datadog all trade quality
> data for real CPU + RAM cost on every managed host. The agent's design rule
> is **be cheap when nobody's looking**: ~4 MB RSS at idle, no background
> polling for stats / containers / images / networks / volumes / stacks. The
> dashboard issuing a request is the only thing that triggers those code
> paths. See [LOG.md](./LOG.md) → "Idle cost philosophy".

## Architecture

```
                                          (Cloudflare → nginx)
   ┌──────────────────────────┐                │
   │   Next.js dashboard      │ ──── wss /ui/ws──┐
   │  (web, port 3000)        │                │ │
   └──────────────────────────┘                │ │
                                               ▼ │
   ┌──────────────────────────┐    ┌──────────────────────┐    docker compose
   │   axum server            │ ⇄  │  /data/sys-manager.db│    on the host VM
   │  (server, port 8080)     │    │  (SQLite, WAL)       │
   └────────────┬─────────────┘
                │   wss /agent/ws
                ▼
   ┌─────────────────────────────────────────────────────────────┐
   │  sys-manager-agent on each host (.deb via apt repo)         │
   │  • systemd service control + system stats                   │
   │  • interactive PTY (host shell + per-container exec)        │
   │  • config file read/write                                   │
   │  • docker container/image/network/volume/stack/swarm        │
   │  • streaming docker logs + journalctl                       │
   │  • apt update/upgrade, scheduled update windows             │
   │  • health probes (http/tcp/exec) — opt-in only              │
   │  • backups (tar/gzip → local or s3) — gated by env          │
   └─────────────────────────────────────────────────────────────┘
```

## Repository layout

This superproject pins four submodules — each is its own private GitHub repo:

| Path     | Repo                              | Stack       | Purpose                                                  |
|----------|-----------------------------------|-------------|----------------------------------------------------------|
| `web/`   | `sppidy/sys-mngr-web`             | Next.js 16  | Dashboard SPA — sidebar, per-agent tabs, command palette |
| `server/`| `sppidy/sys-mngr-server`          | axum + SQLx | WS hub, REST API, GitHub OAuth, SQLite store at `/data`  |
| `agent/` | `sppidy/sys-mngr-agent`           | Rust + Tokio| Per-host daemon. Shipped as a `.deb`                     |
| `shared/`| `sppidy/sys-mngr-shared`          | Rust crate  | Wire-format `Message` enum + `PROTOCOL_VERSION`          |

Top-level files in this superproject:

| File                       | Purpose                                                            |
|----------------------------|--------------------------------------------------------------------|
| `docker-compose.yml`       | server + web stack; agent stanza is commented for local-only tests |
| `Dockerfile.server`        | Multi-stage Rust build → distroless runtime                        |
| `Dockerfile.web`           | Next.js standalone build → node:slim runtime                       |
| `Dockerfile.agent`         | Local-test agent image (referenced by the commented compose stanza)|
| `.github/workflows/`       | `agent-deb.yml` — multi-arch (amd64 + arm64) .deb build + apt repo |
| `LOG.md`                   | Running session log — every shipped version, idle cost notes, follow-ups |

## Deploy

The live deploy lives on a docker host reachable over Tailscale. Submodule
commits land first, then the superproject pointer is bumped, then the host
pulls + rebuilds.

```bash
# 1. Commit + push inside the affected submodule(s)
cd web && git commit -am "…" && git push

# 2. Bump the superproject pointer
cd .. && git add web && git commit -m "Bump web: …" && git push

# 3. Pull + rebuild on the docker host
ssh ubuntu@<docker-host> "cd /srv/sys-manager && \
  git pull --recurse-submodules && \
  docker compose up -d --build server web"
```

The `.env` on the docker host carries:

| Var                                              | Required | Notes                                                                        |
|--------------------------------------------------|----------|------------------------------------------------------------------------------|
| `JWT_SECRET`                                     | yes      | Signs session cookies                                                        |
| `GITHUB_CLIENT_ID` / `GITHUB_CLIENT_SECRET`      | yes      | OAuth app                                                                    |
| `ALLOWED_GITHUB_USERS`                           | yes      | Comma list of GitHub logins permitted to sign in                             |
| `AGENT_SECRET`                                   | optional | Bare-token bootstrap path; intentionally empty in the live deploy            |
| `BACKUPS_ENABLED`                                | optional | `true` to mount `/api/backups/*` and run the backup scheduler                |
| `WS_ALLOWED_ORIGINS`                             | optional | Extra origins allowed on `/ui/ws` (UI_URL is always allowed)                 |
| `UPDATE_WEBHOOK_URL` / `UPDATE_WEBHOOK_FORMAT`   | optional | Outbound webhook on `update_window.result`. Format: `json` (default) or `slack`|

## Connecting an agent

1. **Install the .deb** on the target host. The apt repo is signed; use the
   `signed-by=` source line:

   ```bash
   sudo install -m 0755 -d /etc/apt/keyrings
   curl -fsSL https://sys-mgr-repo.sppidy.in/sys-manager.gpg \
     | sudo tee /etc/apt/keyrings/sys-manager.asc > /dev/null
   echo 'deb [signed-by=/etc/apt/keyrings/sys-manager.asc] https://sys-mgr-repo.sppidy.in stable main' \
     | sudo tee /etc/apt/sources.list.d/sys-manager.list
   sudo apt-get update && sudo apt-get install -y sys-manager-agent
   ```

   GPG fingerprint: `9181 1FCB AB45 B996 B40E AD1E C6E2 9AC2 52C7 4AEE`.

2. **Pair it.** The agent prints a one-time pairing code on first boot:

   ```bash
   sudo journalctl -u sys-manager-agent -n 20
   ```

   Open `/device` on the dashboard, sign in with GitHub (must be in the
   `ALLOWED_GITHUB_USERS` allowlist), paste the 8-character code, and
   approve. The agent stores the issued bearer token at
   `/etc/sys-manager/agent-token.txt` and reconnects automatically.

3. **Roll updates** by triggering the CI build and `apt-get install -y` on
   each host:

   ```bash
   gh workflow run agent-deb.yml --ref main
   for h in <host-1> <host-2> …; do
     ssh -n root@$h "rm -rf /var/lib/apt/lists/sys-mgr-repo.sppidy.in_* 2>/dev/null; \
                     apt-get update -qq && \
                     DEBIAN_FRONTEND=noninteractive apt-get install -y sys-manager-agent && \
                     systemctl is-active sys-manager-agent"
   done
   ```

## Local development

The web and server build with no agent attached; you'll just see "no agents
connected".

```bash
# Bring up server + web with hot-reload disabled
docker compose up --build server web

# OR run the web dev server against a local server
cd web && npm install && npm run dev   # http://localhost:3000

# Build the agent natively (Linux only)
cd agent && cargo build --release
```

For a full local end-to-end test (server + web + a containerized agent),
uncomment the `agent:` stanza in `docker-compose.yml`. That stanza mounts
the host's DBus socket so the in-container agent can drive the host's
systemd.

## Wire format

`shared/` defines the `Message` enum that travels in both directions over
the WebSocket. The crate's `PROTOCOL_VERSION` is incremented every time the
enum changes shape so the server can refuse mismatched agents at the
`Register` handshake.

When adding a new field to an existing variant, mark it `#[serde(default)]`
so older agents continue to deserialize the response. New variants always
require an agent rollout.

## Security

- **Auth.** GitHub OAuth → 24h session cookie (`SameSite=Lax`, `Secure`).
- **2FA (TOTP).** Optional per-user. Enroll at `/security`. RFC 6238
  with SHA-1, 6 digits, 30 s period, ±1 step skew. Recovery codes are
  generated at enrollment time, hashed (SHA-256) at rest, and burned on
  use.
- **RBAC.** Two roles, **admin** (read + write) and **viewer**
  (read-only). The first allowlisted GitHub login that signs in is
  promoted to admin; everyone else defaults to viewer. Override via
  `BOOTSTRAP_ADMIN`. Enforced in a tower middleware on `/api/*`:
  mutating methods require admin, all other methods require an
  authenticated, MFA-verified session. Admins manage roles and seats
  at `/admin`.
- **Seat cap.** Community Edition is capped at **3 active seats**.
  New sign-ins past the cap are rejected at the OAuth callback;
  existing users keep their access. Remove a seat at `/admin` to free
  up room. EE lifts this with a license-keyed cap.
- **Audit log.** All sign-ins, MFA events, and meaningful agent /
  scheduler actions land in the `audit` table. Visible at `/activity`.
  **7-day local retention** — an hourly task drops rows past the
  window. EE will offer long retention + SIEM export.
- **CSRF.** Double-submit cookie + `X-CSRF` header on every mutating
  `/api/*` route. The web client routes mutations through
  `web/src/lib/api.ts::apiFetch`.
- **WS Origin allow-list.** `/ui/ws` upgrades reject unknown origins;
  `UI_URL` is always allowed, `WS_ALLOWED_ORIGINS` adds extras.
- **Apt repo.** ed25519-signed `Release` + `InRelease`. The clearsigned
  `InRelease` is verified by `apt` against the public key piped into
  `/etc/apt/keyrings/sys-manager.asc`.
- **OAuth state CSRF.** Random per-flow state in an HttpOnly cookie,
  verified on `/auth/callback`. Defeats the attack where a victim is
  lured into hitting the callback with the attacker's authorization
  code.
- **At-rest encryption.** TOTP secrets and recovery-code hashes are
  encrypted with AES-256-GCM. The key is `SHA-256("sys-manager-aead-v1"
  || JWT_SECRET)`, so a DB-only backup leak (without env vars) yields
  no useful material. Format on disk: `v1:<base64-no-pad nonce>.<base64-no-pad ct>`.
- **Brute-force defence.** Per-login MFA throttle locks after 10 bad
  TOTP attempts for 15 minutes. Per-actor `/api/device/approve`
  throttle on the same shape.
- **Constant-time recovery-code compare.** SHA-256 hash equality runs
  through `subtle::ConstantTimeEq` so the loop time doesn't leak which
  position matched.
- **WebSocket RBAC.** The `/ui/ws` upgrade pins the user's login at
  connect time and re-resolves the role from the DB on every mutating
  `SendToAgent`. Without this, the HTTP RBAC middleware would have
  been bypassable by sending agent-control messages over the WS plane.
- **JWT_SECRET fail-loud.** The server refuses to start if
  `JWT_SECRET` is unset, shorter than 32 chars, or the historical
  placeholder value.
- **Defence-in-depth headers.** HSTS (`max-age=31536000;
  includeSubDomains`), `X-Content-Type-Options: nosniff`,
  `X-Frame-Options: DENY`, `Referrer-Policy: strict-origin-when-cross-origin`,
  and a tight `Permissions-Policy`.
- **Branch protection.** All five repos require signed commits on
  `main`; force-push and deletion are disabled.
- **Per-real-IP rate limiting.** Token bucket on the
  anonymous-attacker surface (`/auth/*`, `/api/me`,
  `/api/auth/mfa/verify`) keyed off `CF-Connecting-IP`. 30 burst, 30
  req/min steady. Defence-in-depth on top of Cloudflare's edge rate
  limiter — see [`docs/CLOUDFLARE.md`](docs/CLOUDFLARE.md) for the
  edge rules.

### Roadmap — Enterprise Edition

The CE feature set above is the **safety floor**: every operator gets
2FA, basic RBAC, and a short local audit log. The Enterprise Edition
ships as a separate sidecar binary that registers with CE over an
extension API and adds the **scale ceiling**: SSO (SAML, OIDC), SCIM
provisioning, custom RBAC with per-resource permissions, multi-tenant
organizations, secrets-manager integration (Vault, SOPS, AWS SM),
long-retention audit log with SIEM export, and a support SLA. CE
remains fully functional without EE; EE without CE is meaningless.

## Idle cost

Continuous loops on the agent — full inventory:

1. WebSocket heartbeat — 25 s ping (well under 1 ms each).
2. Health probes the operator configured. Zero by default.
3. Apt-update window scheduler — 60 s tick that does DateTime math; only
   spawns `apt-get upgrade` when a configured cron expression matches.
   Defaults to nothing.
4. Backup scheduler — same shape, gated behind `BACKUPS_ENABLED`.

That's it. There is no continuous polling for stats, container lists,
image lists, network/volume/stack lists, or prune previews. When no UI is
connected, the agent's average CPU is ≈ 0%. Idle RSS measured at ~4 MB.

Cost banners on every UI surface that triggers a non-trivial agent call
(Stats, Prune, Exec) document the cost model in-place so the operator
never has to guess what's running in the background.

## Useful commands

```bash
# Tail the live server
ssh ubuntu@<docker-host> \
  "docker compose -f /srv/sys-manager/docker-compose.yml logs --tail=200 -f server"

# Inspect approved agent tokens
ssh ubuntu@<docker-host> \
  "docker exec sys-manager-server-1 sqlite3 /data/sys-manager.db \
    'SELECT hostname, datetime(created_at,\"unixepoch\"), datetime(last_seen,\"unixepoch\") FROM tokens'"

# Build + roll a new agent .deb
gh workflow run agent-deb.yml --ref main
```

## License

Private. All rights reserved.
