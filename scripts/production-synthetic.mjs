#!/usr/bin/env node

import { createHash, createHmac, randomBytes } from 'node:crypto';
import { promises as fs } from 'node:fs';
import { dirname } from 'node:path';
import tls from 'node:tls';
import { fileURLToPath } from 'node:url';

const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const MAX_STREAM_BUFFER_BYTES = 1024 * 1024;

function required(value, name) {
  if (!value?.trim()) throw new Error(`${name} is required`);
  return value.trim();
}

function positiveInteger(value, fallback, name) {
  if (value == null || value === '') return fallback;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  return parsed;
}

export function mintSessionJwt({ secret, login, role = 'viewer', now = Math.floor(Date.now() / 1000) }) {
  if (secret !== 'dev' && secret.length < 32) {
    throw new Error('JWT_SECRET must be at least 32 characters');
  }
  if (!['admin', 'viewer'].includes(role)) {
    throw new Error('SHELLFLEET_SYNTHETIC_ROLE must be admin or viewer');
  }
  const encode = (value) => Buffer.from(JSON.stringify(value)).toString('base64url');
  const unsigned = `${encode({ alg: 'HS256', typ: 'JWT' })}.${encode({
    sub: login,
    exp: now + 60,
    iat: now,
    role,
    mfa: true,
    cli: false,
  })}`;
  const signature = createHmac('sha256', secret).update(unsigned).digest('base64url');
  return `${unsigned}.${signature}`;
}

export function validateFleet(payload, { minOnline = 1, maxSnapshotAgeSecs = 60, now = Math.floor(Date.now() / 1000) } = {}) {
  if (!payload || !Array.isArray(payload.hosts)) {
    throw new Error('fleet response does not contain a hosts array');
  }
  const online = payload.hosts.filter((host) => host?.status === 'online');
  if (online.length < minOnline) {
    throw new Error(`fleet has ${online.length} online host(s); expected at least ${minOnline}`);
  }
  const fresh = online.filter((host) => {
    const observedAt = host?.system?.observed_at;
    return Number.isFinite(observedAt) && now - observedAt <= maxSnapshotAgeSecs;
  });
  if (minOnline > 0 && fresh.length === 0) {
    throw new Error(`no online host has a system snapshot newer than ${maxSnapshotAgeSecs}s`);
  }
  return { online: online.length, freshSnapshots: fresh.length, hosts: payload.hosts.length };
}

export function validatePasskeyResponse(payload, expectedRpId) {
  const stateId = payload?.state_id;
  const publicKey = payload?.options?.publicKey;
  if (typeof stateId !== 'string' || stateId.length < 8) {
    throw new Error('passkey begin response has no usable state_id');
  }
  if (typeof publicKey?.challenge !== 'string' || publicKey.challenge.length < 8) {
    throw new Error('passkey begin response has no usable challenge');
  }
  if (publicKey.rpId !== expectedRpId) {
    throw new Error(`passkey RP id is ${JSON.stringify(publicKey.rpId)}; expected ${expectedRpId}`);
  }
  return { rpId: publicKey.rpId };
}

export function parseSseEvent(block) {
  let event = 'message';
  const data = [];
  for (const rawLine of block.split(/\r?\n/)) {
    const line = rawLine.endsWith('\r') ? rawLine.slice(0, -1) : rawLine;
    if (line.startsWith(':')) continue;
    const separator = line.indexOf(':');
    const field = separator === -1 ? line : line.slice(0, separator);
    let value = separator === -1 ? '' : line.slice(separator + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    if (field === 'event') event = value;
    if (field === 'data') data.push(value);
  }
  if (data.length === 0) return null;
  return { event, data: data.join('\n') };
}

export function decodeWebSocketFrame(buffer) {
  if (buffer.length < 2) return null;
  const first = buffer[0];
  const second = buffer[1];
  const fin = (first & 0x80) !== 0;
  const opcode = first & 0x0f;
  const masked = (second & 0x80) !== 0;
  let length = second & 0x7f;
  let offset = 2;
  if (length === 126) {
    if (buffer.length < 4) return null;
    length = buffer.readUInt16BE(2);
    offset = 4;
  } else if (length === 127) {
    if (buffer.length < 10) return null;
    const wideLength = buffer.readBigUInt64BE(2);
    if (wideLength > BigInt(1024 * 1024)) throw new Error('WebSocket frame exceeds 1 MiB');
    length = Number(wideLength);
    offset = 10;
  }
  let mask;
  if (masked) {
    if (buffer.length < offset + 4) return null;
    mask = buffer.subarray(offset, offset + 4);
    offset += 4;
  }
  if (buffer.length < offset + length) return null;
  const payload = Buffer.from(buffer.subarray(offset, offset + length));
  if (mask) {
    for (let index = 0; index < payload.length; index += 1) {
      payload[index] ^= mask[index % 4];
    }
  }
  return { fin, opcode, payload, consumed: offset + length };
}

async function responseError(response, label) {
  let detail = '';
  try {
    detail = (await response.text()).replace(/\s+/g, ' ').slice(0, 180);
  } catch {
    // The status is enough when the response body cannot be read.
  }
  throw new Error(`${label} returned HTTP ${response.status}${detail ? `: ${detail}` : ''}`);
}

async function fetchWithTimeout(url, options, timeoutMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new Error(`request timed out after ${timeoutMs}ms`)), timeoutMs);
  try {
    // Never forward the short-lived session cookie or notification credential
    // through an unexpected redirect. The configured production origin is the
    // only intended destination for authenticated probes.
    return await fetch(url, { redirect: 'manual', ...options, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

async function fetchFleet(baseUrl, cookie, timeoutMs) {
  const response = await fetchWithTimeout(new URL('/api/core/v1/fleet', baseUrl), {
    headers: { Accept: 'application/json', Cookie: cookie },
  }, timeoutMs);
  if (!response.ok) await responseError(response, 'fleet REST probe');
  return response.json();
}

async function probePasskey(baseUrl, expectedRpId, timeoutMs) {
  const response = await fetchWithTimeout(new URL('/api/auth/passkey/login/begin', baseUrl), {
    method: 'POST',
    headers: { Accept: 'application/json', Origin: baseUrl.origin },
  }, timeoutMs);
  if (!response.ok) await responseError(response, 'passkey begin probe');
  return validatePasskeyResponse(await response.json(), expectedRpId);
}

async function probeSse(baseUrl, cookie, timeoutMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new Error(`SSE timed out after ${timeoutMs}ms`)), timeoutMs);
  let reader;
  try {
    const response = await fetch(new URL('/api/core/v1/events', baseUrl), {
      headers: { Accept: 'text/event-stream', Cookie: cookie, 'Cache-Control': 'no-cache' },
      redirect: 'manual',
      signal: controller.signal,
    });
    if (!response.ok) await responseError(response, 'fleet SSE probe');
    if (!response.headers.get('content-type')?.toLowerCase().includes('text/event-stream')) {
      throw new Error('fleet SSE probe returned the wrong content type');
    }
    if (!response.body) throw new Error('fleet SSE probe returned no response stream');
    reader = response.body.getReader();
    const decoder = new TextDecoder();
    let pending = '';
    while (true) {
      const { done, value } = await reader.read();
      if (done) throw new Error('fleet SSE stream closed before a fleet event');
      pending += decoder.decode(value, { stream: true }).replace(/\r\n/g, '\n');
      if (Buffer.byteLength(pending) > MAX_STREAM_BUFFER_BYTES) {
        throw new Error('fleet SSE event exceeds 1 MiB');
      }
      let boundary;
      while ((boundary = pending.indexOf('\n\n')) !== -1) {
        const parsed = parseSseEvent(pending.slice(0, boundary));
        pending = pending.slice(boundary + 2);
        if (!parsed || parsed.event !== 'fleet') continue;
        const event = JSON.parse(parsed.data);
        if (typeof event?.kind !== 'string' || typeof event?.observed_at !== 'number') {
          throw new Error('fleet SSE event has an invalid payload');
        }
        return event;
      }
    }
  } finally {
    clearTimeout(timer);
    controller.abort();
    await reader?.cancel().catch(() => {});
  }
}

async function probeWebSocket(baseUrl, cookie, timeoutMs) {
  const wsUrl = new URL('/ui/ws', baseUrl);
  wsUrl.protocol = 'wss:';
  const port = Number(wsUrl.port || 443);
  const key = randomBytes(16).toString('base64');
  const expectedAccept = createHash('sha1').update(`${key}${WS_GUID}`).digest('base64');

  return new Promise((resolve, reject) => {
    const socket = tls.connect({ host: wsUrl.hostname, port, servername: wsUrl.hostname });
    let settled = false;
    let headersParsed = false;
    let pending = Buffer.alloc(0);
    const timer = setTimeout(() => finish(new Error(`WebSocket timed out after ${timeoutMs}ms`)), timeoutMs);

    function finish(error, value) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      if (error) reject(error);
      else resolve(value);
    }

    socket.once('secureConnect', () => {
      const path = `${wsUrl.pathname}${wsUrl.search}`;
      socket.write([
        `GET ${path} HTTP/1.1`,
        `Host: ${wsUrl.host}`,
        'Connection: Upgrade',
        'Upgrade: websocket',
        'Sec-WebSocket-Version: 13',
        `Sec-WebSocket-Key: ${key}`,
        `Origin: ${baseUrl.origin}`,
        `Cookie: ${cookie}`,
        'User-Agent: shellfleet-production-synthetic/1',
        '',
        '',
      ].join('\r\n'));
    });
    socket.on('error', (error) => finish(new Error(`WebSocket transport failed: ${error.message}`)));
    socket.on('end', () => finish(new Error('WebSocket closed before the initial agent list')));
    socket.on('data', (chunk) => {
      pending = Buffer.concat([pending, chunk]);
      if (pending.length > MAX_STREAM_BUFFER_BYTES) {
        return finish(new Error('WebSocket response exceeds 1 MiB'));
      }
      if (!headersParsed) {
        const marker = pending.indexOf('\r\n\r\n');
        if (marker === -1) return;
        const headerText = pending.subarray(0, marker).toString('latin1');
        pending = pending.subarray(marker + 4);
        const lines = headerText.split('\r\n');
        if (!/^HTTP\/1\.[01] 101\b/.test(lines[0])) {
          return finish(new Error(`WebSocket upgrade returned ${lines[0] || 'an invalid response'}`));
        }
        const headers = new Map(lines.slice(1).map((line) => {
          const separator = line.indexOf(':');
          return [line.slice(0, separator).trim().toLowerCase(), line.slice(separator + 1).trim()];
        }));
        if (headers.get('sec-websocket-accept') !== expectedAccept) {
          return finish(new Error('WebSocket upgrade returned an invalid accept key'));
        }
        headersParsed = true;
      }

      while (headersParsed) {
        let frame;
        try {
          frame = decodeWebSocketFrame(pending);
        } catch (error) {
          return finish(error);
        }
        if (!frame) return;
        pending = pending.subarray(frame.consumed);
        if (frame.opcode === 0x8) return finish(new Error('WebSocket closed before the initial agent list'));
        if (frame.opcode !== 0x1) continue;
        if (!frame.fin) return finish(new Error('fragmented WebSocket response is not supported by the probe'));
        let message;
        try {
          message = JSON.parse(frame.payload.toString('utf8'));
        } catch {
          return finish(new Error('WebSocket returned non-JSON text'));
        }
        if (message?.type !== 'ListAgentsResponse' || !Array.isArray(message?.payload?.agents)) {
          return finish(new Error('WebSocket did not return the initial agent list'));
        }
        return finish(null, { agents: message.payload.agents.length });
      }
    });
  });
}

async function readState(path) {
  try {
    return JSON.parse(await fs.readFile(path, 'utf8'));
  } catch {
    return null;
  }
}

async function writeState(path, state) {
  const temporary = `${path}.${process.pid}.tmp`;
  await fs.mkdir(dirname(path), { recursive: true });
  await fs.writeFile(temporary, `${JSON.stringify(state)}\n`, { mode: 0o600 });
  await fs.rename(temporary, path);
}

async function sendTelegram(message, env, timeoutMs) {
  const token = env.TELEGRAM_BOT_TOKEN?.trim();
  const chatId = env.TELEGRAM_CHAT_ID?.trim();
  if (!token || !chatId) return false;
  const response = await fetchWithTimeout(`https://api.telegram.org/bot${encodeURIComponent(token)}/sendMessage`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ chat_id: chatId, text: message, disable_web_page_preview: true }),
  }, timeoutMs);
  if (!response.ok) throw new Error(`Telegram alert returned HTTP ${response.status}`);
  return true;
}

export async function loadCredentialSecrets(env) {
  const resolved = { ...env };
  const credentialsDirectory = env.CREDENTIALS_DIRECTORY?.trim();
  if (!credentialsDirectory) return resolved;
  const mappings = [
    ['JWT_SECRET', 'jwt-secret'],
    ['TELEGRAM_BOT_TOKEN', 'telegram-bot-token'],
    ['TELEGRAM_CHAT_ID', 'telegram-chat-id'],
  ];
  for (const [environmentName, credentialName] of mappings) {
    if (resolved[environmentName]?.trim()) continue;
    try {
      resolved[environmentName] = (await fs.readFile(`${credentialsDirectory}/${credentialName}`, 'utf8')).trim();
    } catch (error) {
      if (environmentName === 'JWT_SECRET') {
        throw new Error(`required systemd credential ${credentialName} is unavailable: ${error.code || error.message}`);
      }
    }
  }
  return resolved;
}

function safeError(error) {
  return String(error?.message || error || 'unknown failure').replace(/[\r\n]+/g, ' ').slice(0, 500);
}

export async function runSynthetic(env = process.env) {
  const baseUrl = new URL(required(env.SHELLFLEET_SYNTHETIC_BASE_URL || env.UI_URL, 'SHELLFLEET_SYNTHETIC_BASE_URL or UI_URL'));
  if (baseUrl.protocol !== 'https:') throw new Error('production synthetic base URL must use HTTPS');
  baseUrl.pathname = '/';
  baseUrl.search = '';
  baseUrl.hash = '';
  const login = required(env.SHELLFLEET_SYNTHETIC_LOGIN || env.ALLOWED_GITHUB_USERS?.split(',')[0], 'SHELLFLEET_SYNTHETIC_LOGIN or ALLOWED_GITHUB_USERS');
  const secret = required(env.JWT_SECRET, 'JWT_SECRET');
  const role = env.SHELLFLEET_SYNTHETIC_ROLE?.trim() || 'viewer';
  const timeoutMs = positiveInteger(env.SHELLFLEET_SYNTHETIC_TIMEOUT_MS, 25_000, 'SHELLFLEET_SYNTHETIC_TIMEOUT_MS');
  const minOnline = positiveInteger(env.SHELLFLEET_SYNTHETIC_MIN_ONLINE, 1, 'SHELLFLEET_SYNTHETIC_MIN_ONLINE');
  const maxSnapshotAgeSecs = positiveInteger(env.SHELLFLEET_SYNTHETIC_MAX_SNAPSHOT_AGE_SECS, 60, 'SHELLFLEET_SYNTHETIC_MAX_SNAPSHOT_AGE_SECS');
  const expectedRpId = env.SHELLFLEET_SYNTHETIC_RP_ID?.trim() || baseUrl.hostname;
  const token = mintSessionJwt({ secret, login, role });
  const cookie = `__Host-auth_token=${token}`;
  const startedAt = Date.now();

  const firstFleet = await fetchFleet(baseUrl, cookie, timeoutMs);
  const fleet = validateFleet(firstFleet, { minOnline, maxSnapshotAgeSecs });
  const passkey = await probePasskey(baseUrl, expectedRpId, timeoutMs);
  const webSocket = await probeWebSocket(baseUrl, cookie, timeoutMs);
  const event = await probeSse(baseUrl, cookie, timeoutMs);
  const secondFleet = await fetchFleet(baseUrl, cookie, timeoutMs);
  const eventHost = event.agent_id && secondFleet.hosts.find((host) => host.agent_id === event.agent_id);
  const firstEventHost = event.agent_id && firstFleet.hosts.find((host) => host.agent_id === event.agent_id);
  if (event.kind === 'host_updated' && firstEventHost && eventHost && eventHost.last_seen_at <= firstEventHost.last_seen_at) {
    throw new Error(`SSE reported ${event.agent_id} without advancing its durable fleet timestamp`);
  }

  return {
    baseUrl: baseUrl.origin,
    durationMs: Date.now() - startedAt,
    fleet,
    passkey,
    webSocket,
    sse: { kind: event.kind, agentId: event.agent_id || null },
  };
}

async function main() {
  const env = await loadCredentialSecrets(process.env);
  const statePath = env.SHELLFLEET_SYNTHETIC_STATE_FILE || '/var/lib/shellfleet-synthetic/status.json';
  const previous = await readState(statePath);
  const now = Math.floor(Date.now() / 1000);
  const alertCooldown = positiveInteger(env.SHELLFLEET_SYNTHETIC_ALERT_COOLDOWN_SECS, 3600, 'SHELLFLEET_SYNTHETIC_ALERT_COOLDOWN_SECS');
  try {
    const result = await runSynthetic(env);
    if (previous?.status === 'degraded') {
      await sendTelegram(`ShellFleet production probe recovered\n${result.baseUrl}\nREST, SSE, WebSocket, and passkey begin are healthy.`, env, 10_000)
        .catch((error) => console.error(`recovery notification failed: ${safeError(error)}`));
    }
    await writeState(statePath, { status: 'healthy', checked_at: now }).catch((error) => {
      console.error(`state update failed: ${safeError(error)}`);
    });
    console.log(JSON.stringify({ ok: true, ...result }));
  } catch (error) {
    const reason = safeError(error);
    const shouldAlert = previous?.status !== 'degraded' || now - Number(previous?.alerted_at || 0) >= alertCooldown;
    let alertedAt = Number(previous?.alerted_at || 0);
    if (shouldAlert) {
      const sent = await sendTelegram(`ShellFleet production probe failed\n${reason}`, env, 10_000)
        .catch((alertError) => {
          console.error(`failure notification failed: ${safeError(alertError)}`);
          return false;
        });
      if (sent) alertedAt = now;
    }
    await writeState(statePath, { status: 'degraded', checked_at: now, alerted_at: alertedAt, reason }).catch((stateError) => {
      console.error(`state update failed: ${safeError(stateError)}`);
    });
    console.error(JSON.stringify({ ok: false, error: reason }));
    process.exitCode = 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  await main();
}
