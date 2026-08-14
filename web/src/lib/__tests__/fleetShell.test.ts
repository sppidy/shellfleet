import { describe, expect, it } from 'vitest';
import {
  fleetShellCompletions,
  fleetShellPrompt,
  runFleetShellCommand,
  type FleetShellContext,
} from '../fleetShell';

const context: FleetShellContext = {
  hosts: [
    {
      agent_id: 'onic1-id',
      hostname: 'onic1',
      status: 'online',
      protocol_version: 19,
      capabilities: ['systemd', 'docker', 'trusted-root'],
      metadata: {},
      first_seen_at: 100,
      last_seen_at: 990,
      disconnected_at: null,
      system: { observed_at: 985, value: {} },
      services: { observed_at: 986, value: {} },
      docker: { observed_at: 987, value: {} },
      swarm: null,
    },
    {
      agent_id: 'worker-1-id',
      hostname: 'worker-1',
      status: 'offline',
      protocol_version: 19,
      capabilities: ['systemd', 'docker'],
      metadata: {},
      first_seen_at: 100,
      last_seen_at: 800,
      disconnected_at: 801,
      system: { observed_at: 790, value: {} },
      services: { observed_at: 791, value: {} },
      docker: { observed_at: 792, value: {} },
      swarm: null,
    },
  ],
  snapshots: {
    'onic1-id': {
      agentId: 'onic1-id',
      hostname: 'onic1',
      status: 'online',
      lastSeenAt: 990,
      stats: {
        hostname: 'onic1',
        kernel: '6.12',
        uptime_secs: 90_000,
        cpu_count: 4,
        load_1: 0.5,
        load_5: 0.4,
        load_15: 0.3,
        mem_total_kb: 1_000,
        mem_available_kb: 400,
        swap_total_kb: 0,
        swap_free_kb: 0,
        root_disk_total_kb: 2_000,
        root_disk_used_kb: 800,
      },
      services: [
        { name: 'sshd.service', description: 'OpenSSH server', status: 'running', active_state: 'active' },
        { name: 'broken.service', description: 'Broken worker', status: 'failed', active_state: 'failed' },
      ],
      docker: {
        available: true,
        swarm_role: 'manager',
        error: null,
        containers: [
          { id: 'abc', names: 'web', image: 'shellfleet/web:latest', state: 'running', status: 'Up', ports: '8080' },
          { id: 'def', names: 'old-api', image: 'shellfleet/api:old', state: 'exited', status: 'Exited (1)', ports: '' },
        ],
      },
    },
    'worker-1-id': {
      agentId: 'worker-1-id',
      hostname: 'worker-1',
      status: 'offline',
      lastSeenAt: 800,
      stats: {
        hostname: 'worker-1',
        kernel: '6.12',
        uptime_secs: 3_600,
        cpu_count: 64,
        load_1: 20,
        load_5: 19,
        load_15: 18,
        mem_total_kb: 8_000,
        mem_available_kb: 100,
        swap_total_kb: 0,
        swap_free_kb: 0,
        root_disk_total_kb: 10_000,
        root_disk_used_kb: 9_500,
      },
      services: [],
      docker: { available: true, swarm_role: 'worker', error: null, containers: [] },
    },
  },
  healthByAgent: {
    'onic1-id': { agent_id: 'onic1-id', total: 3, green: 2, red: 1, unknown: 0 },
  },
  scopeAgentId: null,
  liveStatus: 'live',
  commandHistory: [],
  nowSeconds: 1_000,
};

function output(command: string, override: Partial<FleetShellContext> = {}): string {
  return runFleetShellCommand(command, { ...context, ...override }).lines
    .map((entry) => entry.text)
    .join('\n');
}

describe('Fleet Shell command engine', () => {
  it('aggregates only online host snapshots for fleet stats', () => {
    const result = output('stats');
    expect(result).toContain('FLEET 1/2 online');
    expect(result).toContain('CPU   4 cores  load 0.50');
    expect(result).toContain('SVC   1/2 healthy  1 failed');
    expect(result).not.toContain('64 cores');
  });

  it('inspects an offline host from its durable snapshot', () => {
    const result = output('stats worker');
    expect(result).toContain('HOST worker-1  OFFLINE');
    expect(result).toContain('CPU  64 cores');
    expect(result).toContain('data 3m ago');
  });

  it('sets and reports host context without requiring the host to be online', () => {
    const result = runFleetShellCommand('use worker-1', context);
    expect(result.effect).toEqual({ type: 'set-scope', agentId: 'worker-1-id' });
    expect(result.lines.map((entry) => entry.text).join('\n')).toContain('last durable snapshots');
    expect(fleetShellPrompt({ ...context, scopeAgentId: 'onic1-id' })).toBe('onic1');
  });

  it('filters warning hosts and supports shell-like aliases', () => {
    const result = output('ls warn');
    expect(result).toContain('onic1');
    expect(result).toContain('worker-1');
    expect(output('top')).toContain('FLEET 1/2 online');
  });

  it('filters services and containers in either fleet or host scope', () => {
    expect(output('services failed onic1')).toContain('broken.service');
    expect(output('services active', { scopeAgentId: 'onic1-id' })).toContain('sshd.service');
    expect(output('containers stopped')).toContain('old-api');
    expect(output('docker running onic1')).toContain('web');
  });

  it('reports health snapshots and searches all durable resource kinds', () => {
    expect(output('health onic1')).toContain('2        1        0');
    expect(output('find shellfleet')).toContain('CONTAINER');
    expect(output('find openssh')).toContain('SERVICE');
    expect(output('find trusted-root')).toContain('HOST');
  });

  it('returns typed navigation effects only for known destinations and online hosts', () => {
    expect(runFleetShellCommand('open onic1 metrics', context).effect).toEqual({
      type: 'open-host',
      agentId: 'onic1-id',
      view: 'metrics',
    });
    expect(runFleetShellCommand('terminal', context).effect).toEqual({ type: 'navigate', target: 'terminal' });
    expect(runFleetShellCommand('open worker-1 docker', context).effect).toBeUndefined();
    expect(output('open worker-1 docker')).toContain('is offline');
  });

  it('rejects arbitrary OS commands and points operators to the explicit terminal', () => {
    const result = output('rm -rf /');
    expect(result).toContain('unknown Fleet Shell command: rm');
    expect(result).toContain('OS commands run only in `terminal`');
  });

  it('completes commands, filters, views, and hostnames', () => {
    expect(fleetShellCompletions('st', context)).toEqual(['stats']);
    expect(fleetShellCompletions('services fa', context)).toEqual(['services failed']);
    expect(fleetShellCompletions('stats wo', context)).toEqual(['stats worker-1']);
    expect(fleetShellCompletions('open onic1 met', context)).toEqual(['open onic1 metrics']);
    expect(fleetShellCompletions('stats onic1 ', context)).toEqual([]);
    expect(fleetShellCompletions('services failed on', context)).toEqual(['services failed onic1']);
    expect(fleetShellCompletions('open metrics ', context)).toEqual([]);
  });

  it('keeps command history local to the shell context', () => {
    const result = output('history', { commandHistory: ['stats', 'hosts offline', 'history'] });
    expect(result).toContain('  1  stats');
    expect(result).toContain('  3  history');
  });
});
