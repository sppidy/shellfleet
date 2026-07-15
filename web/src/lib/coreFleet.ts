import { apiFetch } from './api';
import type {
  DockerListPayload,
  ServiceInfo,
  SwarmListPayload,
  SystemStatsPayload,
} from './types';

export type CoreLiveStatus = 'connecting' | 'live' | 'degraded';
export type ConnectionStatus = 'online' | 'offline';

export type SnapshotValue = { observed_at: number; value: unknown };
export type FleetHost = {
  agent_id: string;
  hostname: string;
  status: ConnectionStatus;
  protocol_version: number;
  capabilities: string[];
  metadata: Record<string, string>;
  first_seen_at: number;
  last_seen_at: number;
  disconnected_at: number | null;
  system: SnapshotValue | null;
  services: SnapshotValue | null;
  docker: SnapshotValue | null;
  swarm: SnapshotValue | null;
};
export type FleetResponse = {
  generated_at: number;
  offline_after_seconds: number;
  hosts: FleetHost[];
};
export type CoreAgentSnapshot = {
  agentId: string;
  hostname: string;
  status: ConnectionStatus;
  lastSeenAt: number;
  stats?: SystemStatsPayload;
  services?: ServiceInfo[];
  docker?: DockerListPayload;
  swarm?: SwarmListPayload;
};

export class FleetApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
  ) {
    super(`fleet request failed: ${status} ${code}`);
    this.name = 'FleetApiError';
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return isObject(value) && Object.values(value).every((item) => typeof item === 'string');
}

function isSnapshot(value: unknown): value is SnapshotValue | null {
  return value === null || (isObject(value) && typeof value.observed_at === 'number' && 'value' in value);
}

function assertFleet(value: unknown): asserts value is FleetResponse {
  if (
    !isObject(value) ||
    typeof value.generated_at !== 'number' ||
    typeof value.offline_after_seconds !== 'number' ||
    !Array.isArray(value.hosts)
  ) {
    throw new FleetApiError(502, 'invalid_fleet_payload');
  }
  for (const host of value.hosts) {
    if (
      !isObject(host) ||
      typeof host.agent_id !== 'string' ||
      typeof host.hostname !== 'string' ||
      (host.status !== 'online' && host.status !== 'offline') ||
      typeof host.protocol_version !== 'number' ||
      !Array.isArray(host.capabilities) ||
      !host.capabilities.every((item) => typeof item === 'string') ||
      !isStringRecord(host.metadata) ||
      typeof host.first_seen_at !== 'number' ||
      typeof host.last_seen_at !== 'number' ||
      (host.disconnected_at !== null && typeof host.disconnected_at !== 'number') ||
      !isSnapshot(host.system) ||
      !isSnapshot(host.services) ||
      !isSnapshot(host.docker) ||
      !isSnapshot(host.swarm)
    ) {
      throw new FleetApiError(502, 'invalid_host_payload');
    }
  }
}

export async function fetchFleet(signal?: AbortSignal): Promise<FleetResponse> {
  const response = await apiFetch('/api/core/v1/fleet', { signal });
  if (!response.ok) throw new FleetApiError(response.status, 'fleet_unavailable');
  const value: unknown = await response.json();
  assertFleet(value);
  return value;
}

function payload<T>(
  host: FleetHost,
  slot: keyof Pick<FleetHost, 'system' | 'services' | 'docker' | 'swarm'>,
  expected: string,
): T | undefined {
  const snapshot = host[slot];
  if (
    !snapshot ||
    !isObject(snapshot.value) ||
    snapshot.value.type !== expected ||
    !isObject(snapshot.value.payload)
  ) {
    if (snapshot) {
      console.error('[shellfleet] malformed durable snapshot', {
        agent_id: host.agent_id,
        slot,
        observed_at: snapshot.observed_at,
      });
    }
    return undefined;
  }
  return snapshot.value.payload as T;
}

export function snapshotsByAgent(hosts: FleetHost[]): Record<string, CoreAgentSnapshot> {
  return Object.fromEntries(
    hosts.map((host) => [
      host.agent_id,
      {
        agentId: host.agent_id,
        hostname: host.hostname,
        status: host.status,
        lastSeenAt: host.last_seen_at,
        stats: payload<SystemStatsPayload>(host, 'system', 'SystemStatsResponse'),
        services: payload<{ services: ServiceInfo[] }>(
          host,
          'services',
          'ListServicesResponse',
        )?.services,
        docker: payload<DockerListPayload>(host, 'docker', 'DockerListResponse'),
        swarm: payload<SwarmListPayload>(host, 'swarm', 'SwarmListResponse'),
      },
    ]),
  );
}
