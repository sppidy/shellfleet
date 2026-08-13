# shellfleet-agent (Helm)

In-cluster install of the ShellFleet k8s agent. Pulls the cluster
into the dashboard's Kubernetes tab — pods, deployments, services,
ingresses, PVCs, events, describe, logs, and (opt-in) `kubectl exec`.

## Quick start

```bash
helm install sysmgr ./helm/shellfleet-agent \
  --namespace shellfleet --create-namespace \
  --set server.apiUrl=https://dashboard.example.com \
  --set server.wsUrl=wss://dashboard.example.com/agent/ws
```

Pair on first run by tailing the Pod logs:

```bash
kubectl -n shellfleet logs -f deploy/sysmgr-shellfleet-agent
```

Paste the printed code at `/device` in the dashboard. The agent
stores its rotating credentials on a PVC and starts answering Kubernetes
queries. Restarts and chart upgrades reuse that identity without another
operator approval.

By default the chart provisions a retained 64 MiB `ReadWriteOnce` PVC. To use
an existing claim, set `persistence.existingClaim`. For an intentionally
ephemeral install, set `persistence.enabled=false`; Pod replacement will then
require pairing again unless bootstrap credentials are supplied.

To migrate credentials from a Secret, set `token.existingSecret`. The Secret
must contain `agent-token.txt` and may also contain `agent-refresh.txt` and
`agent-token-expiry.txt`. The files are copied only when the state volume is
uninitialized, so future refresh-token rotations are not reset to stale Secret
data. Treat the Secret as a one-time migration input, not a backup: after the
agent connects from the PVC, clear `token.existingSecret` on the next upgrade
and delete the stale Secret according to your credential-retention policy.

## RBAC

| flag           | what it grants                                   | default |
| -------------- | ------------------------------------------------ | ------- |
| `rbac.read`    | get/list/watch pods, deps, svcs, ingresses, …    | **on**  |
| `rbac.exec`    | create/get on pods/exec and pods/attach          | off     |
| `rbac.write`   | create/update/patch/delete + scale subresources  | off     |

`exec` is the only knob you'd flip for slice 4 functionality. `write`
exists today as a forward-compatible binding — the agent doesn't ship
apply handlers yet, that lands in slice 6 / EE multi-cluster.

## Image

CI publishes multi-arch images to `ghcr.io/sppidy/shellfleet/agent-k8s`
on every k8s-related commit. The chart's default image points there.

To roll your own:

```bash
docker build -f Dockerfile.agent.k8s -t my-registry/agent-k8s:latest .
helm upgrade sysmgr ./helm/shellfleet-agent \
  --set image.repository=my-registry/agent-k8s \
  --set image.tag=latest
```

## Credential cleanup

The chart annotates its PVC with Helm's `keep` policy by default. Uninstalling
the release therefore does not discard the paired identity. Delete the claim
explicitly when that is your intent:

```bash
kubectl -n shellfleet delete pvc <release>-shellfleet-agent-state
```
