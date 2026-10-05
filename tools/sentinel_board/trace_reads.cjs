// Exercise the real Board reader; share the trace fixture with its other UI consumer.
const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan } = require('./harness.cjs');
const { trace, deferred } = require('../agent_trace_harness.cjs');

async function fixture(t, read) {
  const m = await mount(undefined, { getAgentTrace: read });
  t.after(m.unmount);
  m.b.selected.value = scan('A');
  return m;
}

test('Board rejects cross-task trace responses and never publishes their contents', async t => {
  const m = await fixture(t, async () => trace('B', 'other-task-private-data'));
  await m.b.loadSelectedTrace('A', true);
  assert.equal(m.b.liveTrace.value, undefined);
  assert.equal(m.b.liveTraceBusy.value, false);
  assert.deepEqual(m.events, [['error', '运行轨迹读取失败，请重试']]);
});

test('Board trace errors use a fixed message without exposing backend paths or secrets', async t => {
  const m = await fixture(t, async () => { throw new Error('/private/db?token=secret'); });
  await m.b.loadSelectedTrace('A', true);
  assert.deepEqual(m.events, [['error', '运行轨迹读取失败，请重试']]);
  await m.b.loadSelectedTrace('A');
  assert.equal(m.events.length, 1, 'background failure must stay silent');
});

test('Board validates display fields before replacing a trace and permits a valid retry', async t => {
  let next = trace('A');
  const m = await fixture(t, async () => next);
  const mutations = [value => { value.summary.tools = null; }, value => { value.events = {}; },
    value => { value.summary.agentCount = -1; }, value => { value.summary.totalTokens = NaN; },
    value => { value.summary.llmRequests = Number.MAX_SAFE_INTEGER + 1; },
    value => { value.summary.sourceAuthority = 'unverified'; },
    value => { value.events = [{ id: 'broken', targetUrl: 42 }]; },
    value => { value.summary.tools = [{ name: 'tool', calls: '1', results: 0 }]; },
    value => { value.promptAudit = { instructionSha256: null }; }];
  for (const mutate of mutations) {
    next = trace('A'); await m.b.loadSelectedTrace('A');
    assert.equal(m.b.liveTrace.value.summary.scanId, 'A');
    next = trace('A'); mutate(next); await m.b.loadSelectedTrace('A');
    assert.equal(m.b.liveTrace.value, undefined);
    assert.equal(m.b.liveTraceBusy.value, false);
  }
  next = trace('A'); next.summary.sourceAuthority = 'historical_external'; next.extra = 'forward-compatible';
  await m.b.loadSelectedTrace('A');
  assert.equal(m.b.liveTrace.value.summary.sourceAuthority, 'historical_external');
});

test('Board preserves valid historical events and nullable prompt metadata without rewriting content', async t => {
  const next = trace('A');
  next.summary.sourceAuthority = 'historical_external';
  next.summary.tools = [{ name: 'fixture-tool', calls: 1, results: 1 }];
  next.events = [{ id: 'event-1', sessionId: 'session', callId: 'call', targetUrl: '',
    eventType: 'function_call_output', role: 'tool', name: 'fixture-tool', status: 'recorded',
    detail: 'sanitized fixture text', detailSize: 22, detailTruncated: false, createdAt: '' }];
  next.promptAudit = { captureMode: 'metadata', source: 'fixture', captureLevel: 'metadata',
    exactModelRequest: false, model: '', deployment: 'local', fullPower: false, recordedAt: '',
    instructionSha256: '', instructionChars: 0, instruction: null, notice: 'fixture' };
  const m = await fixture(t, async () => next);
  await m.b.loadSelectedTrace('A');
  assert.deepEqual(m.b.liveTrace.value, next);
  next.promptAudit = null;
  await m.b.loadSelectedTrace('A');
  assert.deepEqual(m.b.liveTrace.value, next);
});

test('Board rejects missing required metrics, inherited identity and malformed event fields', async t => {
  let next;
  const m = await fixture(t, async () => next);
  for (const key of ['scanId', 'tools', 'instructionHash', 'exactRequestCapture', 'tokenUsageEstimated',
    ...Object.keys(trace('A').summary).filter(key => typeof trace('A').summary[key] === 'number')]) {
    next = trace('A'); delete next.summary[key];
    await m.b.loadSelectedTrace('A');
    assert.equal(m.b.liveTrace.value, undefined, key);
  }
  next = trace('A'); delete next.summary.scanId; Object.setPrototypeOf(next.summary, { scanId: 'A' });
  await m.b.loadSelectedTrace('A');
  assert.equal(m.b.liveTrace.value, undefined);
  for (const bad of [null, [], {}, { summary: trace('A').summary, events: [null] }]) {
    next = bad; await m.b.loadSelectedTrace('A');
    assert.equal(m.b.liveTrace.value, undefined);
  }
});

test('Board trace project changes fence A to B to A responses and clear prior display', async t => {
  const old = deferred(), current = deferred(); let reads = 0;
  const m = await fixture(t, async () => ++reads === 1 ? old.promise : current.promise);
  const first = m.b.loadSelectedTrace('A', true);
  m.b.projectFilter.value = 2;
  m.b.projectFilter.value = undefined;
  const second = m.b.loadSelectedTrace('A', true);
  old.reject(new Error('old-project-secret')); await first;
  assert.equal(m.b.liveTraceBusy.value, true);
  assert.deepEqual(m.events, []);
  current.resolve(trace('A', 'current-project')); await second;
  m.b.projectFilter.value = 3;
  assert.equal(m.b.liveTrace.value, undefined);
  assert.equal(m.b.liveTraceBusy.value, false);
});

test('Board ignores pending trace completion and refuses new reads after unmount', async t => {
  const pending = deferred(); let reads = 0;
  const m = await fixture(t, async () => { reads++; return pending.promise; });
  const work = m.b.loadSelectedTrace('A', true);
  m.unmount();
  pending.resolve(trace('A')); await work;
  await m.b.loadSelectedTrace('A', true);
  assert.equal(reads, 1);
  assert.equal(m.b.liveTrace.value, undefined);
  assert.equal(m.b.liveTraceBusy.value, false);
  assert.deepEqual(m.events, []);
});
