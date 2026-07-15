//! K8s API surface — only compiled into the `shellfleet-agent-k8s`
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
use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Pod, Service};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{DeleteParams, DynamicObject, GroupVersionKind, Patch, PatchParams};
use kube::{Api, Client, ResourceExt, discovery};
use shared::{K8sDeployment, K8sEvent, K8sIngress, K8sPod, K8sPvc, K8sService, MAX_OUTPUT_BYTES};

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
            let total = p.spec.as_ref().map(|s| s.containers.len()).unwrap_or(0);
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
            let class = ing.spec.as_ref().and_then(|s| s.ingress_class_name.clone());
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
            let storage_class = p.spec.as_ref().and_then(|s| s.storage_class_name.clone());
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
            let obj_kind = e.involved_object.kind.clone().unwrap_or_else(|| "?".into());
            let obj_name = e.involved_object.name.clone().unwrap_or_else(|| "?".into());
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

/// Fetch one object and return it as YAML. The kind dispatch is
/// explicit because each typed Api<T> wants a different `T`; we
/// could go through kube's DynamicObject API instead but the
/// trade-off (looser typing, runtime kind resolution) isn't worth
/// it for the six concrete kinds we expose. `managedFields` is
/// stripped to keep the modal readable — kubectl does the same by
/// default in its describe output.
pub async fn describe(kind: &str, namespace: Option<&str>, name: &str) -> Result<String, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;

    let yaml = match kind {
        "pod" => describe_one::<Pod>(client, namespace, name).await?,
        "deployment" => describe_one::<Deployment>(client, namespace, name).await?,
        "service" => describe_one::<Service>(client, namespace, name).await?,
        "ingress" => describe_one::<Ingress>(client, namespace, name).await?,
        "pvc" => describe_one::<PersistentVolumeClaim>(client, namespace, name).await?,
        "event" => describe_one::<Event>(client, namespace, name).await?,
        other => return Err(format!("unsupported kind: {other}")),
    };
    if yaml.len() > MAX_OUTPUT_BYTES {
        return Err(format!(
            "describe response exceeds {MAX_OUTPUT_BYTES}-byte limit"
        ));
    }
    Ok(yaml)
}

// ─── slice 6 (v2): apply / scale / delete ──────────────────────

const APPLY_FIELD_MANAGER: &str = "shellfleet";
const APPLY_NAMESPACES_ENV: &str = "SHELLFLEET_K8S_APPLY_NAMESPACES";

const ALLOWED_APPLY_RESOURCES: &[(&str, &str)] = &[
    ("v1", "Pod"),
    ("v1", "Service"),
    ("v1", "ConfigMap"),
    ("v1", "Secret"),
    ("v1", "PersistentVolumeClaim"),
    ("apps/v1", "Deployment"),
    ("apps/v1", "StatefulSet"),
    ("apps/v1", "DaemonSet"),
    ("apps/v1", "ReplicaSet"),
    ("batch/v1", "Job"),
    ("batch/v1", "CronJob"),
    ("networking.k8s.io/v1", "Ingress"),
    ("autoscaling/v2", "HorizontalPodAutoscaler"),
];

fn configured_apply_namespaces() -> Result<Vec<String>, String> {
    let raw = std::env::var(APPLY_NAMESPACES_ENV).map_err(|_| {
        format!(
            "Kubernetes apply is disabled: set {APPLY_NAMESPACES_ENV} to a comma-separated namespace allow-list"
        )
    })?;
    let namespaces: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_owned)
        .collect();

    if namespaces.is_empty() {
        return Err(format!(
            "Kubernetes apply is disabled: {APPLY_NAMESPACES_ENV} contains no namespaces"
        ));
    }
    if namespaces.iter().any(|namespace| namespace == "*") {
        return Err(format!(
            "Kubernetes apply refuses wildcard namespaces in {APPLY_NAMESPACES_ENV}"
        ));
    }
    Ok(namespaces)
}

fn validate_pod_spec(spec: &serde_json::Value) -> Result<(), String> {
    for field in ["hostNetwork", "hostPID", "hostIPC"] {
        if spec.get(field).and_then(serde_json::Value::as_bool) == Some(true) {
            return Err(format!("pod spec may not enable {field}"));
        }
    }

    if spec
        .get("automountServiceAccountToken")
        .and_then(serde_json::Value::as_bool)
        != Some(false)
    {
        return Err(
            "pod spec must set automountServiceAccountToken: false to prevent service-account impersonation"
                .into(),
        );
    }

    if let Some(volumes) = spec.get("volumes").and_then(serde_json::Value::as_array) {
        for volume in volumes {
            if volume.get("hostPath").is_some() {
                return Err("pod spec may not mount hostPath volumes".into());
            }
            let projected_sources = volume
                .pointer("/projected/sources")
                .and_then(serde_json::Value::as_array);
            if projected_sources.is_some_and(|sources| {
                sources
                    .iter()
                    .any(|source| source.get("serviceAccountToken").is_some())
            }) {
                return Err("pod spec may not project service-account tokens".into());
            }
        }
    }

    for container_field in ["containers", "initContainers", "ephemeralContainers"] {
        let Some(containers) = spec
            .get(container_field)
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for container in containers {
            let security = container.get("securityContext");
            if security
                .and_then(|value| value.get("privileged"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                return Err(format!("{container_field} may not run privileged"));
            }
            if security
                .and_then(|value| value.get("allowPrivilegeEscalation"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                return Err(format!(
                    "{container_field} may not allow privilege escalation"
                ));
            }
            if security
                .and_then(|value| value.pointer("/capabilities/add"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|capabilities| !capabilities.is_empty())
            {
                return Err(format!("{container_field} may not add Linux capabilities"));
            }
            if security
                .and_then(|value| value.get("procMount"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|mode| mode.eq_ignore_ascii_case("Unmasked"))
            {
                return Err(format!("{container_field} may not use an unmasked /proc"));
            }
            if security
                .and_then(|value| value.pointer("/seccompProfile/type"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|mode| mode.eq_ignore_ascii_case("Unconfined"))
            {
                return Err(format!("{container_field} may not disable seccomp"));
            }
            if security
                .and_then(|value| value.pointer("/windowsOptions/hostProcess"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                return Err(format!("{container_field} may not use Windows HostProcess"));
            }

            if container
                .get("ports")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|ports| {
                    ports.iter().any(|port| {
                        port.get("hostPort")
                            .and_then(serde_json::Value::as_u64)
                            .is_some_and(|port| port > 0)
                    })
                })
            {
                return Err(format!("{container_field} may not expose host ports"));
            }
        }
    }
    Ok(())
}

fn validate_apply_document(
    document: &serde_json::Value,
    allowed_namespaces: &[String],
    force: bool,
) -> Result<(), String> {
    if force {
        return Err(
            "forced server-side apply is disabled because it can seize field ownership".into(),
        );
    }

    let api_version = document
        .get("apiVersion")
        .and_then(serde_json::Value::as_str)
        .ok_or("manifest is missing apiVersion")?;
    let kind = document
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or("manifest is missing kind")?;
    if !ALLOWED_APPLY_RESOURCES.contains(&(api_version, kind)) {
        return Err(format!(
            "apply of {api_version} {kind} is not allowed; cluster-scoped, RBAC, and custom resources are denied"
        ));
    }

    let namespace = document
        .pointer("/metadata/namespace")
        .and_then(serde_json::Value::as_str)
        .filter(|namespace| !namespace.is_empty())
        .ok_or("manifest must set metadata.namespace")?;
    if !allowed_namespaces
        .iter()
        .any(|allowed| allowed == namespace)
    {
        return Err(format!("apply to namespace {namespace:?} is not allowed"));
    }

    let pod_spec_pointer = match kind {
        "Pod" => Some("/spec"),
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job" => {
            Some("/spec/template/spec")
        }
        "CronJob" => Some("/spec/jobTemplate/spec/template/spec"),
        _ => None,
    };
    if let Some(pointer) = pod_spec_pointer {
        let spec = document
            .pointer(pointer)
            .ok_or_else(|| format!("{kind} manifest is missing its pod spec"))?;
        validate_pod_spec(spec)?;
    }

    Ok(())
}

/// Server-side apply of one or more YAML docs. Multi-doc input
/// (`---` separated) is supported; each doc is applied in order
/// and the joined result is returned. The dynamic-object path
/// uses kube's discovery to resolve the GVK to an `ApiResource`,
/// so any cluster-known kind works without a per-kind match.
pub async fn apply(yaml: &str, dry_run: bool, force: bool) -> Result<String, String> {
    let allowed_namespaces = configured_apply_namespaces()?;
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;

    let mut out = String::new();

    for (i, raw) in yaml.split("\n---\n").enumerate() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let obj: DynamicObject =
            serde_yaml::from_str(raw).map_err(|e| format!("doc {i}: parse: {e}"))?;
        let document =
            serde_json::to_value(&obj).map_err(|e| format!("doc {i}: normalize manifest: {e}"))?;
        validate_apply_document(&document, &allowed_namespaces, force)
            .map_err(|e| format!("doc {i}: policy: {e}"))?;
        let types = obj
            .types
            .as_ref()
            .ok_or_else(|| format!("doc {i}: missing apiVersion/kind"))?;
        let gvk = GroupVersionKind::try_from(types).map_err(|e| format!("doc {i}: gvk: {e}"))?;

        let (ar, caps) = discovery::pinned_kind(&client, &gvk)
            .await
            .map_err(|e| format!("doc {i}: discovery: {e}"))?;

        let name = obj.name_any();
        let api: Api<DynamicObject> = if caps.scope == discovery::Scope::Namespaced {
            // No silent "default" fallback: the operator must spell
            // out the target namespace in the manifest. Otherwise a
            // forgotten `metadata.namespace:` would route the apply
            // into `default` without anyone noticing.
            let ns = obj.metadata.namespace.clone().ok_or_else(|| {
                format!(
                    "doc {i} ({} {}): missing metadata.namespace; \
                     specify it in the manifest (namespaced kind, \
                     no default fallback)",
                    gvk.kind, name,
                )
            })?;
            Api::namespaced_with(client.clone(), &ns, &ar)
        } else {
            Api::all_with(client.clone(), &ar)
        };

        let mut params = PatchParams::apply(APPLY_FIELD_MANAGER);
        if dry_run {
            params.dry_run = true;
        }

        match api.patch(&name, &params, &Patch::Apply(&obj)).await {
            Ok(applied) => {
                out.push_str(&format!(
                    "{} {}/{} {}\n",
                    if dry_run { "(dry-run)" } else { "applied" },
                    gvk.kind,
                    obj.metadata.namespace.as_deref().unwrap_or("-"),
                    applied.name_any(),
                ));
            }
            Err(e) => {
                return Err(format!(
                    "doc {i} ({} {}/{}): {e}",
                    gvk.kind,
                    obj.metadata.namespace.as_deref().unwrap_or("-"),
                    name,
                ));
            }
        }
    }

    Ok(out)
}

/// Scale a Deployment / StatefulSet / ReplicaSet to `replicas` via
/// the `/scale` subresource. `kind` is matched case-insensitively.
pub async fn scale(kind: &str, namespace: &str, name: &str, replicas: i32) -> Result<(), String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;

    let patch = serde_json::json!({ "spec": { "replicas": replicas } });
    let params = PatchParams::default();

    match kind.to_lowercase().as_str() {
        "deployment" | "deploy" | "deployments" => {
            let api: Api<Deployment> = Api::namespaced(client, namespace);
            api.patch_scale(name, &params, &Patch::Merge(&patch))
                .await
                .map_err(|e| format!("scale deployment: {e}"))?;
        }
        "statefulset" | "sts" | "statefulsets" => {
            let api: Api<StatefulSet> = Api::namespaced(client, namespace);
            api.patch_scale(name, &params, &Patch::Merge(&patch))
                .await
                .map_err(|e| format!("scale statefulset: {e}"))?;
        }
        "replicaset" | "rs" | "replicasets" => {
            let api: Api<ReplicaSet> = Api::namespaced(client, namespace);
            api.patch_scale(name, &params, &Patch::Merge(&patch))
                .await
                .map_err(|e| format!("scale replicaset: {e}"))?;
        }
        other => return Err(format!("unsupported scale kind: {other}")),
    }

    Ok(())
}

/// Delete a single pod. `grace_period_secs = Some(0)` is the
/// `--force --grace-period=0` equivalent (immediate delete).
pub async fn delete_pod(
    namespace: &str,
    name: &str,
    grace_period_secs: Option<i64>,
) -> Result<(), String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let mut params = DeleteParams::default();
    if let Some(g) = grace_period_secs {
        params = params.grace_period(g.max(0) as u32);
    }
    api.delete(name, &params)
        .await
        .map_err(|e| format!("delete pod: {e}"))?;
    Ok(())
}

async fn describe_one<K>(
    client: Client,
    namespace: Option<&str>,
    name: &str,
) -> Result<String, String>
where
    K: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>
        + serde::Serialize
        + Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
    <K as kube::Resource>::DynamicType: Default,
{
    let api: Api<K> = match namespace {
        Some(ns) => Api::namespaced(client, ns),
        // None on a namespaced resource is meaningless; fail loud.
        None => return Err("namespaced kind needs a namespace".into()),
    };
    let mut obj = api.get(name).await.map_err(|e| format!("get: {e}"))?;
    // Strip managedFields — verbose, useless in a describe view.
    obj.meta_mut().managed_fields = None;
    serde_yaml::to_string(&obj).map_err(|e| format!("yaml: {e}"))
}

#[cfg(test)]
mod tests {
    use super::validate_apply_document;
    use serde_json::json;

    fn allowed_namespaces() -> Vec<String> {
        vec!["ops".to_string()]
    }

    fn safe_deployment() -> serde_json::Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "web", "namespace": "ops" },
            "spec": {
                "selector": { "matchLabels": { "app": "web" } },
                "template": {
                    "metadata": { "labels": { "app": "web" } },
                    "spec": {
                        "containers": [{
                            "name": "web",
                            "image": "nginx:1.27",
                            "securityContext": {
                                "allowPrivilegeEscalation": false
                            }
                        }],
                        "automountServiceAccountToken": false
                    }
                }
            }
        })
    }

    #[test]
    fn apply_policy_allows_safe_namespaced_workload() {
        assert!(validate_apply_document(&safe_deployment(), &allowed_namespaces(), false).is_ok());

        // Exercise the same DynamicObject normalization used by apply(), not
        // only a hand-built JSON value.
        let manifest = serde_yaml::to_string(&safe_deployment()).unwrap();
        let object: kube::api::DynamicObject = serde_yaml::from_str(&manifest).unwrap();
        let normalized = serde_json::to_value(object).unwrap();
        assert!(validate_apply_document(&normalized, &allowed_namespaces(), false).is_ok());
    }

    #[test]
    fn apply_policy_rejects_cluster_scoped_and_wrong_namespace() {
        let binding = json!({
            "apiVersion": "rbac.authorization.k8s.io/v1",
            "kind": "ClusterRoleBinding",
            "metadata": { "name": "takeover" }
        });
        assert!(validate_apply_document(&binding, &allowed_namespaces(), false).is_err());

        let mut deployment = safe_deployment();
        deployment["metadata"]["namespace"] = json!("kube-system");
        assert!(validate_apply_document(&deployment, &allowed_namespaces(), false).is_err());
    }

    #[test]
    fn apply_policy_rejects_forced_field_ownership() {
        assert!(validate_apply_document(&safe_deployment(), &allowed_namespaces(), true).is_err());
    }

    #[test]
    fn apply_policy_rejects_privileged_pod_settings() {
        let mut deployment = safe_deployment();
        deployment["spec"]["template"]["spec"]["hostPID"] = json!(true);
        assert!(validate_apply_document(&deployment, &allowed_namespaces(), false).is_err());

        let mut deployment = safe_deployment();
        deployment["spec"]["template"]["spec"]["containers"][0]["securityContext"]["privileged"] =
            json!(true);
        assert!(validate_apply_document(&deployment, &allowed_namespaces(), false).is_err());

        let mut deployment = safe_deployment();
        deployment["spec"]["template"]["spec"]["volumes"] = json!([{
            "name": "host",
            "hostPath": { "path": "/" }
        }]);
        assert!(validate_apply_document(&deployment, &allowed_namespaces(), false).is_err());
    }
}
