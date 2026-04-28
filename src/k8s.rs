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
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{
    Event, PersistentVolumeClaim, Pod, Service,
};
use k8s_openapi::api::networking::v1::Ingress;
use kube::{Api, Client, ResourceExt};
use shared::{K8sDeployment, K8sEvent, K8sIngress, K8sPod, K8sPvc, K8sService};

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

pub async fn list_deployments() -> Result<Vec<K8sDeployment>, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Deployment> = Api::all(client);
    let list = api
        .list(&Default::default())
        .await
        .map_err(|e| format!("list deployments: {e}"))?;

    let now = Utc::now();
    let out = list
        .items
        .iter()
        .map(|d| {
            let namespace = d.namespace().unwrap_or_default();
            let name = d.name_any();
            let spec_replicas = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
            let status = d.status.as_ref();
            let ready_replicas = status.and_then(|s| s.ready_replicas).unwrap_or(0);
            let up_to_date = status.and_then(|s| s.updated_replicas).unwrap_or(0);
            let available = status.and_then(|s| s.available_replicas).unwrap_or(0);
            let age_secs = d
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| (now - t.0).num_seconds())
                .unwrap_or(0);
            let image = d
                .spec
                .as_ref()
                .and_then(|s| s.template.spec.as_ref())
                .and_then(|p| p.containers.first())
                .and_then(|c| c.image.clone());
            K8sDeployment {
                namespace,
                name,
                ready: format!("{ready_replicas}/{spec_replicas}"),
                up_to_date,
                available,
                age_secs,
                image,
            }
        })
        .collect();
    Ok(out)
}

pub async fn list_services() -> Result<Vec<K8sService>, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Service> = Api::all(client);
    let list = api
        .list(&Default::default())
        .await
        .map_err(|e| format!("list services: {e}"))?;

    let now = Utc::now();
    let out = list
        .items
        .iter()
        .map(|s| {
            let namespace = s.namespace().unwrap_or_default();
            let name = s.name_any();
            let spec = s.spec.as_ref();
            let kind = spec
                .and_then(|s| s.type_.clone())
                .unwrap_or_else(|| "ClusterIP".into());
            let cluster_ip = spec.and_then(|s| s.cluster_ip.clone());
            let external_ips = spec
                .and_then(|s| s.external_ips.clone())
                .unwrap_or_default();
            let ports = spec
                .and_then(|s| s.ports.as_ref())
                .map(|ps| {
                    ps.iter()
                        .map(|p| {
                            let proto = p.protocol.clone().unwrap_or_else(|| "TCP".into());
                            match p.node_port {
                                Some(np) => format!("{}:{}/{}", p.port, np, proto),
                                None => format!("{}/{}", p.port, proto),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let age_secs = s
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| (now - t.0).num_seconds())
                .unwrap_or(0);
            K8sService {
                namespace,
                name,
                kind,
                cluster_ip,
                external_ips,
                ports,
                age_secs,
            }
        })
        .collect();
    Ok(out)
}

pub async fn list_ingresses() -> Result<Vec<K8sIngress>, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Ingress> = Api::all(client);
    let list = api
        .list(&Default::default())
        .await
        .map_err(|e| format!("list ingresses: {e}"))?;

    let now = Utc::now();
    let out = list
        .items
        .iter()
        .map(|ing| {
            let namespace = ing.namespace().unwrap_or_default();
            let name = ing.name_any();
            let class = ing
                .spec
                .as_ref()
                .and_then(|s| s.ingress_class_name.clone());
            let hosts: Vec<String> = ing
                .spec
                .as_ref()
                .and_then(|s| s.rules.as_ref())
                .map(|rs| rs.iter().filter_map(|r| r.host.clone()).collect())
                .unwrap_or_default();
            let addresses: Vec<String> = ing
                .status
                .as_ref()
                .and_then(|s| s.load_balancer.as_ref())
                .and_then(|lb| lb.ingress.as_ref())
                .map(|ents| {
                    ents.iter()
                        .filter_map(|e| e.ip.clone().or_else(|| e.hostname.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let age_secs = ing
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| (now - t.0).num_seconds())
                .unwrap_or(0);
            K8sIngress {
                namespace,
                name,
                class,
                hosts,
                addresses,
                age_secs,
            }
        })
        .collect();
    Ok(out)
}

pub async fn list_pvcs() -> Result<Vec<K8sPvc>, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<PersistentVolumeClaim> = Api::all(client);
    let list = api
        .list(&Default::default())
        .await
        .map_err(|e| format!("list pvcs: {e}"))?;

    let now = Utc::now();
    let out = list
        .items
        .iter()
        .map(|p| {
            let namespace = p.namespace().unwrap_or_default();
            let name = p.name_any();
            let status_obj = p.status.as_ref();
            let status = status_obj
                .and_then(|s| s.phase.clone())
                .unwrap_or_else(|| "Unknown".into());
            let volume_name = p.spec.as_ref().and_then(|s| s.volume_name.clone());
            let capacity = status_obj
                .and_then(|s| s.capacity.as_ref())
                .and_then(|c| c.get("storage"))
                .map(|q| q.0.clone());
            let access_modes = p
                .spec
                .as_ref()
                .and_then(|s| s.access_modes.as_ref())
                .map(|m| {
                    m.iter()
                        .map(|s| match s.as_str() {
                            "ReadWriteOnce" => "RWO".to_string(),
                            "ReadOnlyMany" => "ROX".to_string(),
                            "ReadWriteMany" => "RWX".to_string(),
                            "ReadWriteOncePod" => "RWOP".to_string(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let storage_class = p
                .spec
                .as_ref()
                .and_then(|s| s.storage_class_name.clone());
            let age_secs = p
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| (now - t.0).num_seconds())
                .unwrap_or(0);
            K8sPvc {
                namespace,
                name,
                status,
                volume_name,
                capacity,
                access_modes,
                storage_class,
                age_secs,
            }
        })
        .collect();
    Ok(out)
}

/// Cluster-wide events, newest-first, capped at 200. A busy cluster
/// can produce hundreds of events per minute and we don't want to
/// blow up the WS frame.
pub async fn list_events() -> Result<Vec<K8sEvent>, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Event> = Api::all(client);
    let list = api
        .list(&Default::default())
        .await
        .map_err(|e| format!("list events: {e}"))?;

    let now = Utc::now();
    let mut out: Vec<K8sEvent> = list
        .items
        .iter()
        .map(|e| {
            let namespace = e.metadata.namespace.clone().unwrap_or_default();
            let kind = e.type_.clone().unwrap_or_else(|| "Normal".into());
            let reason = e.reason.clone().unwrap_or_default();
            let obj_kind = e
                .involved_object
                .kind
                .clone()
                .unwrap_or_else(|| "?".into());
            let obj_name = e
                .involved_object
                .name
                .clone()
                .unwrap_or_else(|| "?".into());
            let object = format!("{obj_kind}/{obj_name}");
            let message = e.message.clone().unwrap_or_default();
            let count = e.count.unwrap_or(1);
            // Prefer last_timestamp; fall back to event_time, then creation.
            let ts = e
                .last_timestamp
                .as_ref()
                .map(|t| t.0)
                .or_else(|| e.event_time.as_ref().map(|t| t.0))
                .or_else(|| e.metadata.creation_timestamp.as_ref().map(|t| t.0));
            let age_secs = ts.map(|t| (now - t).num_seconds()).unwrap_or(0);
            K8sEvent {
                namespace,
                kind,
                reason,
                object,
                message,
                count,
                age_secs,
            }
        })
        .collect();

    // Newest first (smallest age).
    out.sort_by_key(|e| e.age_secs);
    out.truncate(200);

    Ok(out)
}
