// @vitest-environment node
import { describe, it, expect } from 'vitest';
import { formatRelative, formatExpiry, isExpired, toUnixExpiry } from '../apiKeys';

const NOW = 1_700_000_000; // fixed reference

describe('apiKeys helpers', () => {
  it('formatRelative buckets seconds/minutes/hours/days', () => {
    expect(formatRelative(0, NOW)).toBe('never');
    expect(formatRelative(NOW - 30, NOW)).toBe('30s ago');
    expect(formatRelative(NOW - 120, NOW)).toBe('2m ago');
    expect(formatRelative(NOW - 7200, NOW)).toBe('2h ago');
    expect(formatRelative(NOW - 172800, NOW)).toBe('2d ago');
  });

  it('isExpired is false for null/future, true for past', () => {
    expect(isExpired(null, NOW)).toBe(false);
    expect(isExpired(NOW + 60, NOW)).toBe(false);
    expect(isExpired(NOW - 60, NOW)).toBe(true);
  });

  it('formatExpiry labels never/expired and otherwise a date', () => {
    expect(formatExpiry(null, NOW)).toBe('never');
    expect(formatExpiry(NOW - 60, NOW)).toBe('expired');
    expect(formatExpiry(NOW + 86400, NOW)).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it('toUnixExpiry round-trips a date string and rejects junk', () => {
    expect(toUnixExpiry('')).toBeNull();
    expect(toUnixExpiry('not-a-date')).toBeNull();
    const u = toUnixExpiry('2030-01-15');
    expect(typeof u).toBe('number');
    expect(u! > 1_800_000_000).toBe(true);
  });
});
