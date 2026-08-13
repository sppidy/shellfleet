# Reliable Agent Credential Lifecycle

**Status:** implemented and self-reviewed  
**Base:** `codex/durable-fleet-read-plane` at `bcaae51`  
**Scope:** Rust HTTP portability, agent credential persistence, device-token
single-use guarantees, and the Helm agent lifecycle

## Problem statement

The durable Fleet read plane makes agent identity and the latest snapshots
survive browser and server reconnects. The Kubernetes packaging does not yet
provide the matching lifecycle guarantee for the agent itself:

- The agent stores its access token, refresh token, and access-token expiry in
  `/var/lib/shellfleet-agent`.
- The Kubernetes image creates `/etc/shellfleet`, the chart mounts a single
  token there, and the chart uses only ephemeral state.
- The Kubernetes image always starts the binary with `--pair`. Explicit pairing
  intentionally ignores any stored credential, so every Pod start creates a
  new device authorization even if usable state exists.
- Device-issued access tokens expire after one hour. Copying only
  `agent-token.txt` into a Secret cannot preserve the rotating credential set.
- Agent credential writes truncate files in place and commit the new access
  token before the new refresh token. A crash between writes can leave an
  unrecoverable mixture after the server has invalidated the prior refresh
  token.
- Server refresh rotation selects a refresh token before opening its delete and
  insert transaction. Concurrent requests can both observe the same supposedly
  single-use token. Approved device codes have the same select/insert/delete
  race.
- Every Rust HTTP client enables `native-tls`, although the agent and server
  already use rustls. A clean local build therefore requires OpenSSL development
  headers and currently fails before project code compiles on the development
  host.

These are one lifecycle problem: an identity should be issued once, committed
safely, accepted exactly once at each rotation boundary, and remain usable
across the restarts that Kubernetes routinely performs.

## Goals and invariants

1. A newly installed Kubernetes agent pairs when it has no credential.
2. A restarted Kubernetes agent reuses its persisted credential and does not
   force a new pairing.
3. A credential rejected with HTTP 401 can enter pairing automatically only in
   the container-specific `pair-if-needed` mode. Network and TLS failures never
   discard or replace a credential.
4. Native package behavior remains explicit: without a credential, the systemd
   service exits and tells the operator to run `shellfleet-agent-pair`.
5. The credential state directory is `/var/lib/shellfleet-agent` in the native
   package, generic container, Kubernetes image, journey test, and Helm chart.
6. The refresh token and expiry are durably replaced before the access token is
   replaced. Each individual replacement is atomic and mode `0600`.
7. At most one caller can consume an approved device code, and at most one
   caller can rotate a refresh token, even under concurrent requests.
8. The default Helm install persists credentials across Pod replacement. An
   operator can explicitly choose ephemeral state for disposable clusters.
9. A bootstrap Secret is copied only into an empty state volume and never
   overwrites credentials that the agent has since rotated.
10. The Rust workspace does not require a system OpenSSL installation merely to
    compile its HTTP clients.

## Non-goals

- Replacing bearer credentials with the future Ed25519 enrollment design.
- Moving Kubernetes credentials into the Kubernetes API or granting the agent
  permission to mutate Secrets.
- Making more than one replica share an identity or WebSocket. The chart remains
  a singleton.
- Changing access-token or refresh-token lifetimes.
- Changing browser authentication, durable Fleet REST/SSE contracts, or the
  shared agent protocol version.
- Choosing the native host agent's local authority. That product contract is
  defined separately in `2026-08-13-native-agent-runtime-modes.md`; credential
  state remains portable across both modes.
- Fixing the existing web lint-warning backlog, which is independent of this
  lifecycle and has no errors in the inherited gate.

## Design

### 1. Portable Rust HTTP TLS

Change the three `reqwest` declarations from `native-tls` to reqwest 0.13's
`rustls-no-provider` backend. The agent and server already install the ring
provider at process startup. The CLI will install the same provider before
constructing an HTTP client. Keep platform certificate verification and
hostname verification enabled; no custom verifier or insecure fallback is
introduced.

Acceptance:

- `cargo tree -i openssl-sys` finds no package.
- The workspace compiles on a host without OpenSSL headers.
- HTTPS behavior continues to use the platform trust store through reqwest.

### 2. Crash-safe agent credential store

Move credential file operations behind a small `CredentialStore` rooted at
`/var/lib/shellfleet-agent`.

The store will:

- reject symlinks and non-regular files while reading;
- reject oversized credential files instead of accepting a truncated prefix;
- write a same-directory temporary file, flush and sync it, set mode `0600`, and
  atomically rename it over the destination;
- sync the containing directory after replacement;
- persist a token triple in recovery-safe order: refresh token, expiry, access
  token last;
- remove stale optional refresh/expiry files when a legacy server returns only
  an access token.

The access-token file is the commit marker. At every crash point before it is
replaced, either the old credential remains recoverable or no initial
credential exists and pairing is required again. Once it is replaced, the
matching refresh material is already durable.

Add a startup mode `--pair-if-needed`:

- `--pair`/`pair`: always initiate a fresh pairing, preserving the supported
  native recovery command.
- `--pair-if-needed`: pair only if no access token exists; if a stored token is
  rejected with HTTP 401 and cannot be refreshed, initiate one fresh pairing.
- no pairing argument: require existing state and retain the native package's
  current failure message.

Only an HTTP 401 is evidence that credentials need replacement. Connection
refusal, timeout, DNS failure, certificate failure, and other handshake errors
exit without starting device authorization.

### 3. Atomic server-side consumption

Replace select-then-delete sequences with `DELETE ... RETURNING` inside the
same transaction that inserts the replacement token:

- `consume_pending_agent_and_insert_token` atomically removes one approved,
  unexpired device request and inserts its access/refresh pair.
- `replace_token_on_refresh` atomically removes one non-revoked, unexpired
  refresh row and inserts its replacement pair.

The losing concurrent caller sees no returned row and receives
`invalid_grant`. There is no successful zero-row delete and no second valid
credential fork.

Tests use a multi-connection file-backed SQLite pool and race multiple
consumers. Exactly one result may succeed, no database-lock errors are accepted,
and exactly one replacement token must remain.

### 4. Kubernetes state and startup contract

The Kubernetes image will:

- create `/var/lib/shellfleet-agent` owned by UID/GID `10001`;
- run as UID/GID `10001`, matching the chart security contexts;
- invoke the agent in `--pair-if-needed` mode rather than unconditional
  `--pair` mode.

The Helm chart will mount `agent-state` at `/var/lib/shellfleet-agent` and add:

```yaml
persistence:
  enabled: true
  existingClaim: ""
  storageClass: ""
  accessModes: [ReadWriteOnce]
  size: 64Mi
  retain: true
```

When enabled without an existing claim, the chart creates a PVC. `retain: true`
adds Helm's keep policy because uninstalling a release should not silently
destroy the identity needed to reconnect it. When disabled, an `emptyDir` is
rendered and the documentation explicitly calls out that Pod replacement will
require pairing again.

`token.existingSecret` remains backward-compatible as a bootstrap input. An
init container mounts that Secret read-only and copies only these recognized
files into an empty state volume:

- `agent-token.txt` (required)
- `agent-refresh.txt` (optional for a legacy shared token, required for a
  device-issued rotating credential)
- `agent-token-expiry.txt` (optional)

It never overwrites an existing access-token file, so a chart upgrade cannot
roll a live agent back to stale Secret material. The main container writes only
the state volume; it never attempts to mutate a Secret mount.

A `values.schema.json` will enforce the singleton and basic persistence shape.
A chart render test will cover default persistence, an existing PVC, explicit
ephemeral mode, bootstrap Secret wiring, and invalid replica counts.

## Upgrade behavior

- Native `.deb`: the credential path and startup/pairing invocation remain
  compatible across managed and restricted runtime modes. Existing credential
  files are read in place and become atomically replaced on their next refresh;
  an explicit package purge removes the complete credential state directory.
- Existing Helm install without `token.existingSecret`: the upgrade creates a
  retained PVC and the next Pod displays one pairing code. Subsequent Pod
  replacements reuse the credential.
- Existing Helm install with a legacy shared-token Secret: the init container
  seeds that token once; it remains valid through the server's explicit legacy
  token path.
- Existing Helm install with a complete device credential Secret: all available
  files seed the PVC once, after which rotations occur only on the PVC.
- Existing Helm install with only an expired device access token: the first
  HTTP 401 in `pair-if-needed` mode starts a new pairing rather than crash
  looping on the stale Secret.

## Verification matrix

| Layer | Required evidence |
| --- | --- |
| Credential store | atomic replacement, `0600`, symlink rejection, oversize rejection, safe optional-file cleanup |
| Startup policy | always/if-needed/required decisions and unauthorized-only fallback |
| Server DB | concurrent device exchange and refresh rotation each yield exactly one winner |
| Rust | workspace tests; agent kube-feature tests; clippy when the component is available |
| Dependency graph | no `openssl-sys` reverse dependency |
| Packaging | existing privilege-boundary script still passes |
| Helm | lint plus render-contract matrix and schema rejection |
| Web | typecheck/lint remain no-error; tests and production build pass after a lock-only patched dependency refresh |

## Self-review and resolved objections

### Why a PVC by default instead of a Kubernetes Secret?

The agent rotates its refresh token every hour. A mounted Secret is immutable
from the agent's point of view, and granting cluster-wide Secret mutation would
materially expand compromise impact. A tiny RWO volume gives the unprivileged
process exactly one writable directory and no Kubernetes write API.

### Could the bootstrap init container roll credentials back?

No. It copies recognized files only when the state volume has no non-empty
access-token file. Once the agent has paired or rotated, PVC state wins forever.

### Does automatic pairing hide outages?

No. It is opt-in through `--pair-if-needed` and triggers on an HTTP 401 only.
Transport and certificate failures retain the credential and fail visibly.
Native systemd startup does not use this flag.

### Is three-file persistence really crash-safe?

It cannot be a cross-file transaction, but the write order makes it
recoverable. New refresh material and expiry land atomically before the access
token commit marker. A restart before the final rename sees the old access
token plus a usable new refresh token and can refresh again; a restart after it
sees the complete new set. Initial pairing before the final rename simply has
no committed access token and pairs again.

### What if the volume has no dynamic provisioner?

The Pod remains Pending instead of silently accepting an ephemeral identity.
The install notes explain `persistence.existingClaim` and the explicit
`persistence.enabled=false` escape hatch. Reliability is the safe default;
ephemerality requires an operator choice.

### Does `DELETE ... RETURNING` require an unsupported SQLite?

SQLite added `RETURNING` in 3.35. The server links bundled SQLite through
`libsqlite3-sys`, so runtime host SQLite versions do not control support.

### Does switching reqwest change certificate validation?

The rustls backend retains platform-root and hostname verification. The change
removes the native OpenSSL build dependency; it does not add a custom trust
store, disable verification, or affect the separately pinned agent mTLS
configuration.

### Could Helm uninstall destroy credentials?

The generated claim is marked for retention by default. The operator must
delete it explicitly after uninstall. This is documented because retained
credentials are sensitive state as well as recovery state.

## Implementation self-review results

The final diff review found and resolved seven issues that were not explicit in
the first draft:

1. **Transport-triggered rotation:** the inherited reconnect path attempted a
   refresh after every WebSocket error. Reactive refresh now requires HTTP 401,
   just like automatic pairing, so TLS, DNS, timeout, and 5xx failures leave
   credentials unchanged.
2. **Cross-purpose exchange:** the agent token endpoint previously accepted an
   approved CLI device code. The transactional consume now requires
   `purpose = 'agent'`, and the handler rejects the wrong purpose before
   revealing pending/approved state.
3. **Secret read permissions:** mode `0400` Secret projections were root-only
   even though the Pod runs non-root. Secret material now renders as `0440`
   with the Pod's `fsGroup`, while copied credentials remain `0600`.
4. **Stale bootstrap optionals:** a legacy Secret without refresh/expiry files
   could leave unrelated optional files on a reused volume. Bootstrap now
   clears missing optional files and commits the required non-empty access
   token last.
5. **Runtime prerequisites:** atomic directory sync needs AppArmor access to the
   state directory itself, and rustls needs a populated system trust store.
   The profile now covers the parent directory, while both container and Debian
   package paths explicitly include `ca-certificates`.
6. **Release tag mismatch:** `Chart.appVersion` used a bare semantic version
   while the image publisher emitted only a `v`-prefixed release tag. Agent and
   chart are aligned at `1.2.0`, the publisher emits both spellings from one
   manifest, and the render test enforces version equality.
7. **Patched web graph:** verification found fixable production advisories in
   the inherited lock. The application manifest ranges already admitted fixed
   releases, so only `package-lock.json` changed. `npm audit` now reports zero
   vulnerabilities, and lint, typecheck, 67 tests, and the production build all
   pass on the refreshed graph.

Additional executable evidence from the completed implementation:

- 64 server tests pass, including 16-way concurrent single-winner races and
  rollback-on-insert-failure cases.
- 42 default agent tests and 46 kube-feature agent tests pass when library and
  binary suites are counted together.
- 34 shared/CLI tests pass.
- Workspace and kube-feature clippy both pass with warnings denied.
- The Helm matrix covers managed/retained PVC, existing PVC, explicit
  `emptyDir`, copy-once bootstrap, mTLS Secret permissions, no-storage-class,
  schema typo rejection, and singleton rejection.
- `cargo tree -i openssl-sys` confirms that no such package remains.

## Definition of done

- Every goal and invariant above is represented by implementation or an
  executable check.
- The isolated worktree is clean except for intentional branch changes.
- The original `durable-fleet-read-plane` worktree and the user's dirty `main`
  checkout are untouched.
- The final diff is reviewed for secret exposure, unsafe fallbacks, stale paths,
  and upgrade regressions before handoff.
