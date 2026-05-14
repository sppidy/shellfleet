# Metrics plugin (CE)

ShellFleet doesn't store time-series — that's Prometheus's job.
The metrics plugin turns the dashboard into a thin renderer for
**your existing Prometheus**: configure named panel templates in
YAML, pick which ones show up on the per-agent Metrics tab, and the
server queries Prometheus on demand with the agent's instance label
substituted in.

This is the answer to "I want persistent process / CPU / memory /
disk metrics" without ShellFleet turning into a TSDB.

## CE / EE split

| | CE | EE (sidecar, future) |
|---|---|---|
| Prometheus | ✓ one URL | ✓ multiple, federated, per-tenant |
| Datadog / New Relic / Grafana Cloud | — | ✓ |
| Free-form PromQL from the client | never | never (same posture, EE just has more sources) |
| Panel templates | ✓ unlimited | ✓ + per-tenant overrides |

## Configuration

The plugin reads YAML from the path in `METRICS_CONFIG_PATH` (default
`/etc/shellfleet/metrics.yaml`). On a fresh deploy, copy
[`../metrics.example.yaml`](../metrics.example.yaml) to that path,
edit the Prometheus URL + auth, and restart the server. If the file
is missing or invalid the plugin stays disabled — the dashboard
just hides the Metrics tab.

### YAML schema (high level)

```yaml
prometheus:
  url: https://prometheus.your-domain.example/api/v1
  bearer_token: ${PROMETHEUS_BEARER}    # optional
  basic_auth:                            # optional
    username: shellfleet
    password: ${PROMETHEUS_PASSWORD}
  tls:
    insecure_skip_verify: false          # default false
  timeout_secs: 10                       # default 10

agent_instance_map:                      # optional
  host-a-id: host-a.internal:9100

panels:
  - id: cpu_percent
    title: CPU %
    description: optional one-liner
    unit: percent | bytes | bytes_per_sec | cpu_seconds_per_sec | raw
    query: |
      100 - (avg by (instance) (rate(node_cpu_seconds_total{mode="idle",instance="{instance}"}[1m])) * 100)
```

### Substitutions

The query is a template, not raw user input from the dashboard. Three
placeholders get replaced server-side before the request goes
upstream:

| Placeholder    | Becomes |
|----------------|---------|
| `{agent_id}`   | the raw ShellFleet agent id, e.g. `host-a-id` |
| `{instance}`   | resolved from `agent_instance_map` if set, else `agent_id` with the trailing `-id` stripped |
| `{hostname}`   | alias for `{instance}` |

So `node_cpu_seconds_total{instance="{instance}"}` running for the
`host-a-id` agent becomes
`node_cpu_seconds_total{instance="host-a"}` — assuming your
node_exporter labels the host as `host-a`. If your Prometheus
labels the same host as `host-a.internal:9100`, add the
mapping:

```yaml
agent_instance_map:
  host-a-id: host-a.internal:9100
```

### Env-var expansion

Any `${VAR}` reference is expanded from the server process's env
at config load. Keep the bearer token / basic-auth password in your
`.env` (or a real secrets manager) and reference them from the YAML:

```yaml
prometheus:
  bearer_token: ${PROMETHEUS_BEARER}
```

## Worked example: "process monitoring as a plugin"

The most common reason to ship metrics integration is "I want to
see what process is hogging CPU, with history." Combine
[`process_exporter`](https://github.com/ncabatoff/process-exporter)
on each host with two panel templates and you have it.

### 1. Run process_exporter on each host

```bash
sudo apt install prometheus-process-exporter

cat <<'EOF' | sudo tee /etc/process-exporter/config.yml
process_names:
  - name: "{{.Comm}}"
    cmdline:
    - '.+'
EOF

sudo systemctl enable --now prometheus-process-exporter
```

### 2. Scrape it from your Prometheus

```yaml
# prometheus.yml
scrape_configs:
  - job_name: process_exporter
    static_configs:
      - targets:
          - host-a:9256
          - host-b:9256
          - host-c:9256
        labels:
          instance: <hostname>   # match your ShellFleet agent label
```

### 3. Add panels to `/etc/shellfleet/metrics.yaml`

```yaml
panels:
  - id: proc_cpu_top10
    title: Top processes by CPU (1m rate)
    unit: cpu_seconds_per_sec
    query: |
      topk(10, rate(namedprocess_namegroup_cpu_seconds_total{instance="{instance}"}[1m]))

  - id: proc_mem_top10
    title: Top processes by RSS
    unit: bytes
    query: |
      topk(10, namedprocess_namegroup_memory_bytes{instance="{instance}",memtype="resident"})
```

Reload the server (`docker compose up -d server`), open any agent's
Metrics tab, and you'll see the top 10 processes by CPU and by
memory rendered as time series with whatever retention your
Prometheus has.

## Auth on the API

Both endpoints require an authenticated session:

| Endpoint | Method | Role |
|---|---|---|
| `/api/metrics/panels` | GET | viewer + admin |
| `/api/metrics/query`  | POST | viewer + admin |

Viewers are explicitly allowed because the value of "see graphs
without write power" is real, and there's no mutation possible
through this API. The query endpoint accepts a panel **id** and an
agent id, never raw PromQL from the client, so a viewer can't craft
an expensive query.

A per-actor throttle gates the query endpoint with the same shape
as the MFA throttle: 10 failures in any 15-minute window locks the
caller out for 15 minutes.

## Range picker

The dashboard sends `range: "1h" | "6h" | "24h" | "7d"`. The server
maps each to a sane Prometheus step so a `7d` query doesn't pull
millions of points:

| Range | Step | Approx. points per series |
|---|---|---|
| `1h`  | 30 s | 120 |
| `6h`  | 1 m  | 360 |
| `24h` | 5 m  | 288 |
| `7d`  | 30 m | 336 |

The hard cap on points returned is 5,000 per series.

## Things this plugin deliberately doesn't do

- **Alerting.** Prometheus's job. ShellFleet's health probes cover
  the "agent state" alert use case; let your Prometheus + Alertmanager
  handle metrics-based alerts.
- **Writes.** Read-only. There is no `/api/metrics/push` or similar.
- **Embedding Grafana.** Iframe auth is messy; native rendering is
  cleaner and matches the rest of the UI.
- **Free-form PromQL from the client.** Panel templates only.

## Disabling the plugin

Just delete `/etc/shellfleet/metrics.yaml` (or unset the env var)
and restart the server. The dashboard's `/api/metrics/panels`
endpoint returns `{enabled: false, panels: []}` and the Metrics tab
disappears from the per-agent view.
