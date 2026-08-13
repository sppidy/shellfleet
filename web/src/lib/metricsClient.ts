// Pure helpers for the EE metrics UI — extracted so the render-adjacent logic
// is unit-testable without React rendering (the chart crash class).

/** A panel id is either a built-in string (`cpu_percent`) or `db:<rowid>`. */
export function parsePanelId(id: string): { builtin: boolean; rowid: number | null } {
  if (id.startsWith('db:')) {
    const n = Number(id.slice(3));
    return { builtin: false, rowid: Number.isFinite(n) ? n : null };
  }
  return { builtin: true, rowid: null };
}

/**
 * Source-filter predicate. A pinned panel (its own `source`) shows only when
 * that source is selected; an unpinned panel always shows (and runs against the
 * selected source). Empty `selected` means "no filter" — show everything.
 */
export function panelMatchesSource(panel: { source: string | null }, selected: string): boolean {
  if (!selected) return true;
  if (!panel.source) return true;
  return panel.source === selected;
}

/** Prevents overlapping polls for one panel: skip a tick while a request is in flight. */
export function makePollGate() {
  let inFlight = false;
  return {
    shouldRun: () => !inFlight,
    start: () => { inFlight = true; },
    done: () => { inFlight = false; },
  };
}
