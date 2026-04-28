use serde::{Deserialize, Serialize};

/// Protocol version sent by the agent in the Register handshake. Bump when
/// the wire format changes in a way the server needs to reject older agents
/// for. Value `0` means "legacy agent that predates this field" — those
/// still connect, just without the version-aware fast paths.
/// v16: k8s read API — `K8sListPodsRequest` / `K8sListPodsResponse`
/// for the slice-1 Pods view. Only k8s-feature agents (advertising
/// the `"k8s"` capability) handle these; others return an error.
///
/// v15: Register gains `capabilities: Vec<String>` so the dashboard
/// can hide tabs the agent can't serve. Standard caps: `"systemd"`,
/// `"docker"`, `"swarm"`, `"k8s"`. Empty / missing = legacy v14
/// agent; UI falls back to showing every tab.
///
/// v14: terminal variants gain `session_id` for multi-PTY per host.
/// Empty string = the singleton container-exec session (DockerExec*).
pub const PROTOCOL_VERSION: u32 = 16;

fn default_protocol_version() -> u32 {
    0
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SwarmRole {
    NotInSwarm,
    Worker,
    Manager,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DockerContainer {
    pub id: String,
    pub names: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwarmService {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub replicas: String,
    pub image: String,
    pub ports: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwarmNode {
    pub id: String,
    pub hostname: String,
    pub status: String,
    pub availability: String,
    pub manager_status: String,
    pub engine_version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    /// Agent registering with the server
    Register {
        hostname: String,
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
        /// What this agent can serve. Used by the dashboard to hide
        /// tabs the host can't satisfy (e.g. no Docker tab on a
        /// k8s-only Pod). Empty for pre-v15 agents — UI treats that
        /// as "show every tab" so legacy agents still work.
        #[serde(default)]
        capabilities: Vec<String>,
    },

    /// Server acknowledging registration
    RegisterAck { agent_id: String },

    /// Ping / Pong for heartbeat (application-level; the WebSocket Ping/Pong
    /// frames are also used by the server to keep proxies from idling out).
    Ping,
    Pong,

    /// Request to list systemd services
    ListServicesRequest,

    /// Response containing systemd services
    ListServicesResponse { services: Vec<ServiceInfo> },

    /// Request to control a service (start, stop, restart)
    ControlServiceRequest { name: String, action: String },

    /// Response to control a service
    ControlServiceResponse {
        name: String,
        success: bool,
        error: Option<String>,
    },

    /// Open a host PTY tagged with `session_id`. Multiple sessions per
    /// host are allowed; each is keyed by the client-chosen id (the
    /// dashboard generates a UUID per terminal tab). The empty string
    /// is reserved for the container-exec session — host shells must
    /// never use it.
    StartTerminalRequest { session_id: String },

    /// Terminal data — bidirectional. From client→agent, `session_id`
    /// selects which PTY the bytes are written into. From agent→client,
    /// `session_id` tags which PTY emitted them so the right xterm
    /// instance can render. `session_id == ""` is the container-exec
    /// session (singleton, scoped by container_id on the start
    /// request).
    TerminalData { session_id: String, data: Vec<u8> },

    /// Tear down the host PTY identified by `session_id`. Closing one
    /// tab on the dashboard sends this with the tab's id; other tabs
    /// against the same agent keep running. Old agents (pre-v14)
    /// ignored the variant entirely and the PTY died on the next WS
    /// disconnect; new agents reap immediately.
    StopTerminalRequest { session_id: String },

    /// Resize the PTY identified by `session_id`. Each xterm instance
    /// resizes independently; sizes do not propagate between tabs.
    TerminalResize {
        session_id: String,
        cols: u16,
        rows: u16,
    },

    /// List pods across every namespace the agent's k8s identity can
    /// see. Only agents with the `"k8s"` capability handle this; the
    /// non-k8s .deb returns `K8sListPodsResponse { error: Some(...) }`.
    K8sListPodsRequest,
    K8sListPodsResponse {
        pods: Vec<K8sPod>,
        error: Option<String>,
    },

    /// Slice-2 read API — same capability gate as K8sListPods. Each
    /// pair returns either a populated list or an `error: Some(_)`,
    /// never both. Empty lists are legitimate (cluster has none).
    K8sListDeploymentsRequest,
    K8sListDeploymentsResponse {
        deployments: Vec<K8sDeployment>,
        error: Option<String>,
    },
    K8sListServicesRequest,
    K8sListServicesResponse {
        services: Vec<K8sService>,
        error: Option<String>,
    },
    K8sListIngressesRequest,
    K8sListIngressesResponse {
        ingresses: Vec<K8sIngress>,
        error: Option<String>,
    },
    K8sListPvcsRequest,
    K8sListPvcsResponse {
        pvcs: Vec<K8sPvc>,
        error: Option<String>,
    },
    /// Cluster-wide events sorted newest-first, capped at 200 by the
    /// agent so a busy cluster doesn't blow up the WS frame.
    K8sListEventsRequest,
    K8sListEventsResponse {
        events: Vec<K8sEvent>,
        error: Option<String>,
    },

    /// Describe a single resource — agent fetches the typed object and
    /// serializes it to YAML so the dashboard can render it in a modal
    /// without re-implementing kubectl's section formatter. `kind` is
    /// lowercase: `"pod"` / `"deployment"` / `"service"` / `"ingress"` /
    /// `"pvc"` / `"event"`. `namespace = None` is valid for cluster-
    /// scoped resources but every supported kind is namespaced today.
    K8sDescribeRequest {
        kind: String,
        namespace: Option<String>,
        name: String,
    },
    K8sDescribeResponse {
        kind: String,
        namespace: Option<String>,
        name: String,
        yaml: String,
        error: Option<String>,
    },

    /// Stream a pod's logs. Mirrors the journal-stream pattern:
    /// `stream_id` is minted by the dashboard so multiple concurrent
    /// log views from the same operator don't collide on a single
    /// host. `container` is required when the pod has more than one
    /// container; otherwise the agent picks the first.
    K8sLogsRequest {
        stream_id: String,
        namespace: String,
        pod_name: String,
        container: Option<String>,
        /// Lines of history to backfill before live tailing. 0 = none.
        #[serde(default)]
        tail_lines: i64,
        #[serde(default)]
        follow: bool,
    },
    /// Batched log lines, flushed every 100 lines or 250 ms.
    K8sLogsChunk {
        stream_id: String,
        lines: Vec<String>,
    },
    /// Operator cancels the stream. Aborts the agent-side task; no
    /// further chunks or End message will arrive for this stream_id.
    K8sLogsStop { stream_id: String },
    /// Pod exited / container EOF / stream error. Terminal for the
    /// stream_id — the agent will not send anything more on it.
    K8sLogsEnd {
        stream_id: String,
        error: Option<String>,
    },

    /// Open a `kubectl exec`-equivalent PTY into a container. The
    /// agent attaches to the pod's container via kube-rs's
    /// AttachedProcess and re-uses the v14 TerminalData /
    /// TerminalResize / StopTerminalRequest variants for the byte
    /// stream — `session_id` is the same wire-level key, so the
    /// dashboard's existing xterm.js plumbing works unchanged.
    ///
    /// `command` defaults to `/bin/sh` when empty. `container` is
    /// required when the pod has more than one container.
    K8sExecRequest {
        session_id: String,
        namespace: String,
        pod_name: String,
        container: Option<String>,
        #[serde(default)]
        command: Vec<String>,
    },
    /// Synchronous ack for the open. `success=false` means the agent
    /// could not attach (e.g. pod not running, container picker
    /// missing on a multi-container pod). Subsequent TerminalData
    /// frames flow under the same session_id.
    K8sExecResponse {
        session_id: String,
        success: bool,
        error: Option<String>,
    },

    /// Request to read a configuration file
    ReadConfigRequest { path: String },

    /// Response containing file content
    ReadConfigResponse {
        path: String,
        content: String,
        error: Option<String>,
    },

    /// Request to write a configuration file
    WriteConfigRequest { path: String, content: String },

    /// Response to write config
    WriteConfigResponse {
        path: String,
        success: bool,
        error: Option<String>,
    },

    /// Request a snapshot of system stats (uptime, load, memory, disk, …).
    /// Introduced in protocol_version 2; older agents simply ignore it
    /// because they don't recognise the variant when deserialising.
    SystemStatsRequest,

    /// Snapshot of system-wide resource usage. All sizes in kilobytes
    /// (KiB, 1024 bytes) to match /proc/meminfo and `df -P`.
    SystemStatsResponse {
        hostname: String,
        kernel: String,
        uptime_secs: u64,
        cpu_count: u32,
        load_1: f32,
        load_5: f32,
        load_15: f32,
        mem_total_kb: u64,
        mem_available_kb: u64,
        swap_total_kb: u64,
        swap_free_kb: u64,
        root_disk_total_kb: u64,
        root_disk_used_kb: u64,
    },

    /// Request a list of Docker containers + the agent's swarm role.
    /// Introduced in protocol_version 3.
    DockerListRequest,

    /// Container list (running + stopped) for the agent's local engine.
    /// `available = false` when the agent can't reach `docker`.
    DockerListResponse {
        available: bool,
        swarm_role: SwarmRole,
        containers: Vec<DockerContainer>,
        error: Option<String>,
    },

    /// Request swarm-wide info. Only meaningful on a manager node.
    /// Introduced in protocol_version 3.
    SwarmListRequest,

    /// Swarm-wide services + node list. Empty (with `available=false` /
    /// `is_manager=false`) if the agent isn't a manager.
    SwarmListResponse {
        available: bool,
        is_manager: bool,
        services: Vec<SwarmService>,
        nodes: Vec<SwarmNode>,
        error: Option<String>,
    },

    /// Run a management action against a swarm service (scale, force
    /// update, remove). Only meaningful on a manager. Introduced in
    /// protocol_version 4.
    SwarmServiceActionRequest { name: String, action: SwarmAction },
    SwarmServiceActionResponse {
        name: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    /// Apt update management — introduced in protocol_version 4.
    /// Cheap snapshot of the upgrade picture without running apt-get update.
    AptStatusRequest,
    AptStatusResponse {
        available: bool,
        upgradable: Vec<AptUpgradable>,
        last_update_secs: u64,
        error: Option<String>,
    },

    /// Equivalent to `apt-get update`. Captures stdout/stderr in `log`
    /// for the dashboard to display.
    AptRefreshRequest,
    AptRefreshResponse {
        success: bool,
        log: String,
        error: Option<String>,
    },

    /// Apply upgrades. `package == None` upgrades all upgradable packages
    /// (`apt-get -y upgrade`); a specific package runs
    /// `apt-get -y install --only-upgrade <package>`.
    AptUpgradeRequest { package: Option<String> },
    AptUpgradeResponse {
        package: Option<String>,
        success: bool,
        log: String,
        error: Option<String>,
    },

    /// Create a standalone Docker container on the agent's local engine.
    /// Introduced in protocol_version 5.
    DockerCreateContainerRequest { spec: ContainerSpec },
    DockerCreateContainerResponse {
        success: bool,
        container_id: Option<String>,
        log: String,
        error: Option<String>,
    },

    /// Create a swarm service. Only valid on a manager. Introduced in
    /// protocol_version 5.
    SwarmCreateServiceRequest { spec: ServiceSpec },
    SwarmCreateServiceResponse {
        success: bool,
        service_id: Option<String>,
        log: String,
        error: Option<String>,
    },

    /// Run a lifecycle action against a local docker container.
    /// Introduced in protocol_version 6.
    DockerContainerActionRequest {
        id: String,
        action: DockerContainerAction,
    },
    DockerContainerActionResponse {
        id: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    /// Subscribe to a container's stdout/stderr. The agent will keep
    /// emitting `DockerLogsChunk` until the UI sends `DockerLogsStop`
    /// for the same container_id, the container exits, or the
    /// websocket drops. Introduced in protocol_version 6.
    DockerLogsRequest {
        container_id: String,
        #[serde(default = "default_tail")]
        tail: u32,
        #[serde(default = "default_true")]
        follow: bool,
    },
    DockerLogsChunk {
        container_id: String,
        data: String,
    },
    DockerLogsStop {
        container_id: String,
    },
    /// Sent once at the end of a docker-logs stream. `error == None`
    /// means the stream ended naturally (container stopped or operator
    /// requested DockerLogsStop).
    DockerLogsEnd {
        container_id: String,
        error: Option<String>,
    },

    /// Stream `journalctl -fu <unit>` lines back to the dashboard. Same
    /// lifecycle as DockerLogs*. Introduced in protocol_version 7.
    JournalLogsRequest {
        unit: String,
        #[serde(default = "default_tail")]
        lines: u32,
        #[serde(default = "default_true")]
        follow: bool,
    },
    JournalLogsChunk {
        unit: String,
        data: String,
    },
    JournalLogsStop {
        unit: String,
    },
    JournalLogsEnd {
        unit: String,
        error: Option<String>,
    },

    /// Free-form journalctl streamer — not tied to a specific systemd
    /// unit. Operators filter by units (zero or many), priority,
    /// since-timestamp, SYSLOG_IDENTIFIER, and grep regex. Lines are
    /// batched into chunks (~100 lines or 250 ms, whichever first)
    /// to keep WS overhead low on busy hosts.
    ///
    /// Old agents (pre-this-version) silently ignore the request;
    /// the dashboard times out gracefully.
    JournalStreamRequest {
        /// Stable id minted by the dashboard so multiple concurrent
        /// journal streams from one UI don't collide.
        stream_id: String,
        #[serde(default)]
        units: Vec<String>,
        /// emerg | alert | crit | err | warning | notice | info | debug
        #[serde(default)]
        priority: Option<String>,
        /// `--since` value: "1h ago", "2024-01-01 09:00", etc.
        #[serde(default)]
        since: Option<String>,
        /// Regex grep on the message (`journalctl -g`).
        #[serde(default)]
        grep: Option<String>,
        /// SYSLOG_IDENTIFIER (`-t`).
        #[serde(default)]
        identifier: Option<String>,
        #[serde(default = "default_tail")]
        lines: u32,
        #[serde(default = "default_true")]
        follow: bool,
    },
    JournalStreamChunk {
        stream_id: String,
        /// Up to ~100 lines per chunk to amortize WS framing cost.
        lines: Vec<String>,
    },
    JournalStreamStop {
        stream_id: String,
    },
    JournalStreamEnd {
        stream_id: String,
        error: Option<String>,
    },

    /// Inspect a swarm service: `docker service ps` (replicas + tasks) plus
    /// `docker service inspect` (env, mounts, networks, image digest, etc.).
    /// Manager-only. Introduced in protocol_version 7.
    SwarmServiceInspectRequest {
        name: String,
    },
    SwarmServiceInspectResponse {
        name: String,
        success: bool,
        tasks: Vec<SwarmTask>,
        spec: Option<SwarmServiceSpecSummary>,
        log: String,
        error: Option<String>,
    },

    /// Deploy a compose stack via `docker stack deploy --compose-file -`,
    /// passing the YAML on stdin. Introduced in protocol_version 7.
    SwarmStackDeployRequest {
        stack_name: String,
        compose_yaml: String,
        prune: bool,
    },
    SwarmStackDeployResponse {
        stack_name: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    /// Server pushing the agent's full probe set. Replaces whatever the
    /// agent had previously. Sent on agent register and on every
    /// CRUD operation in `/api/health-probes`. Introduced in
    /// protocol_version 8.
    HealthProbeSyncRequest {
        probes: Vec<HealthProbeSpec>,
    },
    /// Agent reporting probe state changes back to the server. The
    /// agent batches results per probe whenever a probe transitions
    /// state (or completes its first run after sync). Introduced in
    /// protocol_version 8.
    HealthProbeReport {
        results: Vec<HealthProbeResult>,
    },

    /// Server requesting the agent run a backup. The agent archives
    /// the listed paths and writes them to `dest`. Introduced in
    /// protocol_version 9; `mode` added in protocol_version 10
    /// (defaults to `tar` for older callers).
    BackupRunRequest {
        /// Server-side `backup_jobs.id` echoed back in the response so
        /// the server can attribute results to a specific job.
        id: String,
        name: String,
        paths: Vec<String>,
        /// Destination URI. Supported schemes:
        ///   - bare path or `file:///...` — local filesystem
        ///   - `s3://bucket/prefix` — uploaded via the host's `aws` CLI
        dest: String,
        #[serde(default)]
        mode: BackupMode,
    },
    BackupRunResponse {
        id: String,
        name: String,
        success: bool,
        /// URI of the produced archive (local path or s3://...).
        /// Empty on failure.
        archive_path: String,
        /// Bytes written to the archive. 0 on failure.
        bytes: u64,
        log: String,
        error: Option<String>,
    },

    /// Server asking the agent to enumerate existing archives at the
    /// job's destination. Introduced in protocol_version 10.
    BackupListArchivesRequest {
        id: String,
        name: String,
        dest: String,
    },
    BackupListArchivesResponse {
        id: String,
        success: bool,
        archives: Vec<BackupArchive>,
        error: Option<String>,
    },

    /// Server requesting the agent restore a named archive to
    /// `dest_root` (operator-supplied; the agent never auto-extracts
    /// in place). Introduced in protocol_version 10.
    BackupRestoreRequest {
        id: String,
        archive_uri: String,
        dest_root: String,
    },
    BackupRestoreResponse {
        id: String,
        archive_uri: String,
        dest_root: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    /// Snapshot list of docker images on the agent's local engine.
    /// Introduced in protocol_version 11.
    DockerImageListRequest,
    DockerImageListResponse {
        available: bool,
        images: Vec<DockerImage>,
        error: Option<String>,
    },
    /// Remove a docker image. `force=true` adds `--force`.
    /// Introduced in protocol_version 11.
    DockerImageRemoveRequest {
        id: String,
        #[serde(default)]
        force: bool,
    },
    DockerImageRemoveResponse {
        id: String,
        success: bool,
        log: String,
        error: Option<String>,
    },
    /// Pull a docker image by reference (e.g. `nginx:1.27` or
    /// `ghcr.io/owner/image@sha256:…`). The agent runs `docker pull`
    /// synchronously; the response carries the full pull log.
    /// Introduced in protocol_version 11.
    DockerImagePullRequest {
        reference: String,
    },
    DockerImagePullResponse {
        reference: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    // ----- Networks (v12) -----
    DockerNetworkListRequest,
    DockerNetworkListResponse {
        available: bool,
        networks: Vec<DockerNetwork>,
        error: Option<String>,
    },
    DockerNetworkInspectRequest {
        id: String,
    },
    DockerNetworkInspectResponse {
        id: String,
        success: bool,
        /// Raw `docker network inspect <id>` output (single-element JSON array).
        json: String,
        error: Option<String>,
    },
    DockerNetworkCreateRequest {
        name: String,
        driver: String,
        #[serde(default)]
        subnet: Option<String>,
        #[serde(default)]
        attachable: bool,
        #[serde(default)]
        internal: bool,
    },
    DockerNetworkCreateResponse {
        name: String,
        success: bool,
        id: Option<String>,
        log: String,
        error: Option<String>,
    },
    DockerNetworkRemoveRequest {
        id: String,
    },
    DockerNetworkRemoveResponse {
        id: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    // ----- Volumes (v12) -----
    DockerVolumeListRequest,
    DockerVolumeListResponse {
        available: bool,
        volumes: Vec<DockerVolume>,
        error: Option<String>,
    },
    DockerVolumeInspectRequest {
        name: String,
    },
    DockerVolumeInspectResponse {
        name: String,
        success: bool,
        json: String,
        error: Option<String>,
    },
    DockerVolumeRemoveRequest {
        name: String,
        #[serde(default)]
        force: bool,
    },
    DockerVolumeRemoveResponse {
        name: String,
        success: bool,
        log: String,
        error: Option<String>,
    },
    DockerVolumePruneRequest,
    DockerVolumePruneResponse {
        success: bool,
        removed: Vec<String>,
        space_reclaimed_bytes: u64,
        log: String,
        error: Option<String>,
    },

    // ----- Swarm stacks (v12, manager-only) -----
    SwarmStackListRequest,
    SwarmStackListResponse {
        available: bool,
        is_manager: bool,
        stacks: Vec<SwarmStack>,
        error: Option<String>,
    },
    SwarmStackInspectRequest {
        name: String,
    },
    SwarmStackInspectResponse {
        name: String,
        success: bool,
        services: Vec<SwarmService>,
        tasks: Vec<SwarmTask>,
        log: String,
        error: Option<String>,
    },
    SwarmStackRemoveRequest {
        name: String,
    },
    SwarmStackRemoveResponse {
        name: String,
        success: bool,
        log: String,
        error: Option<String>,
    },

    // ----- System prune (v13) -----
    /// `dry_run=true` returns what *would* be pruned via `docker system df -v`
    /// + a synthesised summary; `dry_run=false` runs `docker system prune -af`.
    /// No background loops — only fires when the UI asks.
    DockerSystemPruneRequest {
        #[serde(default)]
        dry_run: bool,
        /// `--volumes` flag passes through. Off by default because
        /// pruning volumes is harder to reverse than images/containers.
        #[serde(default)]
        prune_volumes: bool,
    },
    DockerSystemPruneResponse {
        dry_run: bool,
        success: bool,
        /// Bytes the operator would reclaim (preview) or did reclaim (apply).
        reclaimed_bytes: u64,
        /// Stopped container ids.
        containers_removed: Vec<String>,
        /// Image ids.
        images_removed: Vec<String>,
        /// Network ids.
        networks_removed: Vec<String>,
        /// Volume names (only if prune_volumes was true).
        volumes_removed: Vec<String>,
        log: String,
        error: Option<String>,
    },

    // ----- Container stats snapshot (v13) -----
    /// Single snapshot — agent runs `docker stats --no-stream --format json`,
    /// returns once. UI handles cadence (visibility-aware polling).
    DockerStatsRequest,
    DockerStatsResponse {
        available: bool,
        snapshots: Vec<DockerContainerStats>,
        error: Option<String>,
    },

    // ----- Container exec session (v13) -----
    /// Open `docker exec -it <container_id> <shell>` over a PTY. Reuses the
    /// existing TerminalData/TerminalResize message types for input/output;
    /// the agent maintains a separate exec-session slot from the host
    /// terminal, with one exec session per agent at a time.
    DockerExecStartRequest {
        container_id: String,
        /// "sh", "bash", "ash", etc. Agent picks `sh` if empty.
        #[serde(default)]
        shell: String,
    },
    DockerExecStartResponse {
        container_id: String,
        success: bool,
        error: Option<String>,
    },
    /// Stop any active exec session. The PTY is killed and the docker exec
    /// child reaped. Idempotent — no error if no session was open.
    DockerExecStopRequest,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DockerContainerStats {
    pub id: String,
    pub name: String,
    /// CPU % as docker reports it (0.0 - n*100 on multi-CPU hosts).
    pub cpu_percent: f32,
    /// Memory in bytes / limit in bytes.
    pub mem_bytes: u64,
    pub mem_limit_bytes: u64,
    /// Network I/O cumulative since container start (rx, tx).
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    /// Block I/O cumulative since container start (read, write).
    pub blk_read_bytes: u64,
    pub blk_write_bytes: u64,
    /// Number of process IDs / threads.
    pub pids: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DockerNetwork {
    pub id: String,
    pub name: String,
    pub driver: String,
    /// "local" or "swarm".
    pub scope: String,
    pub created: String,
    pub ipv6: bool,
    pub internal: bool,
    pub attachable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DockerVolume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    /// Bytes. `0` if docker doesn't report it (e.g. external drivers).
    pub size_bytes: u64,
    pub created: String,
    /// "label1=v1,label2=v2" — opaque to the agent, raw from docker.
    pub labels: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwarmStack {
    pub name: String,
    pub services: u32,
    pub orchestrator: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DockerImage {
    /// Short image id (no `sha256:` prefix), e.g. `7d5c1f4d4`.
    pub id: String,
    /// `docker images --format json` returns the same Repository/Tag
    /// fields as the columnar output. `<none>` for dangling layers.
    pub repository: String,
    pub tag: String,
    /// Bytes. `0` if docker doesn't report it.
    pub size_bytes: u64,
    /// Human-friendly created-at string from docker.
    pub created: String,
}

/// How the agent should produce the archive. v1 only ships `tar`;
/// `restic` is reserved for v3 and currently returns
/// "not implemented" from the agent.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackupMode {
    #[default]
    Tar,
    Restic,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BackupArchive {
    /// Just the basename (`etc-sysmanager-1777134782.tar.gz`).
    pub name: String,
    /// Full URI suitable for a follow-up `BackupRestoreRequest`.
    pub uri: String,
    pub bytes: u64,
    /// Unix seconds.
    pub mtime: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthProbeKind {
    Http,
    Tcp,
    /// Run a script from /etc/sys-manager/probes.d/<target>. Exit 0 = green.
    /// Introduced in protocol_version 9.
    Exec,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HealthProbeSpec {
    /// Stable id within the agent — used as the dedup key on sync. The
    /// server's row id stringified is fine.
    pub id: String,
    pub name: String,
    pub kind: HealthProbeKind,
    /// HTTP: full URL ("https://example.com/healthz").
    /// TCP:  "host:port".
    pub target: String,
    pub interval_secs: u32,
    pub timeout_secs: u32,
    /// HTTP only: expected status code. `None` means "any 2xx".
    #[serde(default)]
    pub expect_status: Option<u16>,
    /// HTTP only: substring that must appear in the body. `None` skips
    /// body checking.
    #[serde(default)]
    pub expect_body: Option<String>,
    /// Optional per-probe environment variables in `KEY=VALUE` form.
    /// Mainly used by exec-kind probes (e.g. `THRESHOLD=85`); HTTP/TCP
    /// probes ignore it. Introduced in protocol_version 11.
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthProbeState {
    Green,
    Red,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HealthProbeResult {
    pub id: String,
    pub state: HealthProbeState,
    pub latency_ms: u32,
    /// One-line summary for the operator: status code + reason, error
    /// message, or "ok in <N>ms".
    pub detail: String,
    /// Unix seconds when the probe was sampled.
    pub at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwarmTask {
    pub id: String,
    pub name: String,
    pub node: String,
    pub desired_state: String,
    pub current_state: String,
    pub error: String,
    pub image: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SwarmServiceSpecSummary {
    pub image: String,
    pub image_digest: String,
    pub mode: String,
    pub replicas: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
    pub env: Vec<String>,
    pub mounts: Vec<String>,
    pub networks: Vec<String>,
    pub constraints: Vec<String>,
    pub published_ports: Vec<String>,
}

fn default_tail() -> u32 {
    200
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DockerContainerAction {
    Start,
    Stop,
    Restart,
    Remove,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ContainerSpec {
    pub image: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Each entry is "host:container" or "host:container/proto".
    #[serde(default)]
    pub ports: Vec<String>,
    /// Each entry is "KEY=VALUE".
    #[serde(default)]
    pub env: Vec<String>,
    /// Each entry is "host:container" or "named-volume:container[:ro]".
    #[serde(default)]
    pub volumes: Vec<String>,
    /// One of "no" | "always" | "unless-stopped" | "on-failure".
    #[serde(default)]
    pub restart_policy: Option<String>,
    /// Optional command override (shell-split on the agent).
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub network: Option<String>,
    /// Defaults to true so the container is detached from the agent's
    /// stdin/stdout — matching `docker run -d`.
    #[serde(default = "default_true")]
    pub detached: bool,
    /// `--pull always` if true.
    #[serde(default)]
    pub pull: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServiceSpec {
    pub image: String,
    pub name: String,
    /// Only meaningful when mode is "replicated" (the default).
    #[serde(default)]
    pub replicas: Option<u32>,
    /// "replicated" (default) or "global".
    #[serde(default)]
    pub mode: Option<String>,
    /// Each entry is "published:target" or "published:target/proto".
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    /// Each entry is a `--mount` arg ("type=bind,source=...,target=..."
    /// or "src=...,dst=...").
    #[serde(default)]
    pub mounts: Vec<String>,
    /// Each entry is a `--constraint` arg, e.g. "node.role==manager".
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub command: Option<String>,
    /// Each entry is a network name to attach.
    #[serde(default)]
    pub networks: Vec<String>,
    /// "any" | "on-failure" | "none".
    #[serde(default)]
    pub restart_condition: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", content = "value")]
pub enum SwarmAction {
    /// Scale a replicated service to the given number of replicas.
    Scale(u32),
    /// `docker service update --force` — kicks a rolling update without
    /// changing the spec, useful for picking up a new image tag.
    ForceUpdate,
    /// `docker service rm <name>`.
    Remove,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AptUpgradable {
    pub name: String,
    pub current_version: String,
    pub new_version: String,
    pub source: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct K8sDeployment {
    pub namespace: String,
    pub name: String,
    /// `<ready_replicas>/<spec_replicas>`, e.g. `3/3`.
    pub ready: String,
    pub up_to_date: i32,
    pub available: i32,
    pub age_secs: i64,
    /// First container image — full inspection lives behind Describe.
    pub image: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct K8sService {
    pub namespace: String,
    pub name: String,
    /// `ClusterIP` / `NodePort` / `LoadBalancer` / `ExternalName`.
    pub kind: String,
    pub cluster_ip: Option<String>,
    pub external_ips: Vec<String>,
    /// Pre-formatted `port:nodePort/proto` strings, comma-joined upstream.
    pub ports: Vec<String>,
    pub age_secs: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct K8sIngress {
    pub namespace: String,
    pub name: String,
    pub class: Option<String>,
    pub hosts: Vec<String>,
    pub addresses: Vec<String>,
    pub age_secs: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct K8sPvc {
    pub namespace: String,
    pub name: String,
    /// `Bound` / `Pending` / `Lost`.
    pub status: String,
    pub volume_name: Option<String>,
    pub capacity: Option<String>,
    /// Short-form list, e.g. `["RWO"]`, `["RWO", "RWX"]`.
    pub access_modes: Vec<String>,
    pub storage_class: Option<String>,
    pub age_secs: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct K8sEvent {
    pub namespace: String,
    /// `Normal` / `Warning`.
    pub kind: String,
    pub reason: String,
    /// Pre-formatted `kind/name` of the involved object.
    pub object: String,
    pub message: String,
    pub count: i32,
    /// Seconds since `lastTimestamp` (or `eventTime` for newer events).
    pub age_secs: i64,
}

/// One row in the dashboard's k8s Pods table. The agent collapses
/// the kube-apiserver `Pod` object down to the fields the UI renders;
/// the rest stays on the cluster and is fetched via Describe later.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct K8sPod {
    pub namespace: String,
    pub name: String,
    /// `Running` / `Pending` / `Succeeded` / `Failed` / `Unknown`.
    pub phase: String,
    /// `<ready_containers>/<total_containers>`, e.g. `2/3`.
    pub ready: String,
    /// Sum of restartCount across all container statuses.
    pub restarts: u32,
    /// Seconds since `metadata.creationTimestamp`. UI formats as a
    /// human "5m" / "3h" / "2d".
    pub age_secs: i64,
    pub node: Option<String>,
    /// Just the container names — image / cmd lives behind Describe.
    pub containers: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub description: String,
    /// SUB state from systemctl: running, exited, failed, dead, …
    pub status: String,
    /// ACTIVE state from systemctl: active, inactive, failed, activating, …
    pub active_state: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum UiMessage {
    /// UI asking for online agents
    ListAgentsRequest,

    /// Server telling UI about online agents. `capabilities` is keyed
    /// by agent_id and lists what each can serve (`"systemd"`,
    /// `"docker"`, `"swarm"`, `"k8s"`). Pre-v15 agents register
    /// without capabilities and appear in `agents` with no entry in
    /// `capabilities` — UI treats absence as "show every tab".
    ListAgentsResponse {
        agents: Vec<String>,
        #[serde(default)]
        capabilities: std::collections::HashMap<String, Vec<String>>,
    },

    /// UI sending a message to a specific agent
    SendToAgent { agent_id: String, message: Message },

    /// Server forwarding a message from an agent to the UI
    AgentMessage { agent_id: String, message: Message },
}
