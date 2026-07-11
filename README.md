# ShellFleet Operator Cockpit

The trusted native CLI/TUI for [ShellFleet](https://github.com/sppidy/shellfleet).
It provides host-identity pinning, exact transaction review, signed single-use
root approvals, encrypted root commands, and encrypted root PTYs through an
untrusted ShellFleet relay.

## Community Edition

The complete trusted-root client is available to Community Edition users. It
does not require an Enterprise license or EE sidecar. Enterprise deployments
may add optional quorum, approver-group, requester-separation, and centralized
history capabilities without changing the client trust boundary.

## Build

```bash
cargo build --release
```

The binary is `target/release/shellfleet`.

## Bootstrap

Create an encrypted local approver key:

```bash
shellfleet keygen
```

The command prints the public key and the local-root enrollment command. Run
that command on each managed host, then compare the host's locally displayed
fingerprint before pinning it in the TUI:

```bash
sudo shellfleet-approval-gate --print-host-fingerprint
sudo shellfleet-approval-gate --enroll-approver operator <BASE64_PUBLIC_KEY>
```

Authorize the cockpit without copying a browser session token:

```bash
shellfleet login https://dashboard.example.com
shellfleet
```

`login` displays an eight-character code. Open the displayed `/device?cli=1`
page in an already-authenticated dashboard, approve the code, and the CLI saves
an 8-hour session in `~/.config/shellfleet/cli-session.json` with mode `0600`.
That token is accepted only by the operator WebSocket, not by browser or API
routes. `shellfleet logout` removes it locally. Environment overrides remain
available for non-interactive automation.

The browser, CE server, EE sidecar, and network agent never receive the
approver private key or root-session plaintext key.

## License

AGPL-3.0-or-later. See `LICENSE`.
