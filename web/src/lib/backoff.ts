/**
 * Exponential reconnect backoff for the WebSocket provider: 1s, 2s, 4s, …
 * doubling per attempt, capped at 15s. The attempt count is clamped to [0, 5]
 * so the delay plateaus at the cap instead of growing unboundedly (and a stray
 * negative attempt can't produce a sub-second delay).
 */
export function reconnectDelay(attempt: number): number {
  const clamped = Math.min(Math.max(attempt, 0), 5);
  return Math.min(1000 * 2 ** clamped, 15000);
}
