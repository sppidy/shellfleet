# Kubernetes support

CE includes single-cluster Kubernetes management. Read-mostly today,
with exec-style shells into pods. Multi-cluster federation, Helm-release
UI, and namespace-scoped RBAC overlays are EE.

## What you get (CE)

| subtab            | content                                                          |
| ----------------- | ---------------------------------------------------------------- |
| **pods**          | namespace, name, ready, status, restarts, age, node              |
| **deployments**   | ready, up-to-date, available, image                              |
| **services**      | type, cluster-ip, external-ip, ports                             |
| **ingresses**     | class, hosts, addresses                                          |
| **pvcs**          | status, bound volume, capacity, access modes, storage class      |
| **events**        | last seen, type, reason, object, count, message (capped at 200)  |

On every row of the first five subtabs:

- **describe** -- click a name → modal with the full apiserver YAML,
  sectioned by kubectl-style fields, copyable.
- **logs** (pods only) -- live tail with auto-follow, container picker,
  5000-line scrollback.
- **exec** (pods only) -- embedded xterm with `/bin/sh` into the chosen
  container.

Nothing from the cluster is stored. Every render is one apiserver list
call, polled every 5 seconds while the tab is open. When nobody is
looking, the agent is idle.

## Two install shapes

Same agent binary, two ways to run it. Pick whichever fits.

### A) In-cluster Pod (recommended)

A Helm chart in [`helm/shellfleet-agent/`](../helm/shellfleet-agent/)
deploys the agent as a single-replica `Deployment` with a dedicated
`ServiceAccount` and a read-only `ClusterRole`. Pairing happens via the
device-auth flow -- the Pod prints a code at first run, you paste it at
`/device`.

```bash
helm install sysmgr ./helm/shellfleet-agent \
  --namespace shellfleet --create-namespace \
  --set server.apiUrl=https://dashboard.example.com \
  --set server.wsUrl=wss://dashboard.example.com/agent/ws

kubectl -n shellfleet logs -f deploy/sysmgr-shellfleet-agent
```

See [`HELM.md`](HELM.md) for every configurable value.

### B) Out-of-cluster, on a Linux host

Install the `shellfleet-agent-k8s` `.deb` on a host that can reach your
kube-apiserver, point `KUBECONFIG` at a credential, and the agent treats
the cluster as another target.

```bash
sudo apt install shellfleet-agent-k8s
echo 'KUBECONFIG=/etc/shellfleet/kubeconfig' \
  | sudo tee -a /etc/shellfleet/env
sudo install -m 0640 your-kubeconfig /etc/shellfleet/kubeconfig
sudo systemctl restart shellfleet-agent
```

The `.deb` is mutually exclusive with the standard `shellfleet-agent`
package (Conflicts/Provides). One or the other, not both.

## RBAC posture

CE defaults are read-mostly. Two Helm flags escalate:

```bash
helm upgrade sysmgr ./helm/shellfleet-agent \
  --set rbac.exec=true        # enables pod exec / attach / portforward
  # --set rbac.write=true     # reserved for slice 6 (apply / scale)
```

The `read` ClusterRole covers: pods, pods/log, services, PVCs, events,
namespaces, nodes, configmaps, deployments, statefulsets, daemonsets,
replicasets, ingresses, networkpolicies, jobs, cronjobs.

Out-of-cluster installs use whatever permissions the kubeconfig grants.
A cluster-admin kubeconfig works but is overkill.

## Identity resolution

The agent calls `kube::Client::try_default()`, which falls through:

1. `KUBECONFIG` env var (set in `/etc/shellfleet/env` for `.deb`
   installs, or via the Helm chart's `extraEnv`).
2. `~/.kube/config`.
3. In-cluster `ServiceAccount` token at
   `/var/run/secrets/kubernetes.io/serviceaccount/`.

In-cluster installs always end up at #3. Out-of-cluster installs
need #1 or #2.

> **Gotcha:** if `/root/.kube/config` exists from a previous setup,
> the agent picks it up first and may talk to the wrong cluster.
> Set `KUBECONFIG` explicitly to override.

## Limitations (today)

- **Single cluster per agent.** One Pod = one cluster. Multi-cluster
  federation is EE.
- **No apply / scale / delete.** Slice 6 / v2. The Helm chart's
  `rbac.write` toggle pre-creates the binding so when the feature
  lands, no Helm upgrade is needed.
- **No Helm releases UI.** EE.
- **Distroless images can't exec.** `coredns`, `metrics-server`,
  anything from `gcr.io/distroless/*` ships without `/bin/sh`. The
  exec modal renders `[ session ended ]` immediately. Not a bug --
  use `kubectl debug` when you need a shell on those.
- **Token persistence in the Pod.** First-run pairing writes to
  `/etc/shellfleet/agent-token` inside the container. A Pod restart
  wipes it. Promote to a `Secret` after first pair (see `HELM.md`).

## Roadmap (EE)

- Multi-cluster federation -- one dashboard, N clusters, switch between
  them.
- Helm releases UI -- list / install / upgrade / rollback / values diff.
- Namespace-scoped RBAC overlays -- operator A sees only namespace X,
  operator B sees only namespace Y.
- Operator (CRD + controller) for "install ShellFleet into these N
  clusters from one declaration".
- AI log analysis on top of the existing K8sLogs stream.

See [`../README.md`](../README.md) for the broader CE/EE split.
