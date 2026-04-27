# Contributing to sys-manager

Thanks for considering a contribution! This document covers everything
you need to know before opening a pull request.

## TL;DR

1. Open an issue first if the change is non-trivial.
2. Fork, branch off `main`, keep PRs focused.
3. **Sign the [CLA](CLA.md)** — the bot will comment on your first PR.
4. Sign your commits (`git commit -S`). Branch protection on `main`
   rejects unsigned commits.
5. Follow the existing code style; CI will tell you if you didn't.

## Repository layout

`sys-manager` is a super-repo of git submodules:

| Submodule | Repo | Purpose |
|-----------|------|---------|
| `agent/`  | [`sys-mngr-agent`](https://github.com/sppidy/sys-mngr-agent)   | Rust daemon installed on managed hosts |
| `server/` | [`sys-mngr-server`](https://github.com/sppidy/sys-mngr-server) | Rust/axum control-plane API + WS hub |
| `web/`    | [`sys-mngr-web`](https://github.com/sppidy/sys-mngr-web)       | Next.js dashboard (standalone build) |
| `shared/` | [`sys-mngr-shared`](https://github.com/sppidy/sys-mngr-shared) | Common protocol/types crate |

PRs should target the submodule repo for the component you're changing.
Once merged, a maintainer will bump the pointer in the super-repo.

## Development setup

See [`README.md`](README.md) for the dev quickstart and
[`dist/QUICKSTART.md`](dist/QUICKSTART.md) for the
container-based deployment walkthrough.

Minimum tooling:

- Rust stable (toolchain pinned in `rust-toolchain.toml` where present)
- Node 22+ and npm (for `web/`)
- Docker + Buildx (only if you're touching deployment)

## Contributor License Agreement (CLA)

Before we can merge your contribution, you must agree to the
[**Individual Contributor License Agreement**](CLA.md).

### Why a CLA?

sys-manager is published under the **AGPL-3.0** license. The project
also has a planned commercial enterprise edition that includes
features such as SSO, advanced RBAC, multi-tenancy, secret-manager
integration, and long-retention audit log.

The CLA grants the maintainer the right to:

1. Distribute your contribution under AGPL-3.0 (the public license),
   **and**
2. Re-license it under a commercial license for inclusion in the
   enterprise edition.

Without this dual-licensing right, every contributor would have a veto
on the enterprise edition, which is not workable. The CLA does **not**
take away your copyright — you retain all rights to your own code and
can use it elsewhere however you like.

### How to sign

When you open your first pull request, [CLA Assistant](https://cla-assistant.io/)
will leave a comment with a one-click link. Click through, log in with
GitHub, agree once, and you're set for all future contributions.
Subsequent PRs will not re-prompt.

If you are contributing on behalf of a company, please ask your
employer to sign the corporate CLA instead — open an issue tagged
`cla:corporate` and we'll send the form.

## Commit hygiene

- **Sign every commit** (`-S`). Branch protection on `main` requires
  signed commits across all five repos (super + agent + server + web +
  shared). Set up signing once via `git config --global user.signingkey
  ...` and `commit.gpgsign true`; see GitHub's [signing-commits docs](https://docs.github.com/en/authentication/managing-commit-signature-verification).
- **Conventional-style summaries** are appreciated but not strict:
  `feat(server): add WebAuthn enrollment endpoint`,
  `fix(agent): handle reconnect after suspend`.
- **No `Co-Authored-By: Claude`** trailers. AI-assisted code is fine,
  but you are the author for licensing purposes — you sign the CLA, not
  the model.
- Squash before opening the PR if the branch is messy; a maintainer may
  squash on merge regardless.

## What gets accepted

**Definitely:**

- Bug fixes with a clear repro
- Performance improvements with before/after numbers
- New agent capabilities (file ops, service control, metrics, etc.)
- Web UI polish, accessibility fixes, dark-mode regressions
- Documentation, examples, and quickstarts

**Discuss in an issue first:**

- New top-level features (sidebar pages, dashboard widgets)
- Anything that adds a background loop on the agent — see the idle-cost
  budget below
- Database schema changes
- New dependencies, especially heavy ones

**Probably not:**

- Light-mode-only redesigns (dark-mode is the durable preference)
- Features that duplicate functionality available in the planned
  enterprise edition (see [README's roadmap section](README.md))
- Telemetry / phone-home of any kind in CE

## Idle-cost budget (agent)

The agent runs on small hardware. New features must respect:

- **~4 MB RSS** idle target (currently ~3.5 MB)
- **<1% average CPU** when no UI is connected
- **No outbound traffic** when idle except the WS keepalive

If your patch adds a background loop, call it out in the PR
description and include a measurement of its impact (RSS at idle,
CPU% during a representative window).

## Reporting bugs

Use [GitHub issues](https://github.com/sppidy/sys-manager/issues) with:

- sys-manager version (or commit SHA)
- Agent OS + arch (`uname -a`)
- Server logs around the failure (`docker logs sys-manager-server-1`)
- Browser console for UI bugs
- Steps to reproduce

## Reporting security issues

**Do not open a public issue for security bugs.** Email the maintainer
at `sppidytg@gmail.com` with the subject `[security] sys-manager: ...`
and we'll coordinate a fix and disclosure timeline.

## License

By contributing, you agree that your contributions will be licensed
under the [AGPL-3.0](LICENSE), and that the maintainer may additionally
distribute them under a commercial license as described in the
[CLA](CLA.md).
