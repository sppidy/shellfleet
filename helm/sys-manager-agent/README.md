# sys-manager-agent (Helm)

In-cluster install of the sys-manager k8s agent. Pulls the cluster
into the dashboard's Kubernetes tab — pods, deployments, services,
ingresses, PVCs, events, describe, logs, and (opt-in) `kubectl exec`.

## Quick start

```bash
helm install sysmgr ./helm/sys-manager-agent \
  --namespace sys-manager --create-namespace \
  --set server.apiUrl=https://dashboard.example.com \
  --set server.wsUrl=wss://dashboard.example.com/agent/ws
```

Pair on first run by tailing the Pod logs:

```bash
kubectl -n sys-manager logs -f deploy/sysmgr-sys-manager-agent
```

Paste the printed code at `/device` in the dashboard. The agent
re-registers with a token and starts answering Kubernetes queries.

## RBAC

| flag           | what it grants                                   | default |
| -------------- | ------------------------------------------------ | ------- |
| `rbac.read`    | get/list/watch pods, deps, svcs, ingresses, …    | **on**  |
| `rbac.exec`    | create on pods/exec, attach, portforward         | off     |
| `rbac.write`   | create/update/patch/delete + scale subresources  | off     |

`exec` is the only knob you'd flip for slice 4 functionality. `write`
exists today as a forward-compatible binding — the agent doesn't ship
apply handlers yet, that lands in slice 6 / EE multi-cluster.

## Image

CI publishes multi-arch images to `hrbr.sppidy.in/sys-manager/agent-k8s`
on every k8s-related commit. The chart's default image points there.

To roll your own:

```bash
docker build -f Dockerfile.agent.k8s -t my-registry/agent-k8s:latest .
helm upgrade sysmgr ./helm/sys-manager-agent \
  --set image.repository=my-registry/agent-k8s \
  --set image.tag=latest
```

## See also

- `docs/KUBERNETES.md` — operator overview, install paths, limitations.
- `docs/HELM.md` — every value reference + upgrade / uninstall.
