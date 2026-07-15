import { describe, it, expect } from 'vitest';
import { reconnectDelay } from '../backoff';

describe('reconnectDelay', () => {
  it('doubles each attempt starting at 1s', () => {
    expect(reconnectDelay(0)).toBe(1000);
    expect(reconnectDelay(1)).toBe(2000);
    expect(reconnectDelay(2)).toBe(4000);
    expect(reconnectDelay(3)).toBe(8000);
  });

  it('caps at 15s and clamps the attempt count', () => {
    expect(reconnectDelay(4)).toBe(15000); // 16000 -> capped
    expect(reconnectDelay(5)).toBe(15000); // 32000 -> capped
    expect(reconnectDelay(50)).toBe(15000); // clamp prevents overflow
  });

  it('treats negative attempts as zero', () => {
    expect(reconnectDelay(-3)).toBe(1000);
  });
});
