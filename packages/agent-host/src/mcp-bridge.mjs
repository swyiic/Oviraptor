// MCP stdio outside; private, scoped localhost HTTP/SSE inside. No credential logs.
// List-change wire shape: MCP 2025-06-18 server/tools#list-changed-notification.
const origin = process.env.OVIRAPTOR_HOST_ORIGIN;
const token = process.env.OVIRAPTOR_MCP_TOKEN;
const conversation = process.env.OVIRAPTOR_CONVERSATION;
const MAX_FRAME = 65_536, MAX_BUFFER = MAX_FRAME * 2, MAX_RESPONSE = MAX_FRAME * 2, MAX_CONCURRENT = 16;
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const utf8 = new TextDecoder('utf-8', { fatal: true });
let validOrigin = false;
try { validOrigin = /^http:\/\/127\.0\.0\.1:\d+$/.test(origin) && new URL(origin).port !== ''; } catch { /* Invalid host configuration. */ }
if (!validOrigin || typeof token !== 'string' || !token || token.length > 4096 || /[\r\n\0]/.test(token) || typeof conversation !== 'string' || !conversation || conversation.length > 256 || conversation.includes('\0')) process.exit(2);

const endpoint = `${origin}/mcp/${encodeURIComponent(conversation)}`;
const lifetime = new AbortController(), eventsAbort = new AbortController(), active = new Set();
let buffer = Buffer.alloc(0), ended = false, stopping = false, draining = false, initialized = false, eventStream = null;

function stop(code) {
  if (stopping) return;
  stopping = true; process.exitCode = code; lifetime.abort(); eventsAbort.abort();
  buffer = Buffer.alloc(0); process.stdin.destroy();
  if (!process.stdout.destroyed) process.stdout.end();
}
process.stdin.on('error', () => stop(2));
process.stdout.on('error', () => stop(2));
process.stderr.on('error', () => stop(2));
process.on('SIGTERM', () => stop(0)); process.on('SIGINT', () => stop(0));
process.once('exit', () => { lifetime.abort(); eventsAbort.abort(); });

function write(message) {
  if (stopping) return Promise.resolve(false);
  const frame = JSON.stringify(message) + '\n';
  if (Buffer.byteLength(frame) > MAX_RESPONSE) { stop(2); return Promise.resolve(false); }
  return new Promise(resolve => {
    try { process.stdout.write(frame, error => { if (error) stop(2); resolve(!error && !stopping); }); }
    catch { stop(2); resolve(false); }
  });
}

async function boundedJson(response) {
  if (!response.ok || !response.body) throw Error('unavailable');
  const reader = response.body.getReader(); let size = 0; const parts = [];
  try {
    for (;;) {
      const { done, value } = await reader.read(); if (done) break;
      size += value.byteLength; if (size > MAX_RESPONSE) throw Error('response limit'); parts.push(Buffer.from(value));
    }
    const value = JSON.parse(utf8.decode(Buffer.concat(parts, size)));
    if (!object(value)) throw Error('invalid response'); return value;
  } finally { await reader.cancel().catch(() => {}); }
}

async function subscribe() {
  const signal = AbortSignal.any([lifetime.signal, eventsAbort.signal]);
  const response = await fetch(`${endpoint}/events`, { headers: { Accept: 'text/event-stream', 'X-Oviraptor-Mcp': token }, signal, redirect: 'error', credentials: 'omit', cache: 'no-store' });
  if (!response.ok || !response.body || !response.headers.get('content-type')?.toLowerCase().startsWith('text/event-stream')) throw Error('events unavailable');
  const reader = response.body.getReader(); let bytes = Buffer.alloc(0), data = [], eventBytes = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read(); if (done) { if (!signal.aborted) throw Error('events ended'); return; }
      if (bytes.length + value.byteLength > MAX_BUFFER) throw Error('event buffer limit');
      bytes = Buffer.concat([bytes, value]);
      for (;;) {
        const index = bytes.indexOf(10); if (index < 0) break;
        if (index > MAX_FRAME) throw Error('event line limit');
        let line = utf8.decode(bytes.subarray(0, index)); bytes = bytes.subarray(index + 1); if (line.endsWith('\r')) line = line.slice(0, -1);
        if (!line) {
          if (data.length) {
            const value = JSON.parse(data.join('\n'));
            if (object(value) && value.type === 'tools.changed') {
              if (!await write({ jsonrpc: '2.0', method: 'notifications/tools/list_changed' })) return;
            }
          }
          data = []; eventBytes = 0;
        } else if (line.startsWith('data:')) {
          const payload = line.slice(5).replace(/^ /, ''); eventBytes += Buffer.byteLength(payload) + 1;
          if (eventBytes > MAX_FRAME) throw Error('event limit'); data.push(payload);
        }
      }
      if (bytes.length > MAX_FRAME) throw Error('event line limit');
    }
  } finally { await reader.cancel().catch(() => {}); }
}
function startEvents() {
  if (eventStream || !initialized || ended || stopping) return;
  eventStream = subscribe().catch(() => { if (!ended && !lifetime.signal.aborted && !eventsAbort.signal.aborted) stop(2); });
}

async function request(message, key) {
  try {
    const response = await fetch(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Oviraptor-Mcp': token },
      body: JSON.stringify({ method: message.method, params: message.params ?? {} }),
      signal: AbortSignal.any([lifetime.signal, AbortSignal.timeout(30_000)]), redirect: 'error', credentials: 'omit' });
    const result = await boundedJson(response);
    if (await write({ jsonrpc: '2.0', id: message.id, result }) && message.method === 'initialize') { initialized = true; startEvents(); }
  } catch {
    await write({ jsonrpc: '2.0', id: message.id, error: { code: -32000, message: 'Scoped application capability unavailable' } });
  } finally { active.delete(key); drain(); }
}

function accept(frame) {
  let message;
  try { message = JSON.parse(utf8.decode(frame)); } catch { stop(2); return; }
  if (!object(message) || message.jsonrpc !== '2.0' || typeof message.method !== 'string' || !message.method ||
      Object.keys(message).some(key => !['jsonrpc', 'method', 'id', 'params'].includes(key)) ||
      (message.params !== undefined && !object(message.params))) { stop(2); return; }
  if (!Object.hasOwn(message, 'id')) { if (message.method === 'notifications/initialized') startEvents(); return; }
  if (!(typeof message.id === 'string' || Number.isSafeInteger(message.id))) { stop(2); return; }
  const key = `${typeof message.id}:${message.id}`;
  if (active.has(key)) { stop(2); return; }
  active.add(key); void request(message, key);
}
function validateFrames() {
  let start = 0;
  for (;;) {
    const end = buffer.indexOf(10, start); if (end < 0) return buffer.length - start <= MAX_FRAME;
    if (end - start > MAX_FRAME) return false; start = end + 1;
  }
}
function drain() {
  if (stopping || draining) return;
  draining = true;
  try {
    while (!stopping && active.size < MAX_CONCURRENT) {
      const end = buffer.indexOf(10); if (end < 0) break;
      let frame = buffer.subarray(0, end); buffer = buffer.subarray(end + 1);
      if (frame.at(-1) === 13) frame = frame.subarray(0, frame.length - 1);
      if (!frame.length) { stop(2); break; } accept(frame);
    }
    if (stopping) return;
    if (ended) { if (buffer.length && buffer.indexOf(10) < 0) stop(2); else if (!active.size && !buffer.length) stop(0); }
    else if (active.size >= MAX_CONCURRENT) process.stdin.pause(); else process.stdin.resume();
  } finally { draining = false; }
}
process.stdin.on('data', chunk => {
  if (stopping) return;
  if (buffer.length + chunk.length > MAX_BUFFER) { stop(2); return; }
  buffer = Buffer.concat([buffer, chunk]);
  if (!validateFrames()) { stop(2); return; } drain();
});
process.stdin.on('end', () => { ended = true; eventsAbort.abort(); drain(); });
