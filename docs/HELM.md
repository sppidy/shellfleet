# Helm chart reference — `sys-manager-agent`

Located at [`../helm/sys-manager-agent/`](../helm/sys-manager-agent/).
Installs the k8s flavor of the agent as a single-replica `Deployment`
plus a `ServiceAccount` and gated `ClusterRole`s.

For the operator-level walkthrough (when to use Helm vs the `.deb`,
RBAC posture, distroless gotchas), see [`KUBERNETES.md`](KUBERNETES.md).

## Install

```bash
helm install sysmgr ./helm/sys-manager-agent \
  --namespace sys-manager --create-namespace \
  --set server.apiUrl=https://dashboard.example.com \
  --set server.wsUrl=wss://dashboard.example.com/agent/ws
```

After install, the agent prints a one-time pairing code on first run.
Read it from the Pod logs:

```bash
kubectl -n sys-manager logs -f deploy/sysmgr-sys-manager-agent
```

Paste at `/device` in the dashboard. The cluster appears as a new
agent named `<release>-sys-manager-agent-id`.

## Values

### Server connection

| key                | type   | default | required | notes                          |
| ------------------ | ------ | ------- | -------- | ------------------------------ |
| `server.apiUrl`    | string | example | **yes**  | HTTPS base of the dashboard.   |
| `server.wsUrl`     | string | example | **yes**  | `wss://…/agent/ws`.            |

### Image

| key                          | default                                  |
| ---------------------------- | ---------------------------------------- |
| `image.repository`           | `ghcr.io/sppidy/sys-manager-agent-k8s`   |
| `image.tag`                  | `""` → falls back to `.Chart.appVersion` |
| `image.pullPolicy`           | `IfNotPresent`                           |
| `imagePullSecrets`           | `[]`                                     |

To use your own image, build with the included Dockerfile:

```bash
docker build -f Dockerfile.agent.k8s -t my-registry/agent-k8s:dev .
docker push my-registry/agent-k8s:dev
helm upgrade sysmgr ./helm/sys-manager-agent \
  --set image.repository=my-registry/agent-k8s \
  --set image.tag=dev
```

### Pairing token

| key                       | default | notes                                                         |
| ------------------------- | ------- | ------------------------------------------------------------- |
| `token.existingSecret`    | `""`    | Name of a Secret with key `agent-token`. Mounted ro at        |
|                           |         | `/etc/sys-manager/agent-token` so a Pod restart re-uses it.   |

Two paths:

1. **First install** — leave `token.existingSecret` empty. The agent
   prints a pairing code, you approve it at `/device`, and the
   issued token lands at `/etc/sys-manager/agent-token` inside the
   container. **A Pod restart will wipe it.** Promote to a Secret
   for permanence (steps below).

2. **Re-install or DR** — pre-create a Secret and reference it:

```bash
# After first pairing, capture the token:
kubectl -n sys-manager exec deploy/sysmgr-sys-manager-agent \
  -- cat /etc/sys-manager/agent-token > /tmp/agent-token

# Save it as a Secret:
kubectl -n sys-manager create secret generic sysmgr-token \
  --from-file=agent-token=/tmp/agent-token

# Re-install with the Secret reference so future restarts pick it up:
helm upgrade sysmgr ./helm/sys-manager-agent \
  --set token.existingSecret=sysmgr-token
```

### RBAC

| key            | default | grants                                                |
| -------------- | ------- | ----------------------------------------------------- |
| `rbac.read`    | `true`  | get/list/watch on the read surface (pods, deployments, services, ingresses, PVCs, events, …) |
| `rbac.exec`    | `false` | create on `pods/exec`, `pods/attach`, `pods/portforward` |
| `rbac.write`   | `false` | create/update/patch/delete + scale subresources       |

The flags are independent — flip on whatever your team needs. Each
generates a separate ClusterRole + ClusterRoleBinding so disabling
one doesn't disturb the others.

### Pod knobs

| key              | default                                |
| ---------------- | -------------------------------------- |
| `replicaCount`   | `1` (don't increase — agent is singleton) |
| `resources.requests.cpu`    | `50m`                       |
| `resources.requests.memory` | `64Mi`                      |
| `resources.limits.cpu`      | `500m`                      |
| `resources.limits.memory`   | `256Mi`                     |
| `nodeSelector`              | `{}`                        |
| `tolerations`               | `[]`                        |
| `affinity`                  | `{}`                        |
| `podLabels`                 | `{}`                        |
| `podAnnotations`            | `{}`                        |
| `extraEnv`                  | `[]` (e.g. `RUST_LOG=info`) |
| `extraEnvFrom`              | `[]` (Secret/ConfigMap refs) |

### ServiceAccount

| key                              | default |
| -------------------------------- | ------- |
| `serviceAccount.create`          | `true`  |
| `serviceAccount.annotations`     | `{}`    |
| `serviceAccount.name`            | derived from release name |

## Upgrade

```bash
helm upgrade sysmgr ./helm/sys-manager-agent \
  --reuse-values \
  --set image.tag=<new-tag>
```

`Recreate` strategy is set on the Deployment — the old Pod is killed
before the new one starts. The agent pairs once, so a brief disconnect
(while the new Pod registers with the dashboard) is the only operator-
visible effect.

## Uninstall

```bash
helm uninstall sysmgr -n sys-manager
kubectl delete namespace sys-manager   # if you used --create-namespace
```

The dashboard side keeps the agent's record + token. To revoke, visit
`/admin/agents` and delete the entry.

## Verify

```bash
# Did the agent advertise the k8s capability?
kubectl -n sys-manager logs deploy/sysmgr-sys-manager-agent \
  | grep 'agent capabilities'
# Expect: agent capabilities: ["k8s"]
# (or with extras if e.g. systemd is reachable from inside the Pod;
# normally just k8s in a stripped runtime image.)

# Did the dashboard register it?
# In the server log on the dashboard host:
docker logs sys-manager-server-1 2>&1 \
  | grep "agent registered.*$(helm list -A -f sysmgr -o json | jq -r '.[].name')"
```

## Troubleshooting

**Agent crashes with rustls panic on startup** — the binary needs a
process-level CryptoProvider. Recent images install `ring` at startup;
if you've pinned an old image, upgrade past `1.1.0-ci202604280311`.

**Logs/exec returns "client error (Connect)"** — the agent's
identity resolution picked something other than the in-cluster SA.
Confirm with:

```bash
kubectl -n sys-manager exec deploy/sysmgr-sys-manager-agent \
  -- ls /var/run/secrets/kubernetes.io/serviceaccount/
```

If `KUBECONFIG` is in `extraEnv` and points at a stale file, drop it.

**`exec` button gives "[ session ended ]" immediately** — the target
container is distroless (no `/bin/sh`). Try a different container or
use `kubectl debug` for the upstream pattern.
