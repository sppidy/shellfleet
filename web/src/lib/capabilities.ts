/**
 * Per-agent capability gating. Agents advertise `capabilities` on Register
 * (protocol v15+). Pre-v15 agents have no entry, represented here as `null`,
 * which means "show every tab" so legacy hosts keep working. Once an agent
 * advertises capabilities, a missing one hides the matching UI surface.
 *
 * These predicates are extracted from `app/page.tsx` so the gating logic is
 * unit-testable without rendering the page (W0). Behavior is identical to the
 * previous inline expressions.
 */
export type Capabilities = string[] | null;

/** Docker tab: shown for legacy agents, or when docker OR swarm is advertised. */
export function dockerAvailable(caps: Capabilities): boolean {
  return caps === null || caps.includes('docker') || caps.includes('swarm');
}

/** Swarm surfaces: shown for legacy agents, or when swarm is advertised. */
export function swarmAvailable(caps: Capabilities): boolean {
  return caps === null || caps.includes('swarm');
}

/** Kubernetes tab: ONLY when explicitly advertised (never for legacy agents). */
export function k8sAvailable(caps: Capabilities): boolean {
  return caps !== null && caps.includes('k8s');
}

/** systemd surfaces (apt updates, journal, services): shown unless an agent advertises without it. */
export function systemdAvailable(caps: Capabilities): boolean {
  return caps === null || caps.includes('systemd');
}
