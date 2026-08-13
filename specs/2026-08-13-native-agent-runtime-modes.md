# Native agent runtime modes

**Status:** implemented and self-reviewed  
**Base:** `codex/durable-fleet-read-plane` (`bcaae51`)  
**Scope:** native Debian package runtime authority, upgrade behavior, and
operator-facing mode selection

## Problem

ShellFleet is sold by its own UI as a way to manage systemd, apt, Docker,
backups, configuration files, and interactive host terminals without falling
back to SSH. The current package runs the network agent as an unprivileged
`shellfleet` user under a read-only systemd/AppArmor sandbox, but most of those
structured actions still execute directly in that process. As a result, the
server can authenticate, authorize, and audit an action that the host then
rejects for lack of local authority.

The browser root terminal was later routed through a root broker. That restores
an escape hatch, but it does not restore the product: an administrator should
not need to open a terminal and reproduce an action that already has a typed
dashboard control.

This is also a misleading security boundary. A server-authorized browser
terminal already reaches a root shell through the local broker. Preventing the
same administrator from restarting a service through the structured UI does
not materially reduce the authority of a compromised trusted control plane; it
only makes normal operation less reliable and less auditable.

## Product decision

The native package supports two explicit modes:

- **managed**: the default whenever no explicit mode has been recorded. The agent runs as root and the
  host-management surfaces are expected to work. Server authentication, admin
  RBAC, action classification, approval rules, input validation, and audit
  logging remain in force. This mode is intentionally root-equivalent.
- **restricted**: the agent runs as the `shellfleet` service user with the
  existing capability-free systemd sandbox and AppArmor profile. Read surfaces,
  Kubernetes operations allowed by kube credentials, and explicitly delegated
  local sockets remain available; host mutations may be unavailable.

The choice is made with `shellfleet-agent-mode managed|restricted|status`, not
by hand-editing the packaged unit.

## Requirements

1. A native package install or upgrade without a recorded choice selects
   managed mode so systemd, apt,
   configuration, backup/restore, Docker/Swarm, and terminal controls have the
   local authority implied by the dashboard.
2. Managed is also the binary fallback when `SHELLFLEET_AGENT_MODE` is absent.
   Managed mode still requires UID 0; restricted containers set their mode
   explicitly and remain non-root.
3. Restricted mode retains the current `shellfleet` identity, empty capability
   sets, read-only filesystem sandbox, direct Docker-socket denial, and
   AppArmor confinement.
4. Switching modes stops the service, changes only ShellFleet-owned state and
   its systemd drop-in, reloads systemd, and restores the prior active/inactive
   state.
5. Credentials remain mode-portable. Before entering restricted mode, the
   helper gives the `shellfleet` account ownership of the bounded credential
   state directory and enforces `0700`/`0600` permissions.
6. Upgrading a package that predates runtime modes records managed mode. Once
   an operator explicitly selects restricted mode, subsequent upgrades preserve
   that recorded choice.
7. The root broker accepts connections from either the restricted
   `shellfleet` UID or UID 0. Allowing UID 0 adds no local privilege because a
   root process can already control the broker and its files.
8. The Kubernetes container and local development image stay non-root by
   default. This decision changes the native host package only.
9. Agent credential paths remain excluded from the dashboard config-file API
   in both modes.

## Managed-mode sandbox

Managed mode keeps restrictions that do not contradict host administration:

- `NoNewPrivileges=true`;
- kernel modules, kernel logs, control groups, clock, and hostname protected;
- namespace creation and realtime scheduling restricted;
- native syscall architecture and a private `0077` umask.

It deliberately does **not** use an empty capability bounding set,
`ProtectSystem=strict`, `ProtectHome=yes`, `PrivateDevices=true`, or the
restricted agent AppArmor profile. Those controls prevent package installation,
configuration writes, backup/restore, host inspection, or Docker management.

## Docker behavior

In managed mode the root agent can use the normal local Docker socket. The
existing `shellfleet-docker-proxy` remains useful and supported for restricted
mode. Its opt-in state is preserved across a mode switch; managed mode can also
continue using it when `DOCKER_HOST` is configured.

## Upgrade and rollback

- Fresh install or upgrade without a marker: write mode marker `managed`; no
  restrictive drop-in.
- Upgrade with a marker: reconcile the drop-in to the recorded choice.
- `shellfleet-agent-mode managed`: remove only the mode drop-in and write the
  marker.
- `shellfleet-agent-mode restricted`: install the packaged drop-in, normalize
  credential ownership/permissions, and write the marker.
- Package purge removes the generated marker and mode drop-in. Package removal
  without purge preserves the operator's selection.

Rollback is `sudo shellfleet-agent-mode restricted`; it does not rotate or
discard credentials.

## Security statement

Managed mode must be described honestly: anyone who can issue an authorized
interactive or mutation request through the trusted ShellFleet control plane
can affect the host as root. That is the intended authority of a host-management
product. Operators who do not accept that trust relationship should use
restricted mode, narrower server RBAC, or avoid enrolling the host.

This change does not weaken agent authentication, credential storage, CSRF,
browser/session RBAC, request classification, approval workflows, audit logs,
path validation, argument validation, protocol limits, or transport security.

## Verification

- Pure tests cover mode parsing and the root/non-root runtime contract.
- Broker tests cover both permitted UIDs and reject an unrelated UID.
- Package boundary tests assert that managed is the fresh-install unit, the
  restricted template retains every confinement control, the mode helper owns
  the transition, and upgrades default legacy installs to managed.
- Shell syntax checks cover maintainer scripts and helpers.
- Workspace tests and clippy run with both the default and Kubernetes feature
  sets.
- Existing credential, server transaction, Helm render, and web checks remain
  green.

## Self-review checklist

- Does a fresh package actually expose the authority its UI advertises?
- Is managed consistently the fallback for both native packaging and the
  binary, while containers opt into restricted mode?
- Does an upgrade preserve an explicit choice while applying the documented
  managed default when no choice exists?
- Can switching to restricted strand root-owned credentials?
- Does switching modes unexpectedly start a service that was stopped?
- Does the root broker still serve terminals in both modes?
- Are Kubernetes and development containers still non-root?
- Are credential files still unreachable through config read/write requests?

## Implementation self-review results

The implementation review found and resolved four concrete defects beyond the
initial design:

1. **Wrong expiry filename:** the first mode-transition helper normalized
   `agent-expiry.txt`, while the credential store uses
   `agent-token-expiry.txt`. Both the helper and upgrade path now use the exact
   three-file credential contract, with an executable package assertion.
2. **Pair-helper quoting:** an apostrophe inside the helper's single-quoted
   child script broke POSIX shell parsing. All shipped maintainer scripts and
   helpers now pass both `sh -n` and ShellCheck.
3. **Weak negative assertions:** inherited package checks used top-level
   `! grep` under `set -e`, which ShellCheck correctly flagged as an errexit
   exception. Every negative boundary assertion now has an explicit failing
   branch and diagnostic.
4. **Managed transition ordering:** the mode marker is committed before the
   restrictive drop-in is removed. A crash between those steps therefore
   leaves the safer restricted runtime, while the recorded operator choice can
   be reconciled on the next invocation.

Completed verification evidence:

- 42 default agent tests and 46 Kubernetes-feature agent tests pass, including
  runtime parsing, root/non-root enforcement, broker UID selection, credential
  safety, and the existing operation-input policies.
- Root-container smoke tests confirm that both an unset mode and explicit
  managed mode pass the runtime guard and reach normal credential startup,
  while explicit restricted mode rejects UID 0.
- Workspace and Kubernetes-feature clippy pass with warnings denied.
- ShellCheck and POSIX syntax checks pass for every package helper,
  maintainer script, package-boundary test, and Helm render test.
- Pull-request CI now enforces those package checks and uses the committed Rust
  lockfile for every clippy and test invocation.
- A built Debian package contains the managed unit, restricted template, and
  executable selector at their documented paths and modes.
- Throwaway Debian package tests verify fresh-install `managed`, legacy-upgrade
  `managed`, exact credential ownership normalization, active-service
  preservation, and inactive-service preservation.
- The generic and Kubernetes images remain UID `10001` and explicitly set
  restricted mode.
