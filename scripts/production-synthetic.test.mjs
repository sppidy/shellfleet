import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  decodeWebSocketFrame,
  loadCredentialSecrets,
  mintSessionJwt,
  parseSseEvent,
  validateFleet,
  validatePasskeyResponse,
} from './production-synthetic.mjs';

test('loads secrets from systemd credentials without replacing explicit values', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'shellfleet-synthetic-'));
  try {
    await writeFile(join(directory, 'jwt-secret'), `${'c'.repeat(32)}\n`, { mode: 0o600 });
    await writeFile(join(directory, 'telegram-bot-token'), 'credential-token\n', { mode: 0o600 });
    const loaded = await loadCredentialSecrets({
      CREDENTIALS_DIRECTORY: directory,
      TELEGRAM_BOT_TOKEN: 'explicit-token',
    });
    assert.equal(loaded.JWT_SECRET, 'c'.repeat(32));
    assert.equal(loaded.TELEGRAM_BOT_TOKEN, 'explicit-token');
    assert.equal(loaded.TELEGRAM_CHAT_ID, undefined);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('mints a short-lived, MFA-verified browser session JWT', () => {
  const secret = 'a'.repeat(32);
  const token = mintSessionJwt({ secret, login: 'monitor', role: 'viewer', now: 100 });
  const [header, payload, signature] = token.split('.');
  assert.deepEqual(JSON.parse(Buffer.from(header, 'base64url')), { alg: 'HS256', typ: 'JWT' });
  assert.deepEqual(JSON.parse(Buffer.from(payload, 'base64url')), {
    sub: 'monitor', exp: 160, iat: 100, role: 'viewer', mfa: true, cli: false,
  });
  assert.equal(signature, createHmac('sha256', secret).update(`${header}.${payload}`).digest('base64url'));
});

test('requires fresh snapshots from the configured online fleet floor', () => {
  const payload = {
    hosts: [{ status: 'online', system: { observed_at: 980 } }, { status: 'offline', system: null }],
  };
  assert.deepEqual(validateFleet(payload, { minOnline: 1, maxSnapshotAgeSecs: 30, now: 1_000 }), {
    online: 1, freshSnapshots: 1, hosts: 2,
  });
  assert.throws(() => validateFleet(payload, { minOnline: 2, now: 1_000 }), /expected at least 2/);
  assert.throws(() => validateFleet(payload, { minOnline: 1, maxSnapshotAgeSecs: 10, now: 1_000 }), /no online host/);
});

test('validates the passkey RP id and challenge shape', () => {
  const payload = {
    state_id: 'state-identifier',
    options: { publicKey: { challenge: 'challenge-value', rpId: 'fleet.example.com' } },
  };
  assert.deepEqual(validatePasskeyResponse(payload, 'fleet.example.com'), { rpId: 'fleet.example.com' });
  assert.throws(() => validatePasskeyResponse(payload, 'wrong.example.com'), /expected wrong/);
});

test('parses fleet SSE messages and ignores comments', () => {
  assert.deepEqual(parseSseEvent(': keepalive\nevent: fleet\ndata: {"kind":"host_updated"}'), {
    event: 'fleet', data: '{"kind":"host_updated"}',
  });
  assert.equal(parseSseEvent(': keepalive'), null);
});

test('decodes an extended-length unmasked WebSocket text frame', () => {
  const payload = Buffer.from('x'.repeat(130));
  const frame = Buffer.alloc(4 + payload.length);
  frame[0] = 0x81;
  frame[1] = 126;
  frame.writeUInt16BE(payload.length, 2);
  payload.copy(frame, 4);
  const decoded = decodeWebSocketFrame(frame);
  assert.equal(decoded.fin, true);
  assert.equal(decoded.opcode, 1);
  assert.deepEqual(decoded.payload, payload);
  assert.equal(decoded.consumed, frame.length);
});
