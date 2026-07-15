// @vitest-environment node
import { describe, it, expect } from 'vitest';
import {
  validateActionPattern,
  parseActionsTextarea,
  normalizeActions,
  expandWildcardMinus,
  validatePolicyName,
  validateDescription,
  eePageGuard,
} from '../iamPolicy';

const ALL_ACTIONS = [
  'agent:View',
  'agent:Terminal',
  'agent:Exec',
  'service:Start',
  'service:Stop',
];

describe('validateActionPattern', () => {
  it('accepts *', () => {
    expect(validateActionPattern('*', ALL_ACTIONS)).toBe(true);
  });

  it('accepts category:*', () => {
    expect(validateActionPattern('agent:*', ALL_ACTIONS)).toBe(true);
    expect(validateActionPattern('service:*', ALL_ACTIONS)).toBe(true);
  });

  it('accepts exact action names', () => {
    expect(validateActionPattern('agent:View', ALL_ACTIONS)).toBe(true);
    expect(validateActionPattern('service:Start', ALL_ACTIONS)).toBe(true);
  });

  it('rejects unknown category:* like xyz:*', () => {
    expect(validateActionPattern('xyz:*', ALL_ACTIONS)).toBe(false);
  });

  it('rejects general globs like backup:R*', () => {
    expect(validateActionPattern('backup:R*', ALL_ACTIONS)).toBe(false);
  });

  it('rejects unknown exact actions', () => {
    expect(validateActionPattern('agent:Fly', ALL_ACTIONS)).toBe(false);
  });
});

describe('parseActionsTextarea', () => {
  it('returns valid for well-formed JSON array of valid patterns', () => {
    const r = parseActionsTextarea('["*"]', ALL_ACTIONS);
    expect(r).toEqual({ kind: 'valid', actions: ['*'] });
  });

  it('returns valid for mixed category and exact actions', () => {
    const r = parseActionsTextarea('["agent:*", "service:Start"]', ALL_ACTIONS);
    expect(r).toEqual({ kind: 'valid', actions: ['agent:*', 'service:Start'] });
  });

  it('returns invalid for well-formed JSON with bad patterns', () => {
    const r = parseActionsTextarea('["agent:*", "backup:R*"]', ALL_ACTIONS);
    expect(r).toEqual({ kind: 'invalid', actions: ['agent:*'], errors: ['backup:R*'] });
  });

  it('returns malformed for non-JSON', () => {
    const r = parseActionsTextarea('not json', ALL_ACTIONS);
    expect(r).toEqual({ kind: 'malformed', error: 'Invalid JSON' });
  });

  it('returns malformed for non-array JSON', () => {
    const r = parseActionsTextarea('{}', ALL_ACTIONS);
    expect(r).toEqual({ kind: 'malformed', error: 'Expected a JSON array' });
  });

  it('returns malformed for array with non-string entries', () => {
    const r = parseActionsTextarea('[1, 2, 3]', ALL_ACTIONS);
    expect(r).toEqual({
      kind: 'malformed',
      error: 'Expected a JSON array of strings',
    });
  });
});

describe('normalizeActions', () => {
  it('returns [] for empty selection', () => {
    expect(normalizeActions([], ALL_ACTIONS)).toEqual([]);
  });

  it('collapses to ["*"] when * is present', () => {
    expect(normalizeActions(['*'], ALL_ACTIONS)).toEqual(['*']);
    expect(normalizeActions(['*', 'agent:View'], ALL_ACTIONS)).toEqual(['*']);
  });

  it('emits category:* for fully-selected categories', () => {
    const selection = ['agent:View', 'agent:Terminal', 'agent:Exec'];
    expect(normalizeActions(selection, ALL_ACTIONS)).toEqual(['agent:*']);
  });

  it('emits individual actions for partially-selected categories', () => {
    const selection = ['agent:View', 'service:Start'];
    expect(normalizeActions(selection, ALL_ACTIONS)).toEqual([
      'agent:View',
      'service:Start',
    ]);
  });

  it('deduplicates category:* and exact actions', () => {
    const selection = ['agent:*', 'agent:View'];
    // agent:* subsumes agent:View, so dedup removes it
    const r = normalizeActions(selection, ALL_ACTIONS);
    expect(r).toEqual(['agent:*']);
  });

  it('sorts category:* by prefix then exact actions alphabetically', () => {
    const selection = ['service:Start', 'agent:*'];
    const r = normalizeActions(selection, ALL_ACTIONS);
    // agent:* before service:Start
    expect(r).toEqual(['agent:*', 'service:Start']);
  });
});

describe('expandWildcardMinus', () => {
  it('returns ALL_ACTIONS minus the removed action', () => {
    const r = expandWildcardMinus('agent:View', ALL_ACTIONS);
    expect(r).toHaveLength(ALL_ACTIONS.length - 1);
    expect(r).not.toContain('agent:View');
    expect(r).toContain('agent:Terminal');
  });
});

describe('validatePolicyName', () => {
  it('rejects empty name', () => {
    expect(validatePolicyName('')).toBe('Name is required');
    expect(validatePolicyName('   ')).toBe('Name is required');
  });

  it('rejects too-long name', () => {
    expect(validatePolicyName('a'.repeat(101))).toBe(
      'Name must be <= 100 characters',
    );
  });

  it('accepts valid name', () => {
    expect(validatePolicyName('MyPolicy')).toBeNull();
  });
});

describe('validateDescription', () => {
  it('accepts empty', () => {
    expect(validateDescription('')).toBeNull();
  });

  it('rejects too-long', () => {
    expect(validateDescription('a'.repeat(501))).toBe(
      'Description must be <= 500 characters',
    );
  });

  it('accepts valid', () => {
    expect(validateDescription('hello')).toBeNull();
  });
});

describe('eePageGuard', () => {
  it('loading for loading status', () => {
    expect(eePageGuard('loading', null, 'iam')).toEqual({ kind: 'loading' });
  });

  it('loading for pending_mfa', () => {
    expect(eePageGuard('pending_mfa', 'admin', 'iam')).toEqual({ kind: 'loading' });
  });

  it('redirect-to-login for guest', () => {
    expect(eePageGuard('guest', null, 'iam')).toEqual({
      kind: 'redirect-to-login',
    });
  });

  it('refuse for viewer role', () => {
    expect(eePageGuard('authed', 'viewer', 'iam')).toEqual({
      kind: 'refuse',
      message: '/iam requires the admin role.',
    });
  });

  it('render for authed admin', () => {
    expect(eePageGuard('authed', 'admin', 'iam')).toEqual({ kind: 'render' });
  });
});
