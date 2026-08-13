# ShellFleet Fleet Cockpit

The native fleet application for [ShellFleet](https://github.com/sppidy/shellfleet).
It provides a compact overview of hosts, services, containers, Swarm workloads,
and fleet activity from the durable read APIs. Interactive host access stays in
the web dashboard, where it can share the dashboard's authorization and audit
boundary.

## Build

```bash
cargo build --release
```

The binary is `target/release/shellfleet`.

## Sign in

Authorize the cockpit without copying a browser session token:

```bash
shellfleet login https://dashboard.example.com
shellfleet
```

`login` displays an eight-character code. Open the displayed `/device?cli=1`
page in an already-authenticated dashboard, approve the code, and the CLI saves
an 8-hour session in `~/.config/shellfleet/cli-session.json` with mode `0600`.
That token is accepted only by the read-only fleet and event APIs, not by
browser or administrative API routes. `shellfleet logout` removes it locally.
`SHELLFLEET_URL` and `SHELLFLEET_AUTH_TOKEN` remain available for
non-interactive launches.

## License

AGPL-3.0-or-later. See `LICENSE`.
