//! K8s API surface — only compiled into the `sys-manager-agent-k8s`
//! .deb (cargo `--features kube`). Standard host-agent .debs don't
//! pull kube-rs or k8s-openapi, so the binary stays lean.
//!
//! Identity resolution falls through `Client::try_default()`:
//!   1. `KUBECONFIG` env var if set,
//!   2. `~/.kube/config`,
//!   3. in-cluster ServiceAccount token (when running as a Pod).
//! That covers both deployment shapes — kubeconfig-on-a-host AND
//! Deployment-in-a-cluster — without any agent-side config flag.

use chrono::Utc;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client, ResourceExt};
use shared::K8sPod;

/// Cheap availability probe used at agent startup to decide whether
/// to advertise the `"k8s"` capability. Constructing a `Client`
/// resolves the identity (kubeconfig / SA token) but does NOT make
/// an apiserver call, so this is roughly free even when the agent
/// is not on a k8s host.
pub async fn k8s_available() -> bool {
    Client::try_default().await.is_ok()
}

/// List pods across every namespace the agent's identity has `list`
/// rights on. Cluster-admin kubeconfig (k3s default) sees everything;
/// the Helm chart's read-only ClusterRole sees everything for now,
/// namespace-scoped overlays are an EE concern.
pub async fn list_pods() -> Result<Vec<K8sPod>, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Pod> = Api::all(client);
    let list = api
        .list(&Default::default())
        .await
        .map_err(|e| format!("list pods: {e}"))?;

    let now = Utc::now();
    let pods: Vec<K8sPod> = list
        .items
        .iter()
        .map(|p| {
            let namespace = p.namespace().unwrap_or_default();
            let name = p.name_any();
            let status = p.status.as_ref();
            let phase = status
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("Unknown")
                .to_string();
            let container_statuses = status.and_then(|s| s.container_statuses.as_ref());
            let ready_count = container_statuses
                .map(|cs| cs.iter().filter(|c| c.ready).count())
                .unwrap_or(0);
            let total = p
                .spec
                .as_ref()
                .map(|s| s.containers.len())
                .unwrap_or(0);
            let ready = format!("{ready_count}/{total}");
            let restarts: u32 = container_statuses
                .map(|cs| cs.iter().map(|c| c.restart_count.max(0) as u32).sum())
                .unwrap_or(0);
            let age_secs = p
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| (now - t.0).num_seconds())
                .unwrap_or(0);
            let node = p.spec.as_ref().and_then(|s| s.node_name.clone());
            let containers = p
                .spec
                .as_ref()
                .map(|s| s.containers.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();
            K8sPod {
                namespace,
                name,
                phase,
                ready,
                restarts,
                age_secs,
                node,
                containers,
            }
        })
        .collect();

    Ok(pods)
}
