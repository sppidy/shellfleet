import type {
  CoreAgentSnapshot,
  CoreLiveStatus,
  FleetHost,
} from './coreFleet';
import type { HealthSnapshotRow } from './types';

export type FleetShellTone = 'normal' | 'dim' | 'accent' | 'success' | 'warning' | 'error' | 'info';

export type FleetShellLine = {
  text: string;
  tone?: FleetShellTone;
};

export type FleetShellHostView =
  | 'dashboard'
  | 'docker'
  | 'kubernetes'
  | 'metrics'
  | 'journal'
  | 'updates'
  | 'health'
  | 'config';

export type FleetShellDestination = 'overview' | 'terminal' | 'activity' | 'notifications';

export type FleetShellEffect =
  | { type: 'refresh' }
  | { type: 'set-scope'; agentId: string | null }
  | { type: 'navigate'; target: FleetShellDestination }
  | { type: 'open-host'; agentId: string; view: FleetShellHostView };

export type FleetShellResult = {
  lines: FleetShellLine[];
  effect?: FleetShellEffect;
  clear?: boolean;
};

export type FleetShellContext = {
  hosts: FleetHost[];
  snapshots: Record<string, CoreAgentSnapshot>;
  healthByAgent: Record<string, HealthSnapshotRow>;
  scopeAgentId: string | null;
  liveStatus: CoreLiveStatus;
  commandHistory?: string[];
  nowSeconds?: number;
};

type CommandHelp = {
  command: string;
  usage: string;
  summary: string;
};

const MAX_ROWS = 40;

const COMMAND_HELP: CommandHelp[] = [
  { command: 'stats', usage: 'stats [host]', summary: 'fleet or host CPU, load, memory, disk, and workload totals' },
  { command: 'hosts', usage: 'hosts [all|online|offline|warn]', summary: 'list hosts from the durable fleet snapshot' },
  { command: 'use', usage: 'use <host|fleet>', summary: 'set the default host context for later commands' },
  { command: 'services', usage: 'services [all|active|failed|inactive] [host]', summary: 'list systemd services' },
  { command: 'containers', usage: 'containers [all|running|stopped] [host]', summary: 'list Docker containers' },
  { command: 'health', usage: 'health [host]', summary: 'show health-probe rollups' },
  { command: 'find', usage: 'find <text>', summary: 'search hosts, services, and containers' },
  { command: 'open', usage: 'open <destination> | open <host> [view] | open <view>', summary: 'open a dashboard destination or host view' },
  { command: 'terminal', usage: 'terminal', summary: 'open the admin multi-host root terminal' },
  { command: 'refresh', usage: 'refresh', summary: 'request a new durable fleet snapshot' },
  { command: 'history', usage: 'history', summary: 'show recent Fleet Shell commands' },
  { command: 'clear', usage: 'clear', summary: 'clear this local transcript' },
  { command: 'help', usage: 'help [command]', summary: 'show command help' },
];

const ALIASES: Record<string, string> = {
  '?': 'help',
  docker: 'containers',
  ls: 'hosts',
  probes: 'health',
  select: 'use',
  top: 'stats',
};

const HOST_VIEWS: Record<string, FleetShellHostView> = {
  dashboard: 'dashboard',
  overview: 'dashboard',
  services: 'dashboard',
  stats: 'dashboard',
  docker: 'docker',
  containers: 'docker',
  kubernetes: 'kubernetes',
  k8s: 'kubernetes',
  metrics: 'metrics',
  journal: 'journal',
  updates: 'updates',
  health: 'health',
  config: 'config',
};

const GLOBAL_DESTINATIONS: readonly FleetShellDestination[] = ['overview', 'terminal', 'activity', 'notifications'];
const SERVICE_FILTERS = ['all', 'active', 'failed', 'inactive'] as const;
const CONTAINER_FILTERS = ['all', 'running', 'stopped'] as const;
const HOST_FILTERS = ['all', 'online', 'offline', 'warn'] as const;

function line(text: string, tone: FleetShellTone = 'normal'): FleetShellLine {
  return { text, tone };
}

function normalizeCommand(command: string): string {
  const normalized = command.toLowerCase();
  return ALIASES[normalized] ?? normalized;
}

function now(context: FleetShellContext): number {
  return context.nowSeconds ?? Math.floor(Date.now() / 1000);
}

function clip(value: string, width: number): string {
  if (value.length <= width) return value;
  return `${value.slice(0, Math.max(0, width - 1))}…`;
}

function cell(value: string, width: number): string {
  return clip(value, width).padEnd(width, ' ');
}

function formatKib(kib: number): string {
  if (!Number.isFinite(kib) || kib < 0) return '—';
  let value = kib * 1024;
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function percent(used: number, total: number): string {
  if (total <= 0) return '—';
  return `${Math.round((used / total) * 100)}%`;
}

function formatUptime(seconds: number): string {
  if (seconds <= 0) return '—';
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function formatAge(unixSeconds: number, nowSeconds: number): string {
  const elapsed = Math.max(0, nowSeconds - unixSeconds);
  if (elapsed < 60) return `${elapsed}s ago`;
  if (elapsed < 3_600) return `${Math.floor(elapsed / 60)}m ago`;
  if (elapsed < 86_400) return `${Math.floor(elapsed / 3_600)}h ago`;
  return `${Math.floor(elapsed / 86_400)}d ago`;
}

function snapshotFor(host: FleetHost, context: FleetShellContext): CoreAgentSnapshot {
  return context.snapshots[host.agent_id] ?? {
    agentId: host.agent_id,
    hostname: host.hostname,
    status: host.status,
    lastSeenAt: host.last_seen_at,
  };
}

function hostNames(host: FleetHost): string[] {
  return [host.hostname, host.agent_id, host.agent_id.replace(/-id$/, '')]
    .map((value) => value.toLowerCase());
}

type HostResolution =
  | { host: FleetHost }
  | { lines: FleetShellLine[] };

function resolveHost(query: string, context: FleetShellContext): HostResolution {
  const needle = query.trim().replace(/^@/, '').toLowerCase();
  if (!needle) return { lines: [line('host name required', 'error')] };

  const exact = context.hosts.filter((host) => hostNames(host).includes(needle));
  if (exact.length === 1) return { host: exact[0] };

  const prefix = context.hosts.filter((host) =>
    hostNames(host).some((name) => name.startsWith(needle)),
  );
  if (prefix.length === 1) return { host: prefix[0] };
  if (prefix.length > 1 || exact.length > 1) {
    const matches = (exact.length > 1 ? exact : prefix)
      .map((host) => host.hostname)
      .sort((a, b) => a.localeCompare(b));
    return {
      lines: [
        line(`ambiguous host "${query}": ${matches.join(', ')}`, 'warning'),
        line('type more of the hostname or agent id', 'dim'),
      ],
    };
  }
  return {
    lines: [
      line(`unknown host: ${query}`, 'error'),
      line('run `hosts` to list known durable fleet members', 'dim'),
    ],
  };
}

function scopedHost(context: FleetShellContext): FleetHost | null {
  if (!context.scopeAgentId) return null;
  return context.hosts.find((host) => host.agent_id === context.scopeAgentId) ?? null;
}

function resolveOptionalHost(
  query: string | undefined,
  context: FleetShellContext,
): HostResolution | { host: null } {
  if (query) return resolveHost(query, context);
  return { host: scopedHost(context) };
}

function warningCount(host: FleetHost, context: FleetShellContext): number {
  const snapshot = snapshotFor(host, context);
  const failed = snapshot.services?.filter((service) => service.active_state === 'failed').length ?? 0;
  const health = context.healthByAgent[host.agent_id];
  return failed + (health?.red ?? 0) + (health?.unknown ?? 0) + (host.status === 'offline' ? 1 : 0);
}

function hostStats(host: FleetHost, context: FleetShellContext): FleetShellResult {
  const snapshot = snapshotFor(host, context);
  const stats = snapshot.stats;
  const services = snapshot.services;
  const docker = snapshot.docker;
  const health = context.healthByAgent[host.agent_id];
  const newestSnapshot = [host.system, host.services, host.docker, host.swarm]
    .filter((value) => value !== null)
    .map((value) => value.observed_at)
    .sort((a, b) => b - a)[0];
  const result = [
    line(
      `HOST ${host.hostname}  ${host.status.toUpperCase()}  protocol v${host.protocol_version}`,
      host.status === 'online' ? 'accent' : 'warning',
    ),
    line(
      `SEEN ${formatAge(host.last_seen_at, now(context))}${newestSnapshot ? `  data ${formatAge(newestSnapshot, now(context))}` : ''}`,
      'dim',
    ),
  ];

  if (stats) {
    const memUsed = stats.mem_total_kb - stats.mem_available_kb;
    result.push(
      line(`CPU  ${stats.cpu_count} cores  load ${stats.load_1.toFixed(2)} ${stats.load_5.toFixed(2)} ${stats.load_15.toFixed(2)}`),
      line(`MEM  ${formatKib(memUsed)} / ${formatKib(stats.mem_total_kb)}  (${percent(memUsed, stats.mem_total_kb)})`),
      line(`DISK ${formatKib(stats.root_disk_used_kb)} / ${formatKib(stats.root_disk_total_kb)}  (${percent(stats.root_disk_used_kb, stats.root_disk_total_kb)})`),
      line(`UP   ${formatUptime(stats.uptime_secs)}  kernel ${stats.kernel}`, 'dim'),
    );
  } else {
    result.push(line('SYSTEM no durable system snapshot', 'warning'));
  }

  const failed = services?.filter((service) => service.active_state === 'failed').length ?? 0;
  result.push(
    services
      ? line(`SERVICES   ${services.length - failed}/${services.length} healthy  ${failed} failed`, failed > 0 ? 'warning' : 'success')
      : line('SERVICES   no durable snapshot', 'dim'),
  );

  if (docker?.available) {
    const running = docker.containers.filter((container) => container.state === 'running').length;
    result.push(line(`CONTAINERS ${running}/${docker.containers.length} running  swarm ${docker.swarm_role}`, running === docker.containers.length ? 'success' : 'warning'));
  } else {
    result.push(line('CONTAINERS unavailable in durable snapshot', 'dim'));
  }

  if (health) {
    result.push(line(`PROBES     ${health.green}/${health.total} green  ${health.red} red  ${health.unknown} unknown`, health.red > 0 || health.unknown > 0 ? 'warning' : 'success'));
  } else {
    result.push(line('PROBES     no health snapshot', 'dim'));
  }

  result.push(line(`CAPS ${host.capabilities.length > 0 ? host.capabilities.join(', ') : '—'}`, 'dim'));
  return { lines: result };
}

function fleetStats(context: FleetShellContext): FleetShellResult {
  const onlineHosts = context.hosts.filter((host) => host.status === 'online');
  let cores = 0;
  let load = 0;
  let reporting = 0;
  let memUsed = 0;
  let memTotal = 0;
  let diskUsed = 0;
  let diskTotal = 0;
  let services = 0;
  let failed = 0;
  let containers = 0;
  let running = 0;
  let probes = 0;
  let green = 0;
  let red = 0;
  let unknown = 0;

  for (const host of onlineHosts) {
    const snapshot = snapshotFor(host, context);
    if (snapshot.stats) {
      cores += snapshot.stats.cpu_count;
      load += snapshot.stats.load_1;
      memTotal += snapshot.stats.mem_total_kb;
      memUsed += snapshot.stats.mem_total_kb - snapshot.stats.mem_available_kb;
      diskTotal += snapshot.stats.root_disk_total_kb;
      diskUsed += snapshot.stats.root_disk_used_kb;
      reporting += 1;
    }
    if (snapshot.services) {
      services += snapshot.services.length;
      failed += snapshot.services.filter((service) => service.active_state === 'failed').length;
    }
    if (snapshot.docker?.available) {
      containers += snapshot.docker.containers.length;
      running += snapshot.docker.containers.filter((container) => container.state === 'running').length;
    }
    const health = context.healthByAgent[host.agent_id];
    if (health) {
      probes += health.total;
      green += health.green;
      red += health.red;
      unknown += health.unknown;
    }
  }

  const feedTone: FleetShellTone = context.liveStatus === 'live' ? 'success' : context.liveStatus === 'connecting' ? 'info' : 'warning';
  return {
    lines: [
      line(`FLEET ${onlineHosts.length}/${context.hosts.length} online  feed ${context.liveStatus}`, feedTone),
      line(`CPU   ${cores} cores  load ${load.toFixed(2)}  ${reporting} reporting`),
      line(`MEM   ${formatKib(memUsed)} / ${formatKib(memTotal)}  (${percent(memUsed, memTotal)})`),
      line(`DISK  ${formatKib(diskUsed)} / ${formatKib(diskTotal)}  (${percent(diskUsed, diskTotal)})`),
      line(`SVC   ${services - failed}/${services} healthy  ${failed} failed`, failed > 0 ? 'warning' : 'success'),
      line(`CTR   ${running}/${containers} running`),
      line(`PROBE ${green}/${probes} green  ${red} red  ${unknown} unknown`, red > 0 || unknown > 0 ? 'warning' : 'success'),
      line('aggregate values use online hosts; host commands can inspect offline durable snapshots', 'dim'),
    ],
  };
}

function runStats(args: string[], context: FleetShellContext): FleetShellResult {
  if (args.length > 1) return { lines: [line('usage: stats [host]', 'error')] };
  const resolved = resolveOptionalHost(args[0], context);
  if ('lines' in resolved) return { lines: resolved.lines };
  return resolved.host ? hostStats(resolved.host, context) : fleetStats(context);
}

function runHosts(args: string[], context: FleetShellContext): FleetShellResult {
  if (args.length > 1) return { lines: [line('usage: hosts [all|online|offline|warn]', 'error')] };
  const requested = (args[0] ?? 'all').toLowerCase();
  const filter = requested === 'warning' ? 'warn' : requested;
  if (!HOST_FILTERS.includes(filter as (typeof HOST_FILTERS)[number])) {
    return { lines: [line(`unknown host filter: ${requested}`, 'error'), line('filters: all, online, offline, warn', 'dim')] };
  }
  const hosts = context.hosts
    .filter((host) => filter === 'all' || host.status === filter || (filter === 'warn' && warningCount(host, context) > 0))
    .sort((a, b) => a.hostname.localeCompare(b.hostname));
  if (hosts.length === 0) return { lines: [line(`no hosts match ${filter}`, 'dim')] };

  const lines = [line(`${cell('STATUS', 9)}${cell('HOST', 24)}${cell('LOAD', 8)}${cell('MEM', 7)}${cell('DISK', 7)}${cell('FAIL', 7)}PROBES`, 'accent')];
  for (const host of hosts.slice(0, MAX_ROWS)) {
    const snapshot = snapshotFor(host, context);
    const stats = snapshot.stats;
    const failed = snapshot.services?.filter((service) => service.active_state === 'failed').length ?? 0;
    const health = context.healthByAgent[host.agent_id];
    const mem = stats ? percent(stats.mem_total_kb - stats.mem_available_kb, stats.mem_total_kb) : '—';
    const disk = stats ? percent(stats.root_disk_used_kb, stats.root_disk_total_kb) : '—';
    const probes = health ? `${health.green}/${health.total}` : '—';
    lines.push(line(
      `${cell(host.status, 9)}${cell(host.hostname, 24)}${cell(stats ? stats.load_1.toFixed(2) : '—', 8)}${cell(mem, 7)}${cell(disk, 7)}${cell(String(failed), 7)}${probes}`,
      warningCount(host, context) > 0 ? 'warning' : 'normal',
    ));
  }
  if (hosts.length > MAX_ROWS) lines.push(line(`… ${hosts.length - MAX_ROWS} more hosts; refine the filter`, 'dim'));
  lines.push(line(`${hosts.length} host${hosts.length === 1 ? '' : 's'}`, 'dim'));
  return { lines };
}

function parseFilteredTarget<T extends string>(
  args: string[],
  filters: readonly T[],
  context: FleetShellContext,
  usage: string,
): { filter: T; hosts: FleetHost[] } | { lines: FleetShellLine[] } {
  let filter = filters[0];
  const hostParts: string[] = [];
  for (const argument of args) {
    const lowered = argument.toLowerCase();
    if (filters.includes(lowered as T)) {
      if (filter !== filters[0]) return { lines: [line(`usage: ${usage}`, 'error')] };
      filter = lowered as T;
    } else {
      hostParts.push(argument);
    }
  }
  if (hostParts.length > 1) return { lines: [line(`usage: ${usage}`, 'error')] };
  if (hostParts.length === 1) {
    const resolved = resolveHost(hostParts[0], context);
    return 'lines' in resolved ? resolved : { filter, hosts: [resolved.host] };
  }
  const scoped = scopedHost(context);
  return { filter, hosts: scoped ? [scoped] : context.hosts };
}

function runServices(args: string[], context: FleetShellContext): FleetShellResult {
  const parsed = parseFilteredTarget(args, SERVICE_FILTERS, context, 'services [all|active|failed|inactive] [host]');
  if ('lines' in parsed) return { lines: parsed.lines };
  const rows = parsed.hosts.flatMap((host) =>
    (snapshotFor(host, context).services ?? []).map((service) => ({ host, service })),
  ).filter(({ service }) => {
    if (parsed.filter === 'all') return true;
    if (parsed.filter === 'inactive') return service.active_state !== 'active' && service.active_state !== 'failed';
    return service.active_state === parsed.filter;
  }).sort((a, b) => {
    const failureOrder = Number(b.service.active_state === 'failed') - Number(a.service.active_state === 'failed');
    return failureOrder || a.host.hostname.localeCompare(b.host.hostname) || a.service.name.localeCompare(b.service.name);
  });
  if (rows.length === 0) return { lines: [line(`no services match ${parsed.filter} in the current scope`, 'dim')] };

  const lines = [line(`${cell('STATE', 11)}${cell('HOST', 22)}${cell('SERVICE', 30)}DESCRIPTION`, 'accent')];
  for (const { host, service } of rows.slice(0, MAX_ROWS)) {
    lines.push(line(
      `${cell(service.active_state || service.status || '—', 11)}${cell(host.hostname, 22)}${cell(service.name, 30)}${clip(service.description || '—', 54)}`,
      service.active_state === 'failed' ? 'error' : service.active_state === 'active' ? 'normal' : 'warning',
    ));
  }
  if (rows.length > MAX_ROWS) lines.push(line(`… ${rows.length - MAX_ROWS} more services; add a state or host filter`, 'dim'));
  lines.push(line(`${rows.length} matching service${rows.length === 1 ? '' : 's'}`, 'dim'));
  return { lines };
}

function runContainers(args: string[], context: FleetShellContext): FleetShellResult {
  const parsed = parseFilteredTarget(args, CONTAINER_FILTERS, context, 'containers [all|running|stopped] [host]');
  if ('lines' in parsed) return { lines: parsed.lines };
  const rows = parsed.hosts.flatMap((host) => {
    const docker = snapshotFor(host, context).docker;
    return docker?.available ? docker.containers.map((container) => ({ host, container })) : [];
  }).filter(({ container }) => {
    if (parsed.filter === 'all') return true;
    if (parsed.filter === 'stopped') return container.state !== 'running';
    return container.state === 'running';
  }).sort((a, b) => a.host.hostname.localeCompare(b.host.hostname) || a.container.names.localeCompare(b.container.names));
  if (rows.length === 0) return { lines: [line(`no containers match ${parsed.filter} in the current scope`, 'dim')] };

  const lines = [line(`${cell('STATE', 11)}${cell('HOST', 22)}${cell('CONTAINER', 28)}${cell('IMAGE', 34)}STATUS`, 'accent')];
  for (const { host, container } of rows.slice(0, MAX_ROWS)) {
    lines.push(line(
      `${cell(container.state || '—', 11)}${cell(host.hostname, 22)}${cell(container.names || container.id.slice(0, 12), 28)}${cell(container.image || '—', 34)}${clip(container.status || '—', 40)}`,
      container.state === 'running' ? 'normal' : 'warning',
    ));
  }
  if (rows.length > MAX_ROWS) lines.push(line(`… ${rows.length - MAX_ROWS} more containers; add a state or host filter`, 'dim'));
  lines.push(line(`${rows.length} matching container${rows.length === 1 ? '' : 's'}`, 'dim'));
  return { lines };
}

function runHealth(args: string[], context: FleetShellContext): FleetShellResult {
  if (args.length > 1) return { lines: [line('usage: health [host]', 'error')] };
  const resolved = resolveOptionalHost(args[0], context);
  if ('lines' in resolved) return { lines: resolved.lines };
  const hosts = (resolved.host ? [resolved.host] : context.hosts)
    .slice()
    .sort((a, b) => a.hostname.localeCompare(b.hostname));
  if (hosts.length === 0) return { lines: [line('no hosts in the durable fleet snapshot', 'dim')] };

  const lines = [line(`${cell('HOST', 26)}${cell('TOTAL', 9)}${cell('GREEN', 9)}${cell('RED', 9)}UNKNOWN`, 'accent')];
  for (const host of hosts.slice(0, MAX_ROWS)) {
    const health = context.healthByAgent[host.agent_id];
    if (!health) {
      lines.push(line(`${cell(host.hostname, 26)}${cell('—', 9)}${cell('—', 9)}${cell('—', 9)}—`, 'dim'));
      continue;
    }
    lines.push(line(
      `${cell(host.hostname, 26)}${cell(String(health.total), 9)}${cell(String(health.green), 9)}${cell(String(health.red), 9)}${health.unknown}`,
      health.red > 0 || health.unknown > 0 ? 'warning' : 'success',
    ));
  }
  if (hosts.length > MAX_ROWS) lines.push(line(`… ${hosts.length - MAX_ROWS} more hosts`, 'dim'));
  return { lines };
}

function runFind(args: string[], context: FleetShellContext): FleetShellResult {
  const query = args.join(' ').trim();
  if (!query) return { lines: [line('usage: find <text>', 'error')] };
  const needle = query.toLowerCase();
  const scoped = scopedHost(context);
  const hosts = scoped ? [scoped] : context.hosts;
  const matches: FleetShellLine[] = [];
  for (const host of hosts) {
    const snapshot = snapshotFor(host, context);
    if ([host.hostname, host.agent_id, ...host.capabilities].some((value) => value.toLowerCase().includes(needle))) {
      matches.push(line(`HOST      ${cell(host.hostname, 24)}${host.status}  ${host.capabilities.join(', ')}`, host.status === 'online' ? 'normal' : 'warning'));
    }
    for (const service of snapshot.services ?? []) {
      if (`${service.name} ${service.description}`.toLowerCase().includes(needle)) {
        matches.push(line(`SERVICE   ${cell(host.hostname, 24)}${cell(service.name, 30)}${service.active_state}  ${clip(service.description, 50)}`, service.active_state === 'failed' ? 'error' : 'normal'));
      }
    }
    if (snapshot.docker?.available) {
      for (const container of snapshot.docker.containers) {
        if (`${container.names} ${container.image} ${container.id}`.toLowerCase().includes(needle)) {
          matches.push(line(`CONTAINER ${cell(host.hostname, 24)}${cell(container.names || container.id.slice(0, 12), 30)}${container.state}  ${clip(container.image, 80)}`, container.state === 'running' ? 'normal' : 'warning'));
        }
      }
    }
  }
  if (matches.length === 0) return { lines: [line(`no durable snapshot matches for "${query}"`, 'dim')] };
  const lines = matches.slice(0, MAX_ROWS);
  if (matches.length > MAX_ROWS) lines.push(line(`… ${matches.length - MAX_ROWS} more matches; narrow the query`, 'dim'));
  lines.push(line(`${matches.length} match${matches.length === 1 ? '' : 'es'}`, 'dim'));
  return { lines };
}

function runUse(args: string[], context: FleetShellContext): FleetShellResult {
  if (args.length === 0) {
    const host = scopedHost(context);
    return { lines: [line(host ? `current context: ${host.hostname}` : 'current context: fleet', 'info')] };
  }
  if (args.length !== 1) return { lines: [line('usage: use <host|fleet>', 'error')] };
  if (['fleet', '/', '..'].includes(args[0].toLowerCase())) {
    return { lines: [line('context set to fleet', 'success')], effect: { type: 'set-scope', agentId: null } };
  }
  const resolved = resolveHost(args[0], context);
  if ('lines' in resolved) return { lines: resolved.lines };
  return {
    lines: [
      line(`context set to ${resolved.host.hostname}`, 'success'),
      ...(resolved.host.status === 'offline' ? [line('host is offline; commands will use its last durable snapshots', 'warning')] : []),
    ],
    effect: { type: 'set-scope', agentId: resolved.host.agent_id },
  };
}

function runOpen(args: string[], context: FleetShellContext): FleetShellResult {
  if (args.length === 0 || args.length > 2) {
    return {
      lines: [
        line('usage: open <destination> | open <host> [view] | open <view>', 'error'),
        line('destinations: overview, terminal, activity, notifications', 'dim'),
        line('views: dashboard, docker, k8s, metrics, journal, updates, health, config', 'dim'),
      ],
    };
  }
  const first = args[0].toLowerCase();
  if (args.length === 1 && GLOBAL_DESTINATIONS.includes(first as (typeof GLOBAL_DESTINATIONS)[number])) {
    return {
      lines: [line(`opening ${first}…`, 'info')],
      effect: { type: 'navigate', target: first as FleetShellDestination },
    };
  }

  let host: FleetHost | null = null;
  let viewName: string;
  if (args.length === 1 && HOST_VIEWS[first]) {
    host = scopedHost(context);
    viewName = first;
    if (!host) return { lines: [line(`no host context; use \`open <host> ${first}\` or \`use <host>\``, 'error')] };
  } else {
    const resolved = resolveHost(args[0], context);
    if ('lines' in resolved) return { lines: resolved.lines };
    host = resolved.host;
    viewName = (args[1] ?? 'dashboard').toLowerCase();
  }
  const view = HOST_VIEWS[viewName];
  if (!view) return { lines: [line(`unknown host view: ${viewName}`, 'error'), line('views: dashboard, docker, k8s, metrics, journal, updates, health, config', 'dim')] };
  if (host.status !== 'online') {
    return { lines: [line(`${host.hostname} is offline; live host views cannot be opened`, 'warning'), line(`use \`stats ${host.hostname}\` to inspect its durable snapshot`, 'dim')] };
  }
  return {
    lines: [line(`opening ${host.hostname} / ${view}…`, 'info')],
    effect: { type: 'open-host', agentId: host.agent_id, view },
  };
}

function runHelp(args: string[]): FleetShellResult {
  if (args.length > 1) return { lines: [line('usage: help [command]', 'error')] };
  if (args.length === 1) {
    const command = normalizeCommand(args[0]);
    const help = COMMAND_HELP.find((entry) => entry.command === command);
    if (!help) return { lines: [line(`no help for: ${args[0]}`, 'error')] };
    return { lines: [line(help.usage, 'accent'), line(help.summary), line('results are read-only durable snapshots unless the command explicitly opens another view', 'dim')] };
  }
  return {
    lines: [
      line('FLEET SHELL · READ-ONLY SNAPSHOT COMMANDS', 'accent'),
      ...COMMAND_HELP.map((entry) => line(`${cell(entry.usage, 49)}${entry.summary}`)),
      line('aliases: top=stats, ls=hosts, docker=containers, probes=health, select=use, ?=help', 'dim'),
      line('Tab completes · ↑/↓ recall history · `terminal` opens the separate admin root shell', 'dim'),
    ],
  };
}

export function runFleetShellCommand(input: string, context: FleetShellContext): FleetShellResult {
  const tokens = input.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return { lines: [] };
  const command = normalizeCommand(tokens[0]);
  const args = tokens.slice(1);

  switch (command) {
    case 'help': return runHelp(args);
    case 'stats': return runStats(args, context);
    case 'hosts': return runHosts(args, context);
    case 'services': return runServices(args, context);
    case 'containers': return runContainers(args, context);
    case 'health': return runHealth(args, context);
    case 'find': return runFind(args, context);
    case 'use': return runUse(args, context);
    case 'open': return runOpen(args, context);
    case 'terminal':
      return args.length === 0
        ? { lines: [line('opening the admin multi-host root terminal…', 'info')], effect: { type: 'navigate', target: 'terminal' } }
        : { lines: [line('usage: terminal', 'error'), line('choose the host from the terminal tab bar', 'dim')] };
    case 'refresh':
      return args.length === 0
        ? { lines: [line('fleet refresh requested; snapshots will update in place', 'success')], effect: { type: 'refresh' } }
        : { lines: [line('usage: refresh', 'error')] };
    case 'history': {
      if (args.length > 0) return { lines: [line('usage: history', 'error')] };
      const history = context.commandHistory ?? [];
      if (history.length === 0) return { lines: [line('history is empty', 'dim')] };
      const start = Math.max(0, history.length - 20);
      return { lines: history.slice(start).map((entry, index) => line(`${String(start + index + 1).padStart(3, ' ')}  ${entry}`)) };
    }
    case 'clear':
      return args.length === 0 ? { lines: [], clear: true } : { lines: [line('usage: clear', 'error')] };
    default:
      return {
        lines: [
          line(`unknown Fleet Shell command: ${tokens[0]}`, 'error'),
          line('type `help` for snapshot commands; OS commands run only in `terminal`', 'dim'),
        ],
      };
  }
}

function unique(values: string[]): string[] {
  return [...new Set(values)];
}

export function fleetShellCompletions(input: string, context: FleetShellContext): string[] {
  const trimmedStart = input.trimStart();
  const trailingSpace = /\s$/.test(trimmedStart);
  const tokens = trimmedStart.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return COMMAND_HELP.map((entry) => entry.command);

  if (tokens.length === 1 && !trailingSpace) {
    const prefix = tokens[0].toLowerCase();
    return unique([...COMMAND_HELP.map((entry) => entry.command), ...Object.keys(ALIASES)])
      .filter((candidate) => candidate.startsWith(prefix))
      .sort();
  }

  const command = normalizeCommand(tokens[0]);
  const completedArgs = trailingSpace ? tokens.slice(1) : tokens.slice(1, -1);
  const partial = trailingSpace ? '' : (tokens.at(-1) ?? '').toLowerCase();
  const hosts = context.hosts.map((host) => host.hostname).sort((a, b) => a.localeCompare(b));
  let options: string[] = [];

  if (command === 'help' && completedArgs.length === 0) options = COMMAND_HELP.map((entry) => entry.command);
  else if (command === 'hosts' && completedArgs.length === 0) options = [...HOST_FILTERS];
  else if (command === 'use' && completedArgs.length === 0) options = ['fleet', ...hosts];
  else if (['stats', 'health'].includes(command) && completedArgs.length === 0) options = hosts;
  else if (command === 'services') {
    const hasFilter = completedArgs.some((argument) => SERVICE_FILTERS.includes(argument.toLowerCase() as (typeof SERVICE_FILTERS)[number]));
    const hasHost = completedArgs.some((argument) => !SERVICE_FILTERS.includes(argument.toLowerCase() as (typeof SERVICE_FILTERS)[number]));
    if (!hasFilter) options.push(...SERVICE_FILTERS);
    if (!hasHost) options.push(...hosts);
  }
  else if (command === 'containers') {
    const hasFilter = completedArgs.some((argument) => CONTAINER_FILTERS.includes(argument.toLowerCase() as (typeof CONTAINER_FILTERS)[number]));
    const hasHost = completedArgs.some((argument) => !CONTAINER_FILTERS.includes(argument.toLowerCase() as (typeof CONTAINER_FILTERS)[number]));
    if (!hasFilter) options.push(...CONTAINER_FILTERS);
    if (!hasHost) options.push(...hosts);
  }
  else if (command === 'open') {
    const first = completedArgs[0]?.toLowerCase();
    if (completedArgs.length === 0) {
      options = [...GLOBAL_DESTINATIONS, ...hosts, ...Object.keys(HOST_VIEWS)];
    } else if (completedArgs.length === 1 && first && context.hosts.some((host) => hostNames(host).includes(first))) {
      options = unique(Object.keys(HOST_VIEWS));
    }
  }

  const prefixText = [tokens[0], ...completedArgs].join(' ');
  return unique(options)
    .filter((candidate) => candidate.toLowerCase().startsWith(partial))
    .filter((candidate) => !completedArgs.some((argument) => argument.toLowerCase() === candidate.toLowerCase()))
    .sort((a, b) => a.localeCompare(b))
    .map((candidate) => `${prefixText} ${candidate}`.trim());
}

export function fleetShellPrompt(context: FleetShellContext): string {
  return scopedHost(context)?.hostname ?? 'fleet';
}
