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
  | { type: 'SwarmListResponse'; payload: SwarmListPayload };

export type UiMessage =
  | { type: 'ListAgentsRequest' }
  | { type: 'ListAgentsResponse'; payload: { agents: string[] } }
  | { type: 'SendToAgent'; payload: { agent_id: string; message: AgentMessagePayload } }
  | { type: 'AgentMessage'; payload: { agent_id: string; message: AgentMessagePayload } };
