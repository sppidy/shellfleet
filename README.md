# ShellFleet

A self-hosted dashboard for a fleet of Linux hosts. You run one small Rust
agent per host (or per Kubernetes Pod); it connects to an axum/SQLite server
behind a Next.js dashboard. From there you manage systemd services, Docker
containers and Swarm, Kubernetes, apt updates, health probes, backups,
fan-out commands, and interactive shells — for every host you've connected.

**Docs:** <https://shellfleet.sppidy.in/docs.html> ·
**Apt repo:** <https://shellfleet-repo.sppidy.in/> ·
**Images:** <https://ghcr.io/sppidy/shellfleet>

ShellFleet doesn't store metrics or run a collector. It points at your existing
Prometheus and renders the panels you define, queried on demand. The agent does
no background polling and sits around 4 MB of RAM when idle.

## Quick start

1. Bring up the `server` + `web` stack from the published images — the
   [Quickstart](https://shellfleet.sppidy.in/docs.html#quickstart) has the full
   walkthrough and every environment variable.
2. Install the agent on a target host from the signed apt repo:

   ```bash
   sudo install -m 0755 -d /etc/apt/keyrings
   curl -fsSL https://shellfleet-repo.sppidy.in/shellfleet.gpg \
     | sudo tee /etc/apt/keyrings/shellfleet.asc > /dev/null
   echo 'deb [signed-by=/etc/apt/keyrings/shellfleet.asc] https://shellfleet-repo.sppidy.in stable main' \
     | sudo tee /etc/apt/sources.list.d/shellfleet.list
   sudo apt-get update && sudo apt-get install -y shellfleet-agent
   sudo shellfleet-agent-pair          # prints a code, then restarts the service
   ```

3. Sign in with GitHub, open `/device`, and paste the pairing code to approve
   the agent.

### Docker and Swarm (explicit opt-in)

Docker is disabled for a newly installed agent. On a Docker host, an
administrator can enable ShellFleet's local proxy with:

```bash
sudo shellfleet-docker-proxy enable
```

This keeps the agent out of the `docker` group and preserves its direct Docker
socket deny rule. The enabled proxy is root-owned, accepts only the local
`shellfleet` service account, and is confined to forwarding the local Docker
socket. Docker API access is root-equivalent on typical hosts, so enable it
only for a ShellFleet server and administrators you trust. Disable it with
`sudo shellfleet-docker-proxy disable`.

The proxy socket follows `docker.socket`; do not add dependencies from
`docker.socket` to NFS, Tailscale, or `remote-fs.target`. If Docker data lives
on a remote mount, order `docker.service` after that mount instead. Ordering
the socket itself after a remote filesystem can create a boot cycle and make
the agent correctly stop advertising `docker` and `swarm`.

Verify the advertised state after enabling access:

```bash
sudo shellfleet-docker-proxy status
sudo systemctl is-active docker.service shellfleet-docker-proxy.socket shellfleet-agent
```

## Repository layout

The public product is one monorepo. These are ordinary directories, not Git
submodules or gitlinks, so protocol and consumer changes land atomically:

| Path      | Stack        | Purpose                                                                  |
|-----------|--------------|--------------------------------------------------------------------------|
| `web/`    | Next.js 16   | Dashboard SPA with durable fleet reads and interactive control surfaces |
| `server/` | axum + SQLx  | REST/SSE read plane, WS control hub, OAuth, SQLite projections           |
| `agent/`  | Rust + Tokio | Per-host daemon, shipped as a `.deb`                                     |
| `shared/` | Rust         | Wire format, protocol version, and shared contracts                      |
| `cli/`    | Rust         | Trusted native operator cockpit and device authorization client          |

The rest of the top level is build and deploy plumbing: `docker-compose.yml`
(the server + web stack), the `Dockerfile.*` files, `helm/shellfleet-agent/`
(the in-cluster install chart), `metrics.example.yaml`, and
`.github/workflows/agent-deb.yml` (multi-arch `.deb` build + apt repo publish).
The proprietary `shellfleet-ee` repository stays private and is checked out as
a sibling for compatible Enterprise builds; it is never a submodule.

## Documentation

Everything past the quick start lives on the docs site, so it stays in one
place instead of drifting in this file:

- **[Quickstart & environment variables](https://shellfleet.sppidy.in/docs.html#quickstart)** — deploy, reverse-proxy routes, the `.env`, and agent pairing.
- **[Operator CLI](https://shellfleet.sppidy.in/docs.html#cli)** — device authorization without copying browser cookies or dashboard API keys.
- **[Metrics](https://shellfleet.sppidy.in/docs.html#metrics)** — point the dashboard at your Prometheus; YAML panel templates.
- **[Kubernetes](https://shellfleet.sppidy.in/docs.html#kubernetes)** / **[Helm](https://shellfleet.sppidy.in/docs.html#helm)** — the k8s agent flavor and every chart value.
- **[Webhooks](https://shellfleet.sppidy.in/docs.html#webhooks)** and **[Cloudflare](https://shellfleet.sppidy.in/docs.html#cloudflare)** — outbound events and edge setup.
- **[Enterprise Edition](https://shellfleet.sppidy.in/ee-docs.html)** — SSO/SCIM, passkeys, ACLs, multi-tenancy, runbooks, recording, drift, multi-source metrics with custom charts, SLA, cost, AI log analysis, Vault.

## Development

The server and web build with no agent attached (you'll just see "no agents
connected"):

```bash
docker compose up --build server web    # full stack
cd web && npm install && npm run dev     # web dev server → http://localhost:3000
cd agent && cargo build --release        # build the agent (Linux only)
cargo build --release --manifest-path cli/Cargo.toml  # build the operator CLI
```

To test against a real agent locally, uncomment the `agent:` stanza in
`docker-compose.yml` — it mounts the host's DBus socket so the in-container
agent can drive the host's systemd.

## Wire format

`shared/` defines the `Message` enum that travels both ways over the agent and
operator WebSockets, plus `PROTOCOL_VERSION`, which the server checks at the
agent `Register` handshake to reject mismatched agents. Add a field to an existing variant with
`#[serde(default)]` so older agents still deserialize it; a new variant needs an
agent rollout.

Fleet identity, online state, capabilities, and latest snapshots are server-owned
SQLite projections exposed through authenticated REST plus SSE. The dashboard
uses `/ui/ws` only for interactive control and streaming operations. A transient
browser WebSocket failure therefore does not erase the fleet or hide Docker and
Swarm capabilities.

## Security

GitHub OAuth with optional per-user TOTP 2FA; two roles (admin / viewer)
enforced in middleware on `/api/*`; CSRF on every mutating route; a WS origin
allow-list and per-IP rate limiting on the auth surface; a signed apt repo;
TOTP secrets encrypted at rest; and signed commits required on `main`. The
Community Edition is the security floor — the Enterprise Edition adds SSO, custom
RBAC, IP allowlisting, long-retention audit with SIEM streaming, and more.

Report security issues privately: email `sppidytg@gmail.com` with the subject
`[security] ShellFleet: …`. Please don't open a public issue for them.

## Telemetry

The server sends a small anonymous usage report (on by default): a random
per-install id, the version, edition, user and agent **counts**, and enabled
**feature names** — never logins, hostnames, IPs, or agent ids. Turn it off with
`SHELLFLEET_TELEMETRY=off` or the toggle on `/admin`. Reports are HMAC-signed;
set `SHELLFLEET_TELEMETRY_HMAC_KEY` to the same secret configured in the
telemetry Worker before enabling the reporter.

## Contributing

Pull requests are welcome. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) first — it
covers dev setup, the signed-commit requirement on `main`, and the
[CLA](CLA.md) (one click on your first PR via cla-assistant.io).

## License

[AGPL-3.0-or-later](LICENSE) for the Community Edition in this repository. The
closed-source Enterprise Edition sidecar is licensed separately to paying
customers; CE remains fully functional without it. The CLA grants the maintainer
dual-licensing rights so contributor code can flow into both.
