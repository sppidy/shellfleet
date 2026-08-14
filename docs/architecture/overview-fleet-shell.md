# Overview Fleet Shell

## Goal

The Overview page should answer common fleet questions at command-line speed without making an operator leave the dashboard or open one SSH session per host. The Fleet Shell is an embedded, keyboard-first view over the same durable fleet, service, Docker, and health snapshots already used by Overview.

It is intentionally not a second PTY implementation. ShellFleet's existing `/terminal` page remains the explicit admin-only surface for arbitrary root commands, and the `terminal` Fleet Shell command opens it.

## Interaction model

- The initial context is the whole fleet and the prompt is `[fleet] $`.
- `use <host>` changes the local context; `use fleet` returns to aggregate output.
- Up and Down recall local command history, Tab completes commands, filters, views, and known hostnames, and Escape clears the draft.
- Quick-command buttons expose useful starting points without requiring operators to memorize the grammar.
- Output is capped per command and the transcript is bounded so a long browser session does not grow the DOM indefinitely.
- Offline hosts remain queryable from their last durable snapshot. Commands that open live host views explain why an offline host cannot be opened.

## Grammar

| Command | Result |
| --- | --- |
| `stats [host]` | Fleet aggregates or a host's CPU/load, memory, disk, services, containers, and probes |
| `hosts [all\|online\|offline\|warn]` | Host table; `warn` includes offline hosts, failed services, and unhealthy/unknown probes |
| `use <host\|fleet>` | Change the default scope for later commands |
| `services [all\|active\|failed\|inactive] [host]` | Filter durable systemd service rows |
| `containers [all\|running\|stopped] [host]` | Filter durable Docker container rows |
| `health [host]` | Health-probe rollups by host |
| `find <text>` | Search host identity/capabilities, service name/description, and container name/image/id |
| `open [host] <view>` | Open a validated live host view or fixed dashboard destination |
| `terminal` | Open the separate admin multi-host root terminal |
| `refresh` | Ask the existing Core Fleet provider for a fresh durable snapshot |
| `history`, `clear`, `help [command]` | Local shell utilities |

Aliases are `top` for `stats`, `ls` for `hosts`, `docker` for `containers`, `probes` for `health`, `select` for `use`, and `?` for `help`.

## Data and trust boundary

The feature adds no command-execution API. Its command engine is a pure client-side projection of `FleetHost`, `CoreAgentSnapshot`, and `HealthSnapshotRow`. Refresh delegates to the existing Core Fleet provider, so SSE reconnect, request coalescing, authentication, and durable-read behavior remain centralized.

Navigation effects are typed. Static destinations come from a fixed route map; host navigation can only use an agent resolved from the current durable fleet and a whitelisted host view. Raw user input is never passed to `router.push`.

## Acceptance criteria

1. Overview displays a usable Fleet Shell above the host table at desktop and mobile widths.
2. Every documented command has deterministic output and useful empty/error states.
3. Fleet aggregates match Overview by using online snapshots; explicit host queries can read offline snapshots.
4. Context, history, autocomplete, quick commands, clear, and refresh work without a page reload.
5. The shell cannot execute OS input; the explicit `terminal` handoff preserves full dashboard-based root access for authorized operators.
6. Parser/engine and interaction tests pass alongside the existing web checks and production build.

## Self-review record

Completed against the implementation:

- Command ambiguity: exact host identity wins, a unique prefix is accepted, ambiguous prefixes list candidates, and invalid filters/views return their specific usage.
- Offline/stale semantics: aggregate statistics use online hosts like Overview; explicit host commands label and read offline durable snapshots; live-view navigation refuses offline hosts with a useful alternative.
- Navigation integrity: effects use discriminated TypeScript unions, static destinations use a fixed map, host ids must resolve from fleet data, views are whitelisted, and query values are encoded with `URLSearchParams`.
- Resource bounds: list/search commands cap output at 40 rows, command history at 100 entries, and the DOM transcript at 30 entries.
- Keyboard/accessibility: the panel is a named region, output is a named live log, the input has an explicit label and combobox semantics, quick actions are buttons, and Tab/Up/Down/Escape behavior is covered by focused tests and browser interaction.
- Responsive rendering: a production browser pass at 1440 px and 390 px kept the panel within the viewport; wide terminal tables scroll inside the output pane. The browser pass produced no console errors.
- Regression checks: all 80 web unit/component tests, strict TypeScript, lint (with only the repository's pre-existing warnings), and the Next production build pass.
- Journey coverage: the durable fleet browser journey now exercises `stats`, host context, and offline-host stats. This environment has no Docker Compose provider, so the full stack journey remains a CI verification rather than a local result.
