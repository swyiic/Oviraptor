import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { EventEmitter } from 'node:events';
import { GrokAcpRuntime } from '../index.mjs';

const fixture = fileURLToPath(new URL('./fixtures/agent.mjs', import.meta.url));
function runtime(t, options = {}, mode = 'normal') {
  const events = [], children = [];
  const instance = new GrokAcpRuntime({ cwd: '/tmp', requestTimeoutMs: 1_000, promptTimeoutMs: 2_000, cancelTimeoutMs: 200,
    spawn(command, args, config) { assert.equal(command, 'grok'); assert.deepEqual(args, ['--no-auto-update', 'agent', 'stdio']); assert.equal(config.shell, false); const child = spawn(process.execPath, [fixture, mode], config); children.push(child); return child; },
    ...options, onEvent(event) { events.push(event); options.onEvent?.(event); } });
  t.after(() => instance.close()); return { instance, events, children };
}
function until(predicate) {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + 2_000;
    const check = () => { if (predicate()) return resolve(); if (Date.now() > deadline) return reject(Error('Fixture observation deadline exceeded')); setTimeout(check, 5); }; check();
  });
}

function memoryAgent({ onRequest, afterResponse } = {}) {
  const children = [], requests = [];
  return { children, requests, spawn() {
    const child = new EventEmitter(); child.stdout = new EventEmitter(); child.stderr = new EventEmitter(); child.stdin = new EventEmitter(); child.callbacks = []; child.kills = [];
    child.send = message => child.stdout.emit('data', Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n'));
    child.stdin.write = (frame, callback) => {
      child.callbacks.push(callback); const request = JSON.parse(frame); requests.push(request);
      queueMicrotask(() => {
        if (onRequest?.(request, child) === false || !Object.hasOwn(request, 'id')) return;
        const result = request.method === 'initialize' ? { protocolVersion: 1, authMethods: [{ id: 'cached_token' }], agentCapabilities: { loadSession: true } }
          : request.method === 'session/new' ? { sessionId: `memory-session-${children.indexOf(child)}` } : {};
        child.send({ id: request.id, result }); afterResponse?.(request, child);
      });
    };
    child.kill = signal => { child.kills.push(signal); queueMicrotask(() => child.emit('exit', 0)); return true; }; children.push(child); return child;
  } };
}

test('one real process initializes/authenticates once, streams UTF-8 updates and reuses sessions', async t => {
  const { instance, events, children } = runtime(t);
  await Promise.all([instance.start(), instance.start()]);
  const one = await instance.newSession(), two = await instance.newSession();
  let complete = false;
  const first = instance.prompt({ sessionId: one.sessionId, prompt: 'stream' }).then(result => { complete = true; return result; });
  await until(() => events.some(event => event.type === 'text_delta'));
  assert.equal(complete, false, 'Text is delivered before completion metadata');
  const result = await first;
  await instance.prompt({ sessionId: two.sessionId, prompt: 'second' });
  assert.equal(children.length, 1); assert.equal(events.filter(event => event.type === 'runtime_ready').length, 1);
  assert.equal(result.stopReason, 'end_turn'); assert.equal(events.find(event => event.type === 'text_delta').text, '你好 ');
  assert(events.some(event => event.type === 'tool_call')); assert(events.some(event => event.type === 'tool_update'));
  for (const value of Object.values(result.metrics)) assert(value >= 0);
  assert(result.metrics.firstTextMs < result.metrics.totalMs); assert(result.metrics.firstToolMs < result.metrics.totalMs);
  assert.equal(instance.status.pendingRequests, 0);
});

test('same-session prompts serialize, distinct sessions retain their event identity', async t => {
  const { instance, events } = runtime(t), { sessionId } = await instance.newSession();
  const [first, second] = await Promise.all([instance.prompt({ sessionId, prompt: 'one', turnId: 'first' }), instance.prompt({ sessionId, prompt: 'two', turnId: 'second' })]);
  assert.equal(first.stopReason, 'end_turn'); assert.equal(second.stopReason, 'end_turn');
  const ordering = events.filter(event => ['turn_started', 'turn_complete'].includes(event.type)).map(event => `${event.type}:${event.turnId}`);
  assert.deepEqual(ordering, ['turn_started:first', 'turn_complete:first', 'turn_started:second', 'turn_complete:second']);
  assert(events.filter(event => event.type === 'text_delta').every(event => event.sessionId === sessionId));
  const other = await instance.newSession();
  await Promise.all([instance.prompt({ sessionId, prompt: 'A', turnId: 'scope-A' }), instance.prompt({ sessionId: other.sessionId, prompt: 'B', turnId: 'scope-B' })]);
  assert(events.filter(event => event.turnId === 'scope-A').every(event => event.sessionId === sessionId));
  assert(events.filter(event => event.turnId === 'scope-B').every(event => event.sessionId === other.sessionId));
});

test('default permissions reject once; absent reject option cancels and never allows', async t => {
  for (const mode of ['normal', 'allow-only']) {
    const { instance, events } = runtime(t, {}, mode), { sessionId } = await instance.newSession();
    await instance.prompt({ sessionId, prompt: 'permission' });
    const decision = events.find(event => event.type === 'permission_resolved').outcome;
    assert.deepEqual(decision, mode === 'normal' ? { outcome: 'selected', optionId: 'no' } : { outcome: 'cancelled' });
  }
});

test('permission callback only selects an offered option and callback rejection defaults to denial', async t => {
  for (const decision of ['yes', 'invented', 'throw']) {
    const { instance, events } = runtime(t, { onPermission: async () => { if (decision === 'throw') throw Error('Synthetic UI failure'); return decision; } }), { sessionId } = await instance.newSession();
    await instance.prompt({ sessionId, prompt: 'permission' });
    assert.equal(events.find(event => event.type === 'permission_resolved').outcome.optionId, decision === 'yes' ? 'yes' : 'no');
  }
});

test('cancellation settles pending/new permissions, ignores a late approval, accepts final updates and reuses process', async t => {
  let held;
  const { instance, events, children } = runtime(t, { onPermission: () => new Promise(resolve => { held = resolve; }) });
  const { sessionId } = await instance.newSession(); const pending = instance.prompt({ sessionId, prompt: 'permission-cancel' });
  await until(() => held); assert.equal(instance.cancel(sessionId), true);
  held('yes'); const result = await pending;
  assert.equal(result.stopReason, 'cancelled');
  const decisions = events.filter(event => event.type === 'permission_resolved'); assert.equal(decisions.length, 2);
  assert(decisions.every(event => event.outcome.outcome === 'cancelled')); assert.equal(instance.status.pendingPermissions, 0);
  await instance.prompt({ sessionId, prompt: 'next' }); assert.equal(children.length, 1);
});

test('host approval via event request ID is explicit, bounded, and single-use', async t => {
  const { instance, events } = runtime(t, { permissionPolicy: 'ask' }), { sessionId } = await instance.newSession();
  const pending = instance.prompt({ sessionId, prompt: 'permission' });
  await until(() => events.some(event => event.type === 'permission_requested'));
  const requestId = events.find(event => event.type === 'permission_requested').requestId;
  assert.equal(instance.resolvePermission(requestId, 'yes'), true); assert.equal(instance.resolvePermission(requestId, 'yes'), false);
  assert.equal((await pending).stopReason, 'end_turn');
});

test('approval deadline defaults to rejection', async t => {
  const { instance, events } = runtime(t, { permissionPolicy: 'ask', permissionTimeoutMs: 20 }), { sessionId } = await instance.newSession();
  await instance.prompt({ sessionId, prompt: 'permission' });
  assert.equal(events.find(event => event.type === 'permission_resolved').outcome.optionId, 'no');
});

test('synchronous turn-start cancellation is sent after the prompt and is acknowledged', async t => {
  let instance;
  ({ instance } = runtime(t, { onEvent(event) { if (event.type === 'turn_started') instance.cancel(event.sessionId); } }));
  const { sessionId } = await instance.newSession();
  assert.equal((await instance.prompt({ sessionId, prompt: 'wait' })).stopReason, 'cancelled');
});

test('AbortSignal cancellation and an aborted queued prompt never overlap turns', async t => {
  const { instance, events } = runtime(t), { sessionId } = await instance.newSession();
  const active = new AbortController(), queued = new AbortController();
  const first = instance.prompt({ sessionId, prompt: 'wait', signal: active.signal });
  const second = instance.prompt({ sessionId, prompt: 'must-not-send', signal: queued.signal });
  const rejection = assert.rejects(second, { code: 'ACP_ABORTED' }); queued.abort();
  await until(() => events.some(event => event.type === 'text_delta')); active.abort();
  assert.equal((await first).stopReason, 'cancelled'); await rejection;
  assert.equal(events.filter(event => event.type === 'turn_started').length, 1);
});

test('prompt timeout cancels before rejecting; an unresponsive cancel kills and quarantines the process', async t => {
  const { instance } = runtime(t), { sessionId } = await instance.newSession();
  await assert.rejects(instance.prompt({ sessionId, prompt: 'hang', timeoutMs: 20 }), { code: 'ACP_PROMPT_TIMEOUT' });
  assert.equal(instance.status.state, 'ready');
  await assert.rejects(instance.prompt({ sessionId, prompt: 'ignore-cancel', timeoutMs: 20 }), { code: 'ACP_CANCEL_TIMEOUT' });
  assert.equal(instance.status.state, 'failed');
  await assert.rejects(instance.prompt({ sessionId, prompt: 'never-replay' }), { code: 'ACP_SESSION_NEEDS_LOAD' });
});

test('crash rejects the turn; explicit restart+capability-gated load replays history but never replays a prompt', async t => {
  const { instance, events, children } = runtime(t), { sessionId } = await instance.newSession();
  await assert.rejects(instance.prompt({ sessionId, prompt: 'crash' }), error => ['ACP_PROCESS_EXIT'].includes(error.code));
  await instance.start(); await assert.rejects(instance.prompt({ sessionId, prompt: 'not-loaded' }), { code: 'ACP_SESSION_NEEDS_LOAD' });
  await instance.loadSession({ sessionId });
  assert(events.some(event => event.type === 'text_delta' && event.replay && event.text === 'Synthetic restored history'));
  await instance.prompt({ sessionId, prompt: 'restored' }); assert.equal(children.length, 2);
});

test('load unsupported and cross-directory restores are explicit failures', async t => {
  const { instance } = runtime(t, {}, 'no-load'), { sessionId } = await instance.newSession();
  await assert.rejects(instance.loadSession({ sessionId }), { code: 'ACP_LOAD_UNSUPPORTED' });
  await assert.rejects(instance.loadSession({ sessionId, cwd: '/different' }), { code: 'ACP_SCOPE_MISMATCH' });
});

test('version mismatch and missing cached auth fail without creating a session', async t => {
  for (const [mode, code] of [['version-2', 'ACP_VERSION_UNSUPPORTED'], ['no-auth', 'ACP_CACHED_AUTH_UNAVAILABLE'], ['init-hang', 'ACP_REQUEST_TIMEOUT']]) {
    const { instance } = runtime(t, { requestTimeoutMs: 80 }, mode);
    await assert.rejects(instance.newSession(), { code }); assert.equal(instance.status.state, 'failed');
  }
});

test('invalid JSON/envelopes and oversized frames fail closed with sanitized diagnostics', async t => {
  for (const prompt of ['bad-json', 'bad-envelope', 'oversized']) {
    const { instance } = runtime(t), { sessionId } = await instance.newSession();
    await assert.rejects(instance.prompt({ sessionId, prompt }), { code: 'ACP_PROTOCOL_ERROR' }); assert.equal(instance.status.state, 'failed');
  }
});

test('remote error content and stderr are never forwarded to host diagnostics', async t => {
  const { instance, events } = runtime(t), { sessionId } = await instance.newSession();
  await assert.rejects(instance.prompt({ sessionId, prompt: 'remote-error' }), error => error.code === 'ACP_REMOTE_ERROR' && !JSON.stringify(error).includes('synthetic-private-diagnostic'));
  await until(() => instance.status.stderrBytes > 0);
  assert(!JSON.stringify(events).includes('synthetic-private-diagnostic')); assert(!JSON.stringify(events).includes('synthetic-private-stderr'));
});

test('mockable spawn isolates late old-generation write errors and old buffered frames', async t => {
  const children = [], events = [];
  const instance = new GrokAcpRuntime({ cwd: '/tmp', onEvent(event) { events.push(event); }, spawn() {
    const child = new EventEmitter(); child.stdout = new EventEmitter(); child.stderr = new EventEmitter(); child.stdin = new EventEmitter(); child.callbacks = [];
    child.stdin.write = (frame, callback) => { child.callbacks.push(callback); const request = JSON.parse(frame); queueMicrotask(() => {
      child.stdout.emit('data', Buffer.from(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: request.method === 'initialize' ? { protocolVersion: 1, authMethods: [{ id: 'cached_token' }], agentCapabilities: {} } : {} }) + '\n'));
    }); };
    child.kill = () => { queueMicrotask(() => child.emit('exit', 0)); return true; }; children.push(child); return child;
  } });
  t.after(() => instance.close()); await instance.start(); const old = children[0]; old.emit('exit', 9); await instance.start();
  old.callbacks[0](Error('synthetic old pipe failure')); assert.equal(instance.status.state, 'ready');
  old.stdout.emit('data', Buffer.from('not-json\n' + JSON.stringify({ jsonrpc: '2.0', method: 'session/update', params: { sessionId: 'old', update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'stale' } } } }) + '\n'));
  assert.equal(instance.status.state, 'ready'); assert(!events.some(event => event.text === 'stale'));
});

test('an authenticate or new-session response immediately followed by process death never becomes a ready connection/session', async t => {
  for (const method of ['authenticate', 'session/new']) {
    const mock = memoryAgent({ afterResponse(request, child) { if (request.method === method) child.emit('exit', 9); } });
    const instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn }); t.after(() => instance.close());
    await assert.rejects(method === 'authenticate' ? instance.start() : instance.newSession(), { code: 'ACP_CONNECTION_CHANGED' });
    assert.equal(instance.status.state, 'failed'); assert.equal(instance.status.pid, null);
  }
});

test('synchronous restart during bad-frame handling cannot deliver later old frames or kill the new connection', async t => {
  const mock = memoryAgent(); let restart;
  const events = [], instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn, onEvent(event) { events.push(event); if (event.type === 'runtime_error') restart = instance.start(); } });
  t.after(() => instance.close()); await instance.start();
  mock.children[0].stdout.emit('data', Buffer.from('bad\n' + JSON.stringify({ jsonrpc: '2.0', method: 'session/update', params: { sessionId: 'old', update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'old-generation-content' } } } }) + '\n'));
  await restart; assert.equal(instance.status.state, 'ready'); assert(!events.some(event => event.text === 'old-generation-content'));
});

test('old prompt continuation cannot mark a restarted process failed', async t => {
  const mock = memoryAgent({ onRequest(request, child) {
    if (request.method === 'session/prompt') { child.stdout.emit('data', Buffer.from(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { stopReason: 'not-valid' } }) + '\nbad\n')); return false; }
  } }); let restart;
  const instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn, onEvent(event) { if (event.type === 'runtime_error') restart = instance.start(); } });
  t.after(() => instance.close()); const { sessionId } = await instance.newSession();
  await assert.rejects(instance.prompt({ sessionId, prompt: 'old' }), { code: 'ACP_CONNECTION_CHANGED' }); await restart;
  assert.equal(instance.status.state, 'ready'); assert.equal(mock.children.length, 2);
});

test('concurrent loads of an unknown session cannot change its working-directory scope', async t => {
  const mock = memoryAgent(), instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn }); t.after(() => instance.close());
  const results = await Promise.allSettled([instance.loadSession({ sessionId: 'external', cwd: '/scope/a' }), instance.loadSession({ sessionId: 'external', cwd: '/scope/b' })]);
  assert.equal(results[0].status, 'fulfilled'); assert.equal(results[1].status, 'rejected'); assert.equal(results[1].reason.code, 'ACP_SCOPE_MISMATCH');
  assert.equal(mock.requests.filter(request => request.method === 'session/load').length, 1);
});

test('standard ACP stdio MCP server configuration passes through new/load without token inspection', async t => {
  const mock = memoryAgent(), instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn }); t.after(() => instance.close());
  const mcpServers = [{ name: 'oviraptor-apps', command: process.execPath, args: ['/synthetic/mcp-bridge.mjs'], env: [{ name: 'OVIRAPTOR_SCOPE', value: 'synthetic-session-scope' }] }];
  const { sessionId } = await instance.newSession({ mcpServers }); await instance.loadSession({ sessionId, mcpServers });
  for (const request of mock.requests.filter(request => ['session/new', 'session/load'].includes(request.method))) assert.deepEqual(request.params.mcpServers, mcpServers);
});

test('fatal permission cleanup can synchronously restart without killing or orphaning either process', async t => {
  const mock = memoryAgent({ onRequest(request, child) {
    if (request.method === 'session/prompt') { child.send({ id: 'permission', method: 'session/request_permission', params: { sessionId: request.params.sessionId, toolCall: { toolCallId: 't' }, options: [{ optionId: 'allow', name: 'Allow', kind: 'allow_once' }] } }); return false; }
  } }); let restart;
  const instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn, permissionPolicy: 'ask', onEvent(event) { if (event.type === 'permission_resolved') restart = instance.start(); } });
  t.after(() => instance.close()); const { sessionId } = await instance.newSession();
  const pending = instance.prompt({ sessionId, prompt: 'old' }), rejection = assert.rejects(pending, { code: 'ACP_PROTOCOL_ERROR' });
  await until(() => instance.status.pendingPermissions === 1); mock.children[0].stdout.emit('data', Buffer.from('bad\n'));
  await rejection; await restart; assert.equal(instance.status.state, 'ready');
  assert.deepEqual(mock.children[0].kills, ['SIGKILL']); assert.deepEqual(mock.children[1].kills, []);
});

test('a synchronous cancel write error cannot terminate a new connection created by the error callback', async t => {
  const mock = memoryAgent({ onRequest(request) { if (request.method === 'session/prompt') return false; } }); let restart;
  const instance = new GrokAcpRuntime({ cwd: '/tmp', spawn: mock.spawn, onEvent(event) { if (event.type === 'runtime_error') restart = instance.start(); } });
  t.after(() => instance.close()); const { sessionId } = await instance.newSession();
  const pending = instance.prompt({ sessionId, prompt: 'old' }), rejection = assert.rejects(pending, { code: 'ACP_IO_ERROR' });
  await until(() => instance.status.activeTurns === 1); mock.children[0].stdin.write = () => { throw Error('Synthetic pipe failure'); };
  assert.equal(instance.cancel(sessionId), false); await rejection; await restart; assert.equal(instance.status.state, 'ready'); assert.deepEqual(mock.children[1].kills, []);
});

test('closing during a permission request cancels the UI and rejects the turn without leaking pending work', async t => {
  const { instance, events } = runtime(t, { permissionPolicy: 'ask' }), { sessionId } = await instance.newSession();
  const pending = instance.prompt({ sessionId, prompt: 'permission' }), rejection = assert.rejects(pending, { code: 'ACP_CLOSED' });
  await until(() => instance.status.pendingPermissions === 1); await instance.close(); await rejection;
  assert.equal(instance.status.state, 'closed'); assert.equal(instance.status.pendingPermissions, 0); assert.equal(instance.status.pendingRequests, 0);
  assert.equal(events.find(event => event.type === 'permission_resolved').outcome.outcome, 'cancelled'); await assert.rejects(instance.start(), { code: 'ACP_CLOSED' });
});
