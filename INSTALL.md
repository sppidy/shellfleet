# Installing the ShellFleet Agent

The agent ships as a multi-arch `.deb` (amd64 + arm64) published from CI
to an apt repo on GitHub Pages. Each push to `main` produces a new
revision; tags produce signed release builds attached to the GitHub Release.
The `Release` file is GPG-signed; the public key is at
`https://shellfleet-repo.sppidy.in/shellfleet.gpg`.

## Add the apt repo

```bash
# 1. Drop the signing key. apt accepts armored keyrings via signed-by,
#    so no gpg binary needed (works on minimal Debian/Ubuntu).
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://shellfleet-repo.sppidy.in/shellfleet.gpg \
  | sudo tee /etc/apt/keyrings/shellfleet.asc > /dev/null
sudo chmod 0644 /etc/apt/keyrings/shellfleet.asc

# 2. Add the source line, scoped to that keyring.
echo "deb [signed-by=/etc/apt/keyrings/shellfleet.asc] https://shellfleet-repo.sppidy.in stable main" \
  | sudo tee /etc/apt/sources.list.d/shellfleet.list

# 3. Install.
sudo apt update
sudo apt install shellfleet-agent
```

Signing key fingerprint: `9181 1FCB AB45 B996 B40E AD1E C6E2 9AC2
52C7 4AEE`. Verify with `gpg --show-keys /etc/apt/keyrings/shellfleet.asc`.

## Configure

The package installs:

- `/usr/bin/shellfleet-agent` — the binary
- `/lib/systemd/system/shellfleet-agent.service` — the unit
- `/etc/shellfleet/env.example` — annotated environment template

On first install, `/etc/shellfleet/env` is seeded from the example. Edit it
to point at your server, then pair:

```bash
sudo shellfleet-agent --pair
```

It prints an 8-character code. Paste it at
`https://dashboard.example.com/device` (or your own deploy) and approve.
The token is saved at `/etc/shellfleet/agent-token.txt` and survives
upgrades. Then start the service:

```bash
sudo systemctl restart shellfleet-agent
```

## Updating

```bash
sudo apt update && sudo apt install --only-upgrade shellfleet-agent
```

`apt` preserves `/etc/shellfleet/env` and the cached token. The systemd
unit restarts automatically on upgrade (`restart-after-upgrade` in the
package metadata).

## DNS

The apt repo is served from GitHub Pages at `shellfleet-repo.sppidy.in`:

```
shellfleet-repo.sppidy.in.  CNAME  sppidy.github.io.
```

(Or `A` records to GitHub's Pages IPs if your DNS provider can't CNAME at
that level.) The workflow drops a `CNAME` file into the `gh-pages` branch
and a `.nojekyll` marker so apt repo files are served as-is.

## Bootstrap (one-time, repo owner)

The submodule repos (`shellfleet-agent`, `shellfleet-shared`, ...) are private,
so the runner needs a PAT with read access.

1. Create a [fine-grained PAT](https://github.com/settings/personal-access-tokens/new)
   - Resource owner: **sppidy**
   - Repository access: **Only select repositories** → pick `shellfleet`,
     `shellfleet-agent`, `shellfleet-shared`.
   - Permissions → **Repository → Contents → Read-only**.
2. On this repo (`shellfleet`), Settings → Secrets and variables → Actions
   → New repository secret. Name: `SUBMODULES_PAT`. Value: the PAT.
3. Push to `main` once so the workflow runs and creates the `gh-pages` branch.
4. Repo Settings → Pages → Source = **Deploy from a branch** → `gh-pages` /
   `/ (root)`.
5. Subsequent pushes refresh `dists/stable/main/binary-amd64/Packages.gz` and
   the pool. Tagged releases also attach the `.deb` to the GitHub Release.

Signed apt repo (set up in v12):

- The workflow signs `dists/stable/Release` (producing `Release.gpg` +
  `InRelease`) and publishes the public key at `/shellfleet.gpg`.
- Required secrets: `APT_GPG_PRIVATE_KEY` (ASCII-armored secret key).
  Optional: `APT_GPG_PASSPHRASE` if the key is passphrase-protected.
