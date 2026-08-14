# ShellFleet Product Reset Design

**Status:** Product direction approved under delegated CEO authority; written design pending review
**Date:** 2026-07-15
**Decision:** Replace the feature-accumulator product with a focused Linux fleet operations cockpit, delivered through a controlled core reset.

## 1. Executive decision

ShellFleet will be a self-hosted incident and operations cockpit for people responsible for 3-100 Linux servers, especially hosts running Docker or Docker Swarm.

Its promise is:

> See what is unhealthy, understand why, take a safe action, and verify recovery from one place.

ShellFleet will not position itself as a replacement for monitoring systems, container platforms, endpoint management, configuration management, or privileged-access products. It will integrate with those tools where useful and own the fleet-wide operational workflow between detection and recovery.

This reset keeps proven Rust and security work, replaces weak product boundaries, and removes unfinished features from the shipped surface until the core journey is dependable.

## 2. Evidence behind the reset

The current product has no narrow job-to-be-done. Its public promise spans host administration, monitoring, Docker, Swarm, Kubernetes, updates, backup, remote shells, configuration editing, enterprise identity, approvals, drift, cost, SLA, and AI analysis.

The implementation reflects that scope:

- about 42,000 lines across the principal server, agent, shared-protocol, and web surfaces;
- approximately 150 wire-message variants and 93 server route registrations;
- more than 30 dashboard routes;
- four public submodule repositories plus a separate CLI repository and private EE sidecar;
- separate server and Next.js containers for a dashboard that is fundamentally a client application;
- a browser experience that depends on one long-lived WebSocket even for the initial fleet list;
- an installation flow involving GitHub OAuth configuration, several required environment values, separate browser and agent endpoints, and manual certificate provisioning;
- green unit, lint, and build checks but no browser-plus-agent end-to-end release gate.

The live failure mode demonstrated the architectural problem: the agents and server were healthy, while a browser WebSocket failure left the entire dashboard blank and reported the fleet offline. A presentation transport must not be the source of truth for durable fleet state.

## 3. Customer and job-to-be-done

### Primary customer

A technically capable homelab operator, small infrastructure team, managed-service operator, or internal IT generalist responsible for 3-100 Linux machines. They run a mix of bare-metal servers, virtual machines, Docker hosts, and small Docker Swarm clusters. They want to self-host the control plane and do not want to operate MySQL, Redis, Kafka, or a Kubernetes management stack merely to manage a small fleet.

### Core situation

Something is unhealthy or needs routine intervention. The operator needs to answer:

1. Which host, service, or workload is affected?
2. Is the information current?
3. What changed or failed?
4. What safe action can restore service?
5. Did the action succeed and did health recover?

### Success statement

An operator can move from alert to verified recovery without switching among separate host, container, log, and shell dashboards for ordinary incidents.

## 4. Product principles

1. **Fleet truth is durable.** Inventory, last-seen state, health, capabilities, operations, and results live on the server. A browser reconnect never erases them.
2. **Read paths are boring HTTP.** Initial screens and refreshes use authenticated REST. Live enhancement must never be required to render useful state.
3. **Actions are operations, not socket messages.** Every action gets an ID, actor, target, state, timestamps, result, timeout, and audit record.
4. **One public origin.** Browser, API, CLI, enrollment, and agent connectivity use one canonical HTTPS origin. Advanced network modes may be added later.
5. **Secure defaults must also be operable.** Security mechanisms that require manual certificate generation, ownership repair, or undocumented root steps are incomplete product work.
6. **Capabilities are explicit.** Hosts declare what they can observe and operate. The UI only exposes supported actions.
7. **No feature without a journey test.** A feature is not shipped until a released server and released agent complete its real workflow in CI.
8. **Integration over reinvention.** ShellFleet does not rebuild Prometheus, SSH access platforms, Kubernetes dashboards, or configuration-management engines.

## 5. Shipped product surface

### 5.1 Fleet

- Current online, degraded, and offline hosts.
- Compact health summary across CPU pressure, memory pressure, disk, failed services, pending updates, and container state.
- Docker Swarm topology when reported by a manager, including managers, workers, nodes, services, replicas, and failed tasks.
- Active incidents ordered by operational severity and recency.
- Search and filters by host, role, label, capability, service, and container.

### 5.2 Host

Each host has five core sections:

- **Overview:** identity, OS, uptime, resource pressure, connectivity, capabilities, and recent activity.
- **Services:** systemd unit state, recent journal excerpts, and controlled start/stop/restart actions.
- **Containers:** Docker containers, health, image, ports, resource snapshot, logs, and controlled lifecycle actions.
- **Updates:** pending package count, refresh status, and an explicit update operation when enabled locally.
- **Logs:** bounded journal and container-log queries with time, priority, and source filters.

An emergency terminal is not part of the initial core. It returns later as an explicit admin-only capability after ordinary operations are reliable and audited.

### 5.3 Activity

- Durable operation timeline.
- Actor, target, action, state, duration, and result.
- Clear distinction among requested, dispatched, running, succeeded, failed, timed out, and cancelled.
- Filters and export suitable for troubleshooting, not a compliance product.

### 5.4 Settings

- Server health and version.
- Admin and viewer users.
- Agent enrollment, revocation, labels, and capability profiles.
- Public URL and optional OIDC configuration.
- Retention and diagnostics.

## 6. Features removed from the default product

The following surfaces are frozen and removed from navigation and marketing until separately redesigned and proven:

- Kubernetes management;
- backup and restore;
- arbitrary configuration-file editing;
- fan-out commands;
- multi-host and general-purpose terminals;
- AI log analysis;
- custom metrics builders;
- runbooks and approvals;
- break-glass workflows;
- drift, SLA, and cost modules;
- SCIM, complex IAM policies, and long-retention compliance features;
- enterprise tenancy and licensing expansion.

The code is not deleted at the start. It is isolated behind a legacy build boundary so the core can be rebuilt without continuously repairing unrelated surfaces. Features return only through their own product spec, ownership boundary, and end-to-end tests.

## 7. Target architecture

### 7.1 Repository and release boundary

The public server, agent, protocol, CLI, and web code become one monorepo with one release train. The private EE repository remains separate and frozen.

The existing submodules are absorbed after the core API boundary is established. History is preserved, but protocol changes and their consumers become atomic commits. The release version applies to the complete product, not independent floating component versions.

### 7.2 Server

The server is one Rust service containing:

- REST API;
- agent connection gateway;
- SSE event stream;
- on-demand stream relay for logs and future terminal sessions;
- SQLite persistence;
- embedded static web assets.

The production artifact is one container with one persistent volume. The standalone Next.js server is retired. The React UI is built as static assets using Vite and embedded into the Rust server image.

SQLite remains the default database for the target scale. Database migrations are transactional and backed up before destructive changes. No external cache or message broker is introduced.

### 7.3 Browser transport

The browser uses:

- REST for sessions, fleet state, host detail, containers, services, logs, and operations;
- SSE for invalidation events, operation progress, and fleet-status changes;
- WebSocket only for genuinely bidirectional byte streams such as a future terminal.

SSE disconnection shows stale-state age and reconnect status but does not clear displayed data. A page refresh always reconstructs the product from REST and SQLite.

Transport liveness is established by actual REST, SSE, and WebSocket outcomes.
Browser connectivity hints such as `navigator.onLine` may accelerate recovery,
but must never suppress a connection attempt; mobile radios, VPNs, and captive
portals can report stale connectivity state while the application origin is
already reachable.

### 7.4 Agent transport and identity

The agent makes one outbound connection to the canonical public HTTPS origin. No inbound host port is required.

Enrollment works as follows:

1. The agent generates an Ed25519 identity key locally.
2. The agent submits its public key and minimal host identity to an enrollment request endpoint.
3. The user approves the displayed device code in the dashboard.
4. The server binds that public key to a durable agent record.
5. On connection, the server issues a single-use challenge.
6. The agent signs the canonical challenge payload.
7. The server verifies the signature and issues a short-lived connection token.
8. The agent upgrades an outbound WebSocket using that token.

Challenges are short-lived and single-use. Connection tokens are short-lived, audience-bound, and revocable. The agent private key never leaves the host. This replaces long-lived shared tokens and manual mTLS certificate provisioning while retaining cryptographic device identity and one-port deployment.

### 7.5 Agent capability model

The package installs a dedicated `shellfleet` service account and a small, versioned privileged helper. Capability profiles are:

- **Observe:** inventory, health, service state, container state when locally readable, and bounded logs.
- **Operate:** typed service, package, Docker, and Swarm operations through the privileged helper.
- **Emergency shell:** disabled by default and excluded from the initial release.

The privileged helper accepts typed operations, not arbitrary shell strings. Docker access remains explicit because Docker control is effectively root-equivalent, but enabling it is a package-supported command and is reflected as a visible capability—not an environment-file edit.

### 7.6 Durable operation model

All mutations use the same model:

```text
requested -> dispatched -> running -> succeeded
                                |-> failed
                                |-> timed_out
                                |-> cancelled
```

The server creates the operation before dispatch. Agent messages carry the operation ID. Results are idempotent, persisted, and safe to resend after reconnect. Browser and CLI clients observe the same operation resource.

No HTTP request waits for a long-running action to finish. Creating an operation returns `202 Accepted` and its resource URL. REST and SSE expose progress.

### 7.7 Domain and API boundaries

The core domain is split into:

- `identity`: users, sessions, agent identities, enrollment, revocation;
- `inventory`: hosts, labels, capabilities, versions, last-seen state;
- `health`: compact current snapshots and incidents;
- `services`: systemd reads and typed operations;
- `containers`: Docker and Swarm reads and typed operations;
- `operations`: durable lifecycle, dispatch, retry, result, and audit;
- `events`: SSE invalidation and progress events;
- `streams`: bounded log streams and future terminals.

The shared Rust protocol is split along the same boundaries. It retains typed serialization and an explicit protocol version, but no single 1,500-line enum owns unrelated functionality.

## 8. Authentication and authorization

GitHub OAuth is no longer required to start the product.

On first boot, the server writes a one-time bootstrap URL containing a high-entropy secret to its logs. The URL expires after 30 minutes or first use and creates the first local admin with a username and password. Passwords are hashed with Argon2id. Before leaving setup, the admin enrolls TOTP and receives one-time recovery codes; subsequent local-admin login requires the password and a TOTP or recovery code. Generic OIDC is optional. GitHub becomes one identity provider rather than a hard dependency.

The initial role model remains deliberately small:

- **Admin:** manage users, agents, capabilities, and operations.
- **Viewer:** read fleet and activity state.

Host-side capability profiles are independent of user roles. A server admin cannot perform an operation that the target agent has not locally enabled.

Browser mutations use same-origin sessions and CSRF protection. CLI authentication uses a browser-approved device flow and the same REST API as the UI.

## 9. CLI

The first CLI is operational, not a second product:

```text
shellfleetctl login <url>
shellfleetctl doctor
shellfleetctl hosts
shellfleetctl host <id>
shellfleetctl operations list
shellfleetctl operations get <id>
shellfleetctl service restart <host> <unit>
shellfleetctl container restart <host> <container>
```

It consumes the public REST API and operation model. The current approver-key and encrypted root-cockpit work is frozen with the enterprise access features. ShellFleet will not compete with Teleport or SSH for general infrastructure access.

## 10. Installation experience

### Server

The supported quick start requires:

- one container image;
- one persistent volume;
- one published HTTPS origin;
- no mandatory external database, cache, GitHub application, manually generated CA, or second agent port.

The container generates internal secrets on first boot and stores them in the volume. `shellfleetctl doctor` and the Settings page validate public URL, TLS, storage, migrations, and event connectivity.

### Agent

The supported Ubuntu/Debian journey is:

```text
install package
sudo shellfleet-agent enroll https://fleet.example.com
approve displayed code in browser
```

Enrollment writes identity and server configuration atomically, validates permissions, enables the service, connects, and confirms that the server sees the host. Manual editing of `/etc/shellfleet/env` is not part of the normal path.

## 11. Failure behavior

- The dashboard distinguishes server unavailable, session expired, live updates disconnected, and all agents offline.
- Previously fetched state remains visible with its age when SSE disconnects.
- Agent disconnects mark hosts offline after a defined heartbeat threshold; they do not delete inventory.
- Server restart reconstructs fleet state from SQLite and marks agents stale until they reconnect.
- Operation dispatch tolerates reconnects and duplicate results.
- Unsupported capabilities are omitted rather than failing after a click.
- Every error shown to the user has a stable code, a concise explanation, and a diagnostic correlation ID.

## 12. Testing and release gates

Unit and component tests remain, but release authority moves to journey tests using the actual built artifacts.

### Mandatory core journey

CI must:

1. build the release server, web assets, agent, and CLI;
2. start the server from an empty volume;
3. bootstrap an admin through the browser;
4. enroll a protocol-test agent through the real device flow;
5. verify the fleet and host pages through Playwright;
6. verify disconnect and reconnect behavior;
7. expose fixture systemd and Docker state through the agent integration harness;
8. create a service or container operation;
9. verify operation progress, final result, and audit record in both API and browser;
10. authenticate the CLI and run `doctor` and read commands;
11. restart the server and prove durable state is reconstructed;
12. upgrade from the previous supported database and agent protocol versions.

No container image, Debian package, or release tag publishes unless this journey passes.

### Acceptance thresholds

- Fresh-server to first visible host in under ten minutes using only documented commands.
- Fleet page useful after refresh even when live events are unavailable.
- Offline detection within 45 seconds and reconnect reflected within 10 seconds.
- Core read pages load within two seconds at 100 simulated hosts on the reference deployment.
- Every mutation produces exactly one durable operation and audit record.
- Zero manual database edits, cookie extraction, permission repair, or certificate copying in the supported journey.

## 13. Migration strategy

This is a strangler reset inside the existing project.

### Stage A: freeze and isolate

- Freeze non-core features and EE.
- Remove non-core routes from the default navigation and marketing.
- Establish the new core API and operation model alongside legacy routes.
- Add the journey-test harness before migrating behavior.

### Stage B: durable read plane

- Persist agent inventory and snapshots.
- Deliver Fleet and Host read APIs.
- Build the new static UI shell on REST and SSE.
- Keep the legacy UI available only behind a development flag.

### Stage C: enrollment and identity

- Implement key-based enrollment and one-origin connectivity.
- Ship the new agent package flow.
- Provide a controlled migration from existing agent tokens and mTLS identities.

### Stage D: operations

- Introduce durable operations.
- Migrate services, Docker, Swarm, updates, and logs one vertical slice at a time.
- Delete the matching legacy message paths only after their journey tests pass.

### Stage E: CLI and repository consolidation

- Rebuild the CLI on the public REST API.
- Absorb public submodules into the monorepo while preserving history.
- Move to one version and release manifest.

### Stage F: remove legacy product

- Remove the old browser-wide WebSocket provider.
- Remove the Next.js runtime container.
- Delete legacy routes and protocol variants no longer used.
- Publish the reduced product promise and supported upgrade guide.

## 14. Explicit non-goals

The reset does not attempt to:

- replace Prometheus or provide long-term metrics storage;
- replace SSH, Teleport, or a bastion;
- become a general configuration-management engine;
- become a Kubernetes distribution or full Kubernetes dashboard;
- manage employee endpoints or mobile devices;
- provide enterprise compliance, SIEM, cost allocation, or multi-tenancy;
- support arbitrary remote root commands in the core release;
- preserve backward compatibility with every unfinished screen.

## 15. Strategic alternatives considered

### Continue patching

Rejected. It is the fastest route to another local fix but preserves the same product boundaries, submodule friction, ephemeral UI state, and untestable release journey.

### Full greenfield rewrite

Rejected. It discards useful Rust agent, package, protocol, security, and server work and creates a long period with no releasable product.

### Controlled core reset

Selected. It keeps proven internals, creates new boundaries beside the legacy system, migrates complete vertical slices, and deletes old behavior only after real journeys pass.

## 16. Completion definition

The product reset is complete only when:

- a fresh self-hosted install completes the documented server and agent journey without manual repair;
- the default UI contains only the four approved surfaces;
- fleet state survives browser event failure and server restart;
- systemd, Docker, and Swarm reads match real hosts;
- supported mutations use durable operations and are audited;
- browser, agent, server, and CLI journey tests gate every release;
- the old Next.js runtime, browser-wide WebSocket dependency, and obsolete legacy routes are removed;
- public documentation and marketing describe the focused product honestly;
- the live reference deployment passes the same journey using released artifacts.

Until those conditions are proven, the reset remains incomplete regardless of unit-test, CI, or build status.
