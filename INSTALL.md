# Installing the sys-manager agent

The agent ships as a multi-arch `.deb` (amd64 + arm64) published from CI
to an apt repo hosted on GitHub Pages. Each push to `main` produces a new
revision; tags produce signed release builds attached to the GitHub Release.
The `Release` file is GPG-signed; the public key is published at
`https://sys-mgr-repo.sppidy.in/sys-manager.gpg`.

## Add the apt repo

```bash
# 1. Import the repo's signing key into a dedicated keyring.
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://sys-mgr-repo.sppidy.in/sys-manager.gpg \
  | sudo gpg --dearmor -o /etc/apt/keyrings/sys-manager.gpg
sudo chmod 0644 /etc/apt/keyrings/sys-manager.gpg

# 2. Add the source line, scoped to that keyring.
echo "deb [signed-by=/etc/apt/keyrings/sys-manager.gpg] https://sys-mgr-repo.sppidy.in stable main" \
  | sudo tee /etc/apt/sources.list.d/sys-manager.list

# 3. Install.
sudo apt update
sudo apt install sys-manager-agent
```

The signing key fingerprint is `9181 1FCB AB45 B996 B40E AD1E C6E2 9AC2
52C7 4AEE` — verify it after import with
`gpg --show-keys /etc/apt/keyrings/sys-manager.gpg` if you want to be
extra careful.

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

## DNS

The apt repo is served from GitHub Pages at the custom domain
`sys-mgr-repo.sppidy.in`. You need a DNS record:

```
sys-mgr-repo.sppidy.in.  CNAME  sppidy.github.io.
```

(Or `A` records to GitHub's Pages IPs if your DNS provider can't CNAME at
that level.) The workflow drops a `CNAME` file into the `gh-pages` branch
and a `.nojekyll` marker so the apt repo files are served as-is.

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

Signed apt repo (set up in v12):

- The workflow signs `dists/stable/Release` (producing `Release.gpg` +
  `InRelease`) and publishes the public key at `/sys-manager.gpg`.
- Required secrets: `APT_GPG_PRIVATE_KEY` (ASCII-armored secret key).
  Optional: `APT_GPG_PASSPHRASE` if the key is passphrase-protected.
