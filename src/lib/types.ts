export type ServiceInfo = {
  name: string;
  description: string;
  status: string;
  active_state: string;
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

export type HealthProbeKind = 'http' | 'tcp';
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
  | { type: 'StartTerminalRequest' }
  | { type: 'TerminalData'; payload: { data: number[] } }
  | { type: 'TerminalResize'; payload: { cols: number; rows: number } }
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
  | { type: 'SwarmServiceInspectRequest'; payload: { name: string } }
  | { type: 'SwarmServiceInspectResponse'; payload: SwarmServiceInspectPayload }
  | { type: 'SwarmStackDeployRequest'; payload: { stack_name: string; compose_yaml: string; prune: boolean } }
  | { type: 'SwarmStackDeployResponse'; payload: SwarmStackDeployResponse }
  | { type: 'HealthProbeSyncRequest'; payload: { probes: unknown[] } }
  | { type: 'HealthProbeReport'; payload: { results: unknown[] } };

export type UiMessage =
  | { type: 'ListAgentsRequest' }
  | { type: 'ListAgentsResponse'; payload: { agents: string[] } }
  | { type: 'SendToAgent'; payload: { agent_id: string; message: AgentMessagePayload } }
  | { type: 'AgentMessage'; payload: { agent_id: string; message: AgentMessagePayload } };
