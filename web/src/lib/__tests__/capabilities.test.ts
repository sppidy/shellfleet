import { describe, it, expect } from 'vitest';
import {
  dockerAvailable,
  swarmAvailable,
  k8sAvailable,
  systemdAvailable,
} from '../capabilities';

describe('capability gating', () => {
  it('treats legacy agents (null caps) as "show every tab" except k8s', () => {
    // Pre-v15 agents have no advertised capabilities -> null.
    expect(dockerAvailable(null)).toBe(true);
    expect(swarmAvailable(null)).toBe(true);
    expect(systemdAvailable(null)).toBe(true);
    // k8s is the one surface that must NOT show for legacy agents.
    expect(k8sAvailable(null)).toBe(false);
  });

  it('gates docker on docker OR swarm', () => {
    expect(dockerAvailable(['docker'])).toBe(true);
    expect(dockerAvailable(['swarm'])).toBe(true);
    expect(dockerAvailable(['systemd'])).toBe(false);
    expect(dockerAvailable([])).toBe(false);
  });

  it('gates swarm strictly on the swarm capability', () => {
    expect(swarmAvailable(['swarm'])).toBe(true);
    expect(swarmAvailable(['docker'])).toBe(false);
    expect(swarmAvailable([])).toBe(false);
  });

  it('gates k8s strictly on the k8s capability and never for legacy', () => {
    expect(k8sAvailable(['k8s'])).toBe(true);
    expect(k8sAvailable(['docker', 'systemd'])).toBe(false);
    expect(k8sAvailable([])).toBe(false);
  });

  it('gates systemd surfaces on the systemd capability when advertised', () => {
    expect(systemdAvailable(['systemd'])).toBe(true);
    expect(systemdAvailable(['k8s'])).toBe(false);
    expect(systemdAvailable([])).toBe(false);
  });
});
