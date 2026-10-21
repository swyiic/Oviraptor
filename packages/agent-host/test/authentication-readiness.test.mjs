import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';
import { AgentHost } from '../src/service.mjs';

const peer = fileURLToPath(new URL('../../runtime-acp/tests/fixtures/authentication.mjs', import.meta.url));
async function fixture(t, mode) {
  const root = await mkdtemp(join(tmpdir(), 'host-authentication-readiness-')), journal = join(root, 'requests.ndjson');
  const events = []; let host;
  const runtime = new GrokAcpRuntime({ cwd: root, requestTimeoutMs: 1000, env: { PATH: '/usr/bin:/bin', LANG: 'C.UTF-8' },
    spawn(_command, _args, options) { return spawn(process.execPath, [peer, mode, journal], options); },
    onEvent: event => host.runtimeEvent(event) });
  host = new AgentHost({ dataDir: root, runtime, onEvent: event => events.push(event) });
  t.after(async () => { await host.close(); await rm(root, { recursive: true, force: true }); });
  const project = host.store.createProject('Synthetic authentication classification');
  const conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: 'Retain this local message, without login or model calls' });
  const run = host.runs.get(conversation.id); await run.completion;
  return { host, events, conversation, run,
    requests: (await readFile(journal, 'utf8')).trim().split('\n').map(line => JSON.parse(line)),
    metrics: JSON.parse(host.store.db.prepare('SELECT metrics FROM runtime_metrics WHERE run_id=?').get(run.id).metrics),
    messages: host.store.messages(conversation.id) };
}

test('real synthetic subprocess missing login is propagated through status events and durable metrics without model work', async t => {
  const f = await fixture(t, 'grok-com');
  assert.deepEqual(f.requests.map(request => request.method), ['initialize']);
  for (const value of [f.host.status().runtime, f.host.settings().runtime]) {
    assert.equal(value.state, 'authentication_required'); assert.equal(value.authenticationRequired, true);
    assert.match(value.message, /Sign in separately/); assert.match(value.message, /No login or message resend was started/);
  }
  assert.equal(f.host.store.conversation(f.conversation.id).sessionId, null);
  assert.equal(f.host.mcpScopes.size, 0); assert.equal(f.host.loadedSessions.size, 0);
  assert.equal(f.messages[0].role, 'user'); assert.match(f.messages[0].content, /Retain this local message/);
  assert.equal(f.messages[1].status, 'failed'); assert.equal(f.messages[1].content, '');
  assert.equal(f.messages[1].failureCode, 'ACP_CACHED_AUTH_UNAVAILABLE'); assert.equal(f.messages[1].outcomeUnknown, false);
  assert.equal(f.metrics.outcome, 'failed'); assert.equal(f.metrics.authenticationRequired, true);
  assert.equal(f.metrics.outcomeUnknown, false); assert.equal(f.metrics.runtimeEvidence, 'synthetic-or-custom');
  assert.equal(f.metrics.firstTextMs, undefined); assert.equal(f.metrics.firstToolMs, undefined);
  for (const type of ['runtime.status', 'runtime.error', 'message.completed']) {
    const event = f.events.find(event => event.type === type && event.authenticationRequired === true);
    assert(event, `missing ${type} authentication classification`);
  }
  assert(!f.events.some(event => event.type === 'runtime.status' && event.state === 'ready'));
});

test('failure after a prompt is sent retains unknown outcome and never becomes missing-login classification', async t => {
  const f = await fixture(t, 'prompt-rejected');
  assert.deepEqual(f.requests.map(request => request.method), ['initialize', 'authenticate', 'session/new', 'session/prompt']);
  assert.equal(f.messages[1].failureCode, 'ACP_REMOTE_ERROR'); assert.equal(f.messages[1].outcomeUnknown, true);
  assert.equal(f.metrics.outcomeUnknown, true); assert.equal(f.metrics.authenticationRequired, undefined);
  assert.notEqual(f.host.status().runtime.state, 'authentication_required');
  assert(!JSON.stringify(f.events).includes('Synthetic private prompt diagnostic'));
});

test('an unrelated runtime error cannot declare an authentication-required state with copied flags', async t => {
  const root = await mkdtemp(join(tmpdir(), 'host-auth-classification-negative-'));
  const host = new AgentHost({ dataDir: root }); t.after(async () => { await host.close(); await rm(root, { recursive: true, force: true }); });
  host.runtimeEvent({ type: 'runtime_error', code: 'ACP_PROTOCOL_ERROR', authenticationRequired: true, outcomeUnknown: false });
  assert.equal(host.status().runtime.state, 'failed'); assert.equal(host.status().runtime.authenticationRequired, undefined);
});
