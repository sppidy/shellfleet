# ShellFleet monorepo boundary

## Goal

`sppidy/shellfleet` is the canonical public repository for every component
that must evolve with the ShellFleet wire protocol: server, agent, shared
types, web dashboard, operator CLI, images, packaging, Helm, tests, and public
engineering documentation. A protocol change and all of its consumers must be
reviewable, testable, and releasable from one commit.

## Repository boundary

The public tree owns these directories:

- `server/`, `agent/`, `shared/`, and `cli/` in one root Cargo workspace;
- `web/` with its npm lockfile;
- root Dockerfiles and Compose configuration;
- `helm/`, `tests/`, `docs/`, and root GitHub automation.

Proprietary and operational repositories remain private siblings. In
particular, `shellfleet-ee` is not vendored, copied, or added as a submodule.
Its build receives the public `shared/` directory as an explicit BuildKit
context, and its CI checks out that directory from this monorepo. The landing,
licensing, and telemetry services also remain outside the public source tree.

## Invariants

1. The public repository has no `.gitmodules`, gitlinks, or nested Git
   repositories in its committed tree.
2. Rust packages use the root `Cargo.toml` and the single root `Cargo.lock`.
3. Component dependencies use in-tree paths; no build or CI job clones the
   retired standalone public component repositories.
4. Active workflows and dependency-update configuration live under the root
   `.github/` directory.
5. Public images build from the repository root. EE receives only the public
   context it needs, never the inverse.
6. No EE source, proprietary license marker, deployment override, or private
   documentation is committed to the public tree.
7. Pull requests run formatting, linting, unit tests, web checks, Helm checks,
   and the browser/server/agent journey against the same commit.

`scripts/check-monorepo.sh` enforces the structural subset of these rules in
CI so a submodule, nested lockfile, stranded workflow, legacy component URL, or
private EE path cannot quietly return.

## Release model

- Server, web, and agent container images build from root Dockerfiles.
- Standard and Kubernetes agent packages build from the root Cargo lockfile.
- CLI release assets build from the root workspace and upload to the product
  release that triggered the workflow.
- Dependency automation updates Rust at `/`, npm at `/web`, and root workflow
  actions.

The legacy standalone public repositories may be archived only after external
consumers have moved to `sppidy/shellfleet`; archival is a repository-setting
cutover, not part of source migration.

## Completion review

The migration is complete when the invariant script passes, Cargo resolves no
legacy Git source, all root CI jobs pass, EE CI consumes `shared/` from the
public monorepo, release workflows resolve artifacts from root paths, and a
public-tree scan confirms the private boundary.
