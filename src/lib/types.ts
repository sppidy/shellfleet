export type ServiceInfo = {
  name: string;
  description: string;
  status: string;
  active_state: string;
};

export type K8sPod = {
  namespace: string;
  name: string;
  phase: string;
  ready: string;
  restarts: number;
  age_secs: number;
  node: string | null;
  containers: string[];
};

export type K8sDeployment = {
  namespace: string;
  name: string;
  ready: string;
  up_to_date: number;
  available: number;
  age_secs: number;
  image: string | null;
};

export type K8sService = {
  namespace: string;
  name: string;
  kind: string;
  cluster_ip: string | null;
  external_ips: string[];
  ports: string[];
  age_secs: number;
};

export type K8sIngress = {
  namespace: string;
  name: string;
  class: string | null;
  hosts: string[];
  addresses: string[];
  age_secs: number;
};

export type K8sPvc = {
  namespace: string;
  name: string;
  status: string;
  volume_name: string | null;
  capacity: string | null;
  access_modes: string[];
  storage_class: string | null;
  age_secs: number;
};

export type K8sEvent = {
  namespace: string;
  kind: string;
  reason: string;
  object: string;
  message: string;
  count: number;
  age_secs: number;
};

export type SystemStatsPayload = {
  hostname: string;
  kernel: string;
  uptime_secs: number;
  cpu_count: number;
  load_1: number;
  load_5: number;
  load_15: number;
  mem_total_kb: number;
  mem_available_kb: number;
  swap_total_kb: number;
  swap_free_kb: number;
  root_disk_total_kb: number;
  root_disk_used_kb: number;
};

export type SwarmRole = 'notinswarm' | 'worker' | 'manager';

export type DockerContainer = {
  id: string;
  names: string;
  image: string;
  state: string;
  status: string;
  ports: string;
};

export type DockerListPayload = {
  available: boolean;
  swarm_role: SwarmRole;
  containers: DockerContainer[];
  error: string | null;
};

export type SwarmService = {
  id: string;
  name: string;
  mode: string;
  replicas: string;
  image: string;
  ports: string;
};

export type SwarmNode = {
  id: string;
  hostname: string;
  status: string;
  availability: string;
  manager_status: string;
  engine_version: string;
};

export type SwarmListPayload = {
  available: boolean;
  is_manager: boolean;
  services: SwarmService[];
  nodes: SwarmNode[];
  error: string | null;
};

export type SwarmAction =
  | { kind: 'Scale'; value: number }
  | { kind: 'ForceUpdate' }
  | { kind: 'Remove' };

export type SwarmServiceActionResponse = {
  name: string;
  success: boolean;
  log: string;
  error: string | null;
};

export type AptUpgradable = {
  name: string;
  current_version: string;
  new_version: string;
  source: string;
};

export type AptStatusPayload = {
  available: boolean;
  upgradable: AptUpgradable[];
  last_update_secs: number;
  error: string | null;
};

export type AptOpResponse = {
  success: boolean;
  log: string;
  error: string | null;
};

export type AptUpgradeResponsePayload = AptOpResponse & {
  package: string | null;
};

export type ContainerSpec = {
  image: string;
  name?: string | null;
  ports?: string[];
  env?: string[];
  volumes?: string[];
  restart_policy?: string | null;
  command?: string | null;
  network?: string | null;
  detached?: boolean;
  pull?: boolean;
};

export type ServiceSpec = {
  image: string;
  name: string;
  replicas?: number | null;
  mode?: string | null;
  ports?: string[];
  env?: string[];
  mounts?: string[];
  constraints?: string[];
  command?: string | null;
  networks?: string[];
  restart_condition?: string | null;
};

export type CreateContainerResponse = {
  success: boolean;
  container_id: string | null;
  log: string;
  error: string | null;
};

export type CreateServiceResponse = {
  success: boolean;
  service_id: string | null;
  log: string;
  error: string | null;
};

export type DockerContainerAction = 'start' | 'stop' | 'restart' | 'remove';

export type DockerContainerActionResponse = {
  id: string;
  success: boolean;
  log: string;
  error: string | null;
};

export type DockerLogsChunkPayload = {
  container_id: string;
  data: string;
};

export type DockerLogsEndPayload = {
  container_id: string;
  error: string | null;
};

export type JournalLogsChunkPayload = {
  unit: string;
  data: string;
};

export type JournalLogsEndPayload = {
  unit: string;
  error: string | null;
};

export type SwarmTask = {
  id: string;
  name: string;
  node: string;
  desired_state: string;
  current_state: string;
  error: string;
  image: string;
};

export type SwarmServiceSpecSummary = {
  image: string;
  image_digest: string;
  mode: string;
  replicas: number | null;
  created_at: string;
  updated_at: string;
  env: string[];
  mounts: string[];
  networks: string[];
  constraints: string[];
  published_ports: string[];
};

export type SwarmServiceInspectPayload = {
  name: string;
  success: boolean;
  tasks: SwarmTask[];
  spec: SwarmServiceSpecSummary | null;
  log: string;
  error: string | null;
};

export type SwarmStackDeployResponse = {
  stack_name: string;
  success: boolean;
  log: string;
  error: string | null;
};

export type Features = {
  backups_enabled: boolean;
};

export type Notification = {
  id: number;
  kind: string;
  agent_id: string | null;
  level: 'info' | 'warn' | 'error' | string;
  title: string;
  body: string | null;
  created_at: number;
  read_at: number | null;
};

export type BackupJob = {
  id: number;
  agent_id: string;
  name: string;
  paths: string[];
  dest: string;
  cron_expr: string | null;
  enabled: boolean;
  mode: string;
  last_run_at: number;
  last_status: string | null;
  last_archive_path: string | null;
  last_bytes: number | null;
  last_log: string | null;
  updated_at: number;
  next_run_at: number | null;
};

export type BackupArchive = {
  name: string;
  uri: string;
  bytes: number;
  mtime: number;
};

export type BackupRestoreResponse = {
  success: boolean;
  log: string;
  error: string | null;
};

export type LabelsResponse = {
  by_agent: Record<string, string[]>;
  by_label: Record<string, string[]>;
};

export type FanOutKind = 'apt-status' | 'apt-upgrade' | 'docker-list';

export type FanOutRun = {
  id: number;
  kind: string;
  payload: string | null;
  started_at: number;
  actor: string | null;
};

export type FanOutResult = {
  run_id: number;
  agent_id: string;
  status: 'pending' | 'success' | 'failed' | 'offline' | string;
  detail: string | null;
  finished_at: number | null;
};

export type FanOutRunDetail = {
  run: FanOutRun;
  results: FanOutResult[];
};

export type HealthProbeKind = 'http' | 'tcp' | 'exec';
export type HealthProbeState = 'green' | 'red';

export type HealthProbe = {
  id: number;
  agent_id: string;
  name: string;
  kind: HealthProbeKind;
  target: string;
  interval_secs: number;
  timeout_secs: number;
  expect_status: number | null;
  expect_body: string | null;
  enabled: boolean;
  last_run_at: number;
  last_state: string | null;
  last_latency_ms: number | null;
  last_detail: string | null;
  updated_at: number;
  env: string[];
};

export type ProbeLibraryEntry = {
  script: string;
  title: string;
  description: string;
  default_env: { key: string; value: string; description: string }[];
  interval_secs: number;
  timeout_secs: number;
};

export type DockerImage = {
  id: string;
  repository: string;
  tag: string;
  size_bytes: number;
  created: string;
};

export type DockerNetwork = {
  id: string;
  name: string;
  driver: string;
  scope: string;
  created: string;
  ipv6: boolean;
  internal: boolean;
  attachable: boolean;
};

export type DockerVolume = {
  name: string;
  driver: string;
  mountpoint: string;
  size_bytes: number;
  created: string;
  labels: string;
};

export type SwarmStackRow = {
  name: string;
  services: number;
  orchestrator: string;
};

export type DockerContainerStats = {
  id: string;
  name: string;
  cpu_percent: number;
  mem_bytes: number;
  mem_limit_bytes: number;
  net_rx_bytes: number;
  net_tx_bytes: number;
  blk_read_bytes: number;
  blk_write_bytes: number;
  pids: number;
};

export type DockerSystemPrunePayload = {
  dry_run: boolean;
  success: boolean;
  reclaimed_bytes: number;
  containers_removed: string[];
  images_removed: string[];
  networks_removed: string[];
  volumes_removed: string[];
  log: string;
  error: string | null;
};

export type HealthSnapshotRow = {
  agent_id: string;
  total: number;
  green: number;
  red: number;
  unknown: number;
};

export type UpdateWindow = {
  agent_id: string;
  cron_expr: string;
  enabled: boolean;
  last_run_at: number;
  last_status: string | null;
  last_log: string | null;
  updated_at: number;
  next_run_at: number | null;
};

export type AuditRow = {
  id: number;
  ts: number;
  actor: string | null;
  agent_id: string | null;
  kind: string;
  ok: number;
  detail: string | null;
};

export type AgentMessagePayload =
  | { type: 'Register'; payload: { hostname: string; protocol_version?: number } }
  | { type: 'RegisterAck'; payload: { agent_id: string } }
  | { type: 'Ping' }
  | { type: 'Pong' }
  | { type: 'ListServicesRequest' }
  | { type: 'ListServicesResponse'; payload: { services: ServiceInfo[] } }
  | { type: 'ControlServiceRequest'; payload: { name: string; action: string } }
  | { type: 'ControlServiceResponse'; payload: { name: string; success: boolean; error: string | null } }
  | { type: 'K8sListPodsRequest' }
  | { type: 'K8sListPodsResponse'; payload: { pods: K8sPod[]; error: string | null } }
  | { type: 'K8sListDeploymentsRequest' }
  | {
      type: 'K8sListDeploymentsResponse';
      payload: { deployments: K8sDeployment[]; error: string | null };
    }
  | { type: 'K8sListServicesRequest' }
  | {
      type: 'K8sListServicesResponse';
      payload: { services: K8sService[]; error: string | null };
    }
  | { type: 'K8sListIngressesRequest' }
  | {
      type: 'K8sListIngressesResponse';
      payload: { ingresses: K8sIngress[]; error: string | null };
    }
  | { type: 'K8sListPvcsRequest' }
  | {
      type: 'K8sListPvcsResponse';
      payload: { pvcs: K8sPvc[]; error: string | null };
    }
  | { type: 'K8sListEventsRequest' }
  | {
      type: 'K8sListEventsResponse';
      payload: { events: K8sEvent[]; error: string | null };
    }
  | {
      type: 'K8sDescribeRequest';
      payload: { kind: string; namespace: string | null; name: string };
    }
  | {
      type: 'K8sDescribeResponse';
      payload: {
        kind: string;
        namespace: string | null;
        name: string;
        yaml: string;
        error: string | null;
      };
    }
  | { type: 'StartTerminalRequest'; payload: { session_id: string } }
  | { type: 'TerminalData'; payload: { session_id: string; data: number[] } }
  | { type: 'TerminalResize'; payload: { session_id: string; cols: number; rows: number } }
  | { type: 'ReadConfigRequest'; payload: { path: string } }
  | { type: 'ReadConfigResponse'; payload: { path: string; content: string; error: string | null } }
  | { type: 'WriteConfigRequest'; payload: { path: string; content: string } }
  | { type: 'WriteConfigResponse'; payload: { path: string; success: boolean; error: string | null } }
  | { type: 'SystemStatsRequest' }
  | { type: 'SystemStatsResponse'; payload: SystemStatsPayload }
  | { type: 'DockerListRequest' }
  | { type: 'DockerListResponse'; payload: DockerListPayload }
  | { type: 'SwarmListRequest' }
  | { type: 'SwarmListResponse'; payload: SwarmListPayload }
  | { type: 'SwarmServiceActionRequest'; payload: { name: string; action: SwarmAction } }
  | { type: 'SwarmServiceActionResponse'; payload: SwarmServiceActionResponse }
  | { type: 'AptStatusRequest' }
  | { type: 'AptStatusResponse'; payload: AptStatusPayload }
  | { type: 'AptRefreshRequest' }
  | { type: 'AptRefreshResponse'; payload: AptOpResponse }
  | { type: 'AptUpgradeRequest'; payload: { package: string | null } }
  | { type: 'AptUpgradeResponse'; payload: AptUpgradeResponsePayload }
  | { type: 'DockerCreateContainerRequest'; payload: { spec: ContainerSpec } }
  | { type: 'DockerCreateContainerResponse'; payload: CreateContainerResponse }
  | { type: 'SwarmCreateServiceRequest'; payload: { spec: ServiceSpec } }
  | { type: 'SwarmCreateServiceResponse'; payload: CreateServiceResponse }
  | { type: 'DockerContainerActionRequest'; payload: { id: string; action: DockerContainerAction } }
  | { type: 'DockerContainerActionResponse'; payload: DockerContainerActionResponse }
  | { type: 'DockerLogsRequest'; payload: { container_id: string; tail?: number; follow?: boolean } }
  | { type: 'DockerLogsChunk'; payload: DockerLogsChunkPayload }
  | { type: 'DockerLogsStop'; payload: { container_id: string } }
  | { type: 'DockerLogsEnd'; payload: DockerLogsEndPayload }
  | { type: 'JournalLogsRequest'; payload: { unit: string; lines?: number; follow?: boolean } }
  | { type: 'JournalLogsChunk'; payload: JournalLogsChunkPayload }
  | { type: 'JournalLogsStop'; payload: { unit: string } }
  | { type: 'JournalLogsEnd'; payload: JournalLogsEndPayload }
  | {
      type: 'JournalStreamRequest';
      payload: {
        stream_id: string;
        units?: string[];
        priority?: string | null;
        since?: string | null;
        grep?: string | null;
        identifier?: string | null;
        lines?: number;
        follow?: boolean;
      };
    }
  | { type: 'JournalStreamChunk'; payload: { stream_id: string; lines: string[] } }
  | { type: 'JournalStreamStop'; payload: { stream_id: string } }
  | { type: 'JournalStreamEnd'; payload: { stream_id: string; error: string | null } }
  | { type: 'StopTerminalRequest'; payload: { session_id: string } }
  | { type: 'SwarmServiceInspectRequest'; payload: { name: string } }
  | { type: 'SwarmServiceInspectResponse'; payload: SwarmServiceInspectPayload }
  | { type: 'SwarmStackDeployRequest'; payload: { stack_name: string; compose_yaml: string; prune: boolean } }
  | { type: 'SwarmStackDeployResponse'; payload: SwarmStackDeployResponse }
  | { type: 'HealthProbeSyncRequest'; payload: { probes: unknown[] } }
  | { type: 'HealthProbeReport'; payload: { results: unknown[] } }
  | { type: 'DockerImageListRequest' }
  | { type: 'DockerImageListResponse'; payload: { available: boolean; images: DockerImage[]; error: string | null } }
  | { type: 'DockerImageRemoveRequest'; payload: { id: string; force: boolean } }
  | { type: 'DockerImageRemoveResponse'; payload: { id: string; success: boolean; log: string; error: string | null } }
  | { type: 'DockerImagePullRequest'; payload: { reference: string } }
  | { type: 'DockerImagePullResponse'; payload: { reference: string; success: boolean; log: string; error: string | null } }
  | { type: 'DockerNetworkListRequest' }
  | { type: 'DockerNetworkListResponse'; payload: { available: boolean; networks: DockerNetwork[]; error: string | null } }
  | { type: 'DockerNetworkInspectRequest'; payload: { id: string } }
  | { type: 'DockerNetworkInspectResponse'; payload: { id: string; success: boolean; json: string; error: string | null } }
  | { type: 'DockerNetworkCreateRequest'; payload: { name: string; driver: string; subnet?: string | null; attachable?: boolean; internal?: boolean } }
  | { type: 'DockerNetworkCreateResponse'; payload: { name: string; success: boolean; id: string | null; log: string; error: string | null } }
  | { type: 'DockerNetworkRemoveRequest'; payload: { id: string } }
  | { type: 'DockerNetworkRemoveResponse'; payload: { id: string; success: boolean; log: string; error: string | null } }
  | { type: 'DockerVolumeListRequest' }
  | { type: 'DockerVolumeListResponse'; payload: { available: boolean; volumes: DockerVolume[]; error: string | null } }
  | { type: 'DockerVolumeInspectRequest'; payload: { name: string } }
  | { type: 'DockerVolumeInspectResponse'; payload: { name: string; success: boolean; json: string; error: string | null } }
  | { type: 'DockerVolumeRemoveRequest'; payload: { name: string; force: boolean } }
  | { type: 'DockerVolumeRemoveResponse'; payload: { name: string; success: boolean; log: string; error: string | null } }
  | { type: 'DockerVolumePruneRequest' }
  | { type: 'DockerVolumePruneResponse'; payload: { success: boolean; removed: string[]; space_reclaimed_bytes: number; log: string; error: string | null } }
  | { type: 'SwarmStackListRequest' }
  | { type: 'SwarmStackListResponse'; payload: { available: boolean; is_manager: boolean; stacks: SwarmStackRow[]; error: string | null } }
  | { type: 'SwarmStackInspectRequest'; payload: { name: string } }
  | { type: 'SwarmStackInspectResponse'; payload: { name: string; success: boolean; services: SwarmService[]; tasks: SwarmTask[]; log: string; error: string | null } }
  | { type: 'SwarmStackRemoveRequest'; payload: { name: string } }
  | { type: 'SwarmStackRemoveResponse'; payload: { name: string; success: boolean; log: string; error: string | null } }
  | { type: 'DockerSystemPruneRequest'; payload: { dry_run: boolean; prune_volumes: boolean } }
  | { type: 'DockerSystemPruneResponse'; payload: DockerSystemPrunePayload }
  | { type: 'DockerStatsRequest' }
  | { type: 'DockerStatsResponse'; payload: { available: boolean; snapshots: DockerContainerStats[]; error: string | null } }
  | { type: 'DockerExecStartRequest'; payload: { container_id: string; shell: string } }
  | { type: 'DockerExecStartResponse'; payload: { container_id: string; success: boolean; error: string | null } }
  | { type: 'DockerExecStopRequest' };

export type UiMessage =
  | { type: 'ListAgentsRequest' }
  | {
      type: 'ListAgentsResponse';
      payload: {
        agents: string[];
        // agent_id → capability list reported on Register. Pre-v15 agents
        // appear in `agents` with no entry here; treat absence as
        // "show every tab" so legacy hosts still work.
        capabilities?: Record<string, string[]>;
      };
    }
  | { type: 'SendToAgent'; payload: { agent_id: string; message: AgentMessagePayload } }
  | { type: 'AgentMessage'; payload: { agent_id: string; message: AgentMessagePayload } };
