# Installing the sys-manager agent

The agent ships as an amd64 `.deb` published from CI to an apt repo hosted on
GitHub Pages. Each push to `main` produces a new revision; tags produce signed
release builds attached to the GitHub Release.

## Add the apt repo

```bash
echo "deb [trusted=yes] https://sppidy.github.io/sys-manager stable main" \
  | sudo tee /etc/apt/sources.list.d/sys-manager.list
sudo apt update
sudo apt install sys-manager-agent
```

> The repo is unsigned today (`[trusted=yes]`). To switch to a signed repo,
> set the `APT_GPG_PRIVATE_KEY` and `APT_GPG_PASSPHRASE` secrets on this
> repository — the workflow already wires them up. Then drop the
> `[trusted=yes]` flag and import the public key from
> `https://sppidy.github.io/sys-manager/sys-manager.gpg`.

## Configure

The package installs:

- `/usr/bin/sys-manager-agent` — the binary
- `/lib/systemd/system/sys-manager-agent.service` — the unit
- `/etc/sys-manager/env.example` — annotated environment template

On first install, `/etc/sys-manager/env` is seeded from the example. Edit it
to point at a different server, then:

```bash
sudo systemctl restart sys-manager-agent
sudo journalctl -u sys-manager-agent -f
```

The first start prints a pairing code; approve it at
`https://dashboard.example.com/device` (or your own deploy). The token is cached
at `/etc/sys-manager/agent-token.txt` and survives upgrades.

## Updating

```bash
sudo apt update && sudo apt install --only-upgrade sys-manager-agent
```

`apt` will preserve `/etc/sys-manager/env` and the cached token. The systemd
unit restarts automatically on upgrade (`restart-after-upgrade` in the
package's metadata).

## Bootstrap (one-time, repo owner)

The submodule repos (`sys-mngr-agent`, `sys-mngr-shared`, …) are private,
so the runner needs a Personal Access Token with read access to them.

1. Create a [fine-grained PAT](https://github.com/settings/personal-access-tokens/new)
   - Resource owner: **sppidy**
   - Repository access: **Only select repositories** → pick `sys-manager`,
     `sys-mngr-agent`, `sys-mngr-shared`.
   - Permissions → **Repository → Contents → Read-only**.
2. On this repo (`sys-manager`), Settings → Secrets and variables → Actions
   → New repository secret. Name: `SUBMODULES_PAT`. Value: the PAT.
3. Push to `main` once so the workflow runs and creates the `gh-pages` branch.
4. Repo Settings → Pages → Source = **Deploy from a branch** → `gh-pages` /
   `/ (root)`.
5. Subsequent pushes refresh `dists/stable/main/binary-amd64/Packages.gz` and
   the pool. Tagged releases also attach the `.deb` to the GitHub Release.

Optional, for a signed apt repo:

- Add `APT_GPG_PRIVATE_KEY` (ASCII-armored secret key) and (if encrypted)
  `APT_GPG_PASSPHRASE`. The workflow then signs `dists/stable/Release` and
  publishes the public key at `/sys-manager.gpg`. Drop `[trusted=yes]`
  from the apt source line and `apt-key add` the published key instead.
