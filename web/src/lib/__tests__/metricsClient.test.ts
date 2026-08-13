// @vitest-environment node
import { describe, it, expect } from 'vitest';
import { parsePanelId, panelMatchesSource, makePollGate } from '../metricsClient';

describe('metricsClient', () => {
  it('parsePanelId distinguishes builtin vs db', () => {
    expect(parsePanelId('cpu_percent')).toEqual({ builtin: true, rowid: null });
    expect(parsePanelId('db:7')).toEqual({ builtin: false, rowid: 7 });
    expect(parsePanelId('db:nope')).toEqual({ builtin: false, rowid: null });
  });

  it('panelMatchesSource: pinned shows only for its source; unpinned always', () => {
    expect(panelMatchesSource({ source: 'influx' }, 'prom')).toBe(false);
    expect(panelMatchesSource({ source: 'prom' }, 'prom')).toBe(true);
    expect(panelMatchesSource({ source: null }, 'prom')).toBe(true);
    expect(panelMatchesSource({ source: 'influx' }, '')).toBe(true); // no filter
  });

  it('pollGate prevents overlap', () => {
    const g = makePollGate();
    expect(g.shouldRun()).toBe(true);
    g.start();
    expect(g.shouldRun()).toBe(false);
    g.done();
    expect(g.shouldRun()).toBe(true);
  });
});
