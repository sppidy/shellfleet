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

## Bootstrap GitHub Pages (one-time, repo owner)

1. Push to `main` once so the workflow runs and creates the `gh-pages` branch.
2. In repo settings → Pages, set source to **Deploy from a branch** →
   `gh-pages` / `/ (root)`.
3. Subsequent pushes refresh `dists/stable/main/binary-amd64/Packages.gz` and
   the pool. Tagged releases also attach the `.deb` to the GitHub Release.
