import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
async function fixture(t, { deferStart = false } = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-cancellation-truth-'));
  const store = new HostStore(':memory:');
  const starting = deferred(), prompting = deferred(), response = deferred();
  const calls = { start: 0, sessions: 0, prompts: 0, cancels: 0 };
  // The host still performs its real conversation, message and metric writes.
  // Runtime behavior alone is synthetic; no ACP child, account or network exists.
  const runtime = {
    status: { pid: 71 },
    async start() { calls.start++; if (deferStart) await starting.promise; },
    async newSession() { calls.sessions++; return { sessionId: 'synthetic-truth-session' }; },
    prompt() { calls.prompts++; prompting.resolve(); return response.promise; },
    async cancel() { calls.cancels++; return true; },
    async close() { starting.resolve(); if (calls.prompts) response.reject(Object.assign(new Error('Synthetic close'), { code: 'ACP_CLOSED', details: { outcomeUnknown: true } })); },
  };
  const events = [];
  const host = new AgentHost({ dataDir: directory, store, runtime, onEvent: event => events.push(event) });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = store.createProject('Synthetic cancellation truth');
  const conversation = store.createConversation(project.id);
  await host.send(conversation.id, { text: 'Synthetic local-only message' });
  const run = host.runs.get(conversation.id);
  assert(run);
  const actual = () => ({
    message: store.messages(conversation.id).find(message => message.role === 'assistant'),
    metrics: JSON.parse(store.db.prepare('SELECT metrics FROM runtime_metrics WHERE run_id=?').get(run.id).metrics),
    completed: events.find(event => event.type === 'message.completed' && event.runId === run.id),
  });
  return { host, conversation, run, starting, prompting, response, calls, actual };
}

function assertTruth(actual, { status, outcomeUnknown, stopReason, failureCode, cancelRequested = true }) {
  for (const record of [actual.message, actual.metrics, actual.completed]) {
    assert.equal(record.outcomeUnknown, outcomeUnknown);
    assert.equal(record.cancelRequested, cancelRequested);
    if (stopReason !== undefined) assert.equal(record.stopReason, stopReason);
    if (failureCode !== undefined) assert.equal(record.failureCode, failureCode);
  }
  assert.equal(actual.message.status, status);
  assert.equal(actual.metrics.outcome, status);
  assert.equal(actual.completed.status, status);
}

test('unacknowledged ACP cancellation timeout persists unknown outcome instead of confirmed cancellation', { timeout: 5000 }, async t => {
  const f = await fixture(t);
  await f.prompting.promise;
  await f.host.cancel(f.conversation.id);
  f.response.reject(Object.assign(new Error('Synthetic missing cancellation acknowledgement'), { code: 'ACP_CANCEL_TIMEOUT', details: { outcomeUnknown: true } }));
  await f.run.completion;
  const actual = f.actual();
  assert(['failed', 'incomplete'].includes(actual.message.status));
  assert.notEqual(actual.message.status, 'cancelled');
  assertTruth(actual, { status: actual.message.status, outcomeUnknown: true, failureCode: 'ACP_CANCEL_TIMEOUT' });
});

test('a real end_turn racing a cancellation request remains completed with the request recorded', { timeout: 5000 }, async t => {
  const f = await fixture(t);
  await f.prompting.promise;
  await f.host.cancel(f.conversation.id);
  f.response.resolve({ stopReason: 'end_turn', metrics: { totalMs: 1 } });
  await f.run.completion;
  assertTruth(f.actual(), { status: 'completed', outcomeUnknown: false, stopReason: 'end_turn' });
});

test('an actual cancelled stop reason records acknowledged cancellation with known outcome', { timeout: 5000 }, async t => {
  const f = await fixture(t);
  await f.prompting.promise;
  await f.host.cancel(f.conversation.id);
  f.response.resolve({ stopReason: 'cancelled', metrics: { totalMs: 1 } });
  await f.run.completion;
  assertTruth(f.actual(), { status: 'cancelled', outcomeUnknown: false, stopReason: 'cancelled' });
});

test('a local cancellation before any prompt is sent is known and never opens a session or sends a prompt', { timeout: 5000 }, async t => {
  const f = await fixture(t, { deferStart: true });
  await f.host.cancel(f.conversation.id);
  f.starting.resolve();
  await f.run.completion;
  assert.equal(f.calls.sessions, 0);
  assert.equal(f.calls.prompts, 0);
  assert.equal(f.calls.cancels, 0);
  assertTruth(f.actual(), { status: 'cancelled', outcomeUnknown: false });
});

test('restart migrates legacy running and unclassified interrupted messages to unknown without rewriting terminal or classified outcomes', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-restart-truth-'));
  const file = join(directory, 'host.sqlite');
  let store = new HostStore(file);
  t.after(async () => { store?.close(); await rm(directory, { recursive: true, force: true }); });
  const project = store.createProject('Synthetic legacy restart');
  const conversation = store.createConversation(project.id);
  const running = store.addMessage(conversation.id, 'assistant', 'Partial before crash', 'running', 'synthetic-running');
  const historical = store.addMessage(conversation.id, 'assistant', 'Historical partial', 'interrupted', 'synthetic-historical');
  const completed = store.addMessage(conversation.id, 'assistant', 'Known complete', 'completed', 'synthetic-complete');
  const cancelled = store.addMessage(conversation.id, 'assistant', 'Known cancellation', 'cancelled', 'synthetic-cancel');
  // Recreate the real old schema by removing only the later disposition columns.
  // The constructor must add them before attempting its recovery update.
  for (const name of ['outcome_unknown', 'failure_code', 'stop_reason', 'cancel_requested']) store.db.exec(`ALTER TABLE messages DROP COLUMN ${name}`);
  store.close(); store = new HostStore(file);
  let messages = new Map(store.messages(conversation.id).map(message => [message.id, message]));
  for (const id of [running.id, historical.id]) {
    assert.equal(messages.get(id).status, 'interrupted');
    assert.equal(messages.get(id).outcomeUnknown, true);
    assert.equal(messages.get(id).failureCode, 'host_interrupted_restart');
    assert.equal(messages.get(id).stopReason, '');
  }
  assert.equal(messages.get(completed.id).status, 'completed');
  assert.equal(messages.get(completed.id).outcomeUnknown, false);
  assert.equal(messages.get(cancelled.id).status, 'cancelled');
  assert.equal(messages.get(cancelled.id).outcomeUnknown, false);
  // A stored explicit disposition is authoritative; a later open must preserve it.
  store.finishMessage(historical.id, 'Known local interruption', 'interrupted', { outcomeUnknown: false, failureCode: 'synthetic_known_local_interruption', stopReason: '', cancelRequested: true });
  store.close(); store = new HostStore(file);
  messages = new Map(store.messages(conversation.id).map(message => [message.id, message]));
  assert.equal(messages.get(historical.id).outcomeUnknown, false);
  assert.equal(messages.get(historical.id).failureCode, 'synthetic_known_local_interruption');
  assert.equal(messages.get(historical.id).cancelRequested, true);
  assert.equal(messages.get(running.id).outcomeUnknown, true);
});
