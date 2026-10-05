const assert = require('node:assert/strict');
const test = require('node:test');
const { mountTrace, trace, flush, deferred } = require('./agent_trace_harness.cjs');
require('./agent_trace_events.cjs');

test('unmount during initial catalog load cannot start detail reads or leak a polling interval', async (t) => {
  const pending = deferred();
  const h = await mountTrace({ listAgentTraces: () => pending.promise });
  t.after(h.unmount);
  h.unmount();
  pending.resolve([trace('scan-a').summary]);
  await flush();
  assert.equal(h.timers.size, 0);
  assert.equal(h.calls.filter(([method]) => method === 'getAgentTrace').length, 0);
  assert.equal(h.b.traces.value.length, 0);
});

test('selecting another task immediately hides old details and failure never restores them', async (t) => {
  const pending = deferred();
  const h = await mountTrace({ getAgentTrace: (id) => id === 'scan-a' ? trace(id) : pending.promise });
  t.after(h.unmount);
  assert.equal(h.b.detail.value.summary.scanId, 'scan-a');
  const selection = h.b.selectTrace('scan-b');
  assert.equal(h.b.detail.value, undefined);
  assert.match(await h.render(), /正在读取任务轨迹/);
  pending.reject(new Error('detail unavailable'));
  await selection;
  assert.equal(h.b.detail.value, undefined);
  assert.deepEqual(h.notices, [['error', '运行轨迹读取失败，请重试']]);
});

test('A to B to A selections fence the first A response even when the task id matches again', async (t) => {
  const first = deferred(), second = deferred();
  let reads = 0;
  const h = await mountTrace({ getAgentTrace: (id) => {
    if (id !== 'scan-a' || ++reads === 1) return trace(id);
    return reads === 2 ? first.promise : second.promise;
  } });
  t.after(h.unmount);
  const oldA = h.b.selectTrace('scan-a');
  await h.b.selectTrace('scan-b');
  const newA = h.b.selectTrace('scan-a');
  second.resolve(trace('scan-a', 'new-revision'));
  await newA;
  first.resolve(trace('scan-a', 'old-revision'));
  await oldA;
  assert.equal(h.b.detail.value.summary.model, 'new-revision');
  assert.match(await h.render(), /new-revision/);
  assert.doesNotMatch(await h.render(), /old-revision/);
});

test('stale detail errors cannot clear the new request loading state or emit notifications', async (t) => {
  const oldRead = deferred(), newRead = deferred();
  let initial = true;
  const h = await mountTrace({ getAgentTrace: (id) => {
    if (initial) { initial = false; return trace(id); }
    return id === 'scan-b' ? oldRead.promise : newRead.promise;
  } });
  t.after(h.unmount);
  const oldSelection = h.b.selectTrace('scan-b');
  h.b.busy.value = 'other-operation';
  const newSelection = h.b.selectTrace('scan-a');
  oldRead.reject(new Error('obsolete failure'));
  await oldSelection;
  assert.equal(h.b.detailLoading.value, true);
  assert.deepEqual(h.notices, []);
  newRead.resolve(trace('scan-a'));
  await newSelection;
  assert.equal(h.b.detailLoading.value, false);
  assert.equal(h.b.busy.value, 'other-operation');
});

test('polling cannot overwrite a newer manual selection, including an A to B to A round trip', async (t) => {
  const pending = deferred();
  let reads = 0;
  const h = await mountTrace({ getAgentTrace: (id) => {
    if (id === 'scan-a' && ++reads === 2) return pending.promise;
    return trace(id, 'current');
  } });
  t.after(h.unmount);
  await h.tick();
  await h.b.selectTrace('scan-b');
  await h.b.selectTrace('scan-a');
  pending.resolve(trace('scan-a', 'obsolete-poll'));
  await flush();
  assert.equal(h.b.detail.value.summary.model, 'current');
  assert.equal(h.b.traces.value[0].model, 'current');
});

test('reconciliation is single-flight, pauses while hidden, covers completed tasks and stops on unmount', async (t) => {
  const pending = deferred();
  let reads = 0;
  const h = await mountTrace({ getAgentTrace: (id) => ++reads === 1 ? trace(id) : pending.promise });
  t.after(h.unmount);
  h.document.hidden = true;
  await h.tick();
  assert.equal(reads, 1);
  h.document.hidden = false;
  await h.tick();
  await h.tick();
  assert.equal(reads, 2);
  pending.resolve(trace('scan-a', '', 'completed'));
  await flush();
  await h.tick();
  assert.equal(reads, 3);
  h.unmount();
  assert.equal(h.timers.size, 0);
  await h.tick();
  assert.equal(reads, 3);
});

test('a detail response for another task is rejected rather than displayed or merged into the list', async (t) => {
  const h = await mountTrace({ getAgentTrace: () => trace('wrong-task') });
  t.after(h.unmount);
  assert.equal(h.b.detail.value, undefined);
  assert.equal(h.b.traces.value[0].scanId, 'scan-a');
  assert.equal(h.notices.length, 1);
  assert.doesNotMatch(await h.render(), /wrong-task/);
});

test('trace hub refuses malformed detail fields without merging or leaking errors', async t => {
  let next = trace('scan-a');
  const h = await mountTrace({ getAgentTrace: () => next });
  t.after(h.unmount);
  const previous = h.b.traces.value[0];
  next = trace('scan-a'); next.summary.tools = null;
  await h.b.selectTrace('scan-a');
  assert.equal(h.b.detail.value, undefined);
  assert.equal(h.b.traces.value[0], previous);
  assert.deepEqual(h.notices, [['error', '运行轨迹读取失败，请重试']]);
  next = trace('scan-a'); next.events = null;
  await h.b.selectTrace('scan-a');
  assert.equal(h.b.detail.value, undefined);
});

test('unmount while a detail read is pending ignores its result and leaves no timer', async (t) => {
  const pending = deferred();
  const h = await mountTrace({ getAgentTrace: () => pending.promise });
  t.after(h.unmount);
  h.unmount();
  pending.resolve(trace('scan-a'));
  await flush();
  assert.equal(h.b.detail.value, undefined);
  assert.equal(h.timers.size, 0);
  assert.deepEqual(h.notices, []);
});

test('catalog reloads publish only the newest complete snapshot', async (t) => {
  const oldRead = deferred(), newRead = deferred();
  let reads = 0;
  const h = await mountTrace({ listAgentTraces: () => {
    if (++reads === 1) return [trace('scan-a').summary];
    return reads === 2 ? oldRead.promise : newRead.promise;
  } });
  t.after(h.unmount);
  const oldLoad = h.b.load(), newLoad = h.b.load();
  newRead.resolve([trace('scan-a', 'latest-catalog').summary]);
  await newLoad;
  oldRead.resolve([trace('scan-b', 'obsolete-catalog').summary]);
  await oldLoad;
  assert.equal(h.b.traces.value[0].model, 'latest-catalog');
  assert.equal(h.b.selectedId.value, 'scan-a');
  assert.equal(h.b.loading.value, false);
});

test('a failed stale catalog read cannot release a newer load or show an obsolete error', async (t) => {
  const oldRead = deferred(), newRead = deferred();
  let reads = 0;
  const h = await mountTrace({ listAgentTraces: () => {
    if (++reads === 1) return [trace('scan-a').summary];
    return reads === 2 ? oldRead.promise : newRead.promise;
  } });
  t.after(h.unmount);
  const oldLoad = h.b.load(), newLoad = h.b.load();
  oldRead.reject(new Error('obsolete catalog failure'));
  await oldLoad;
  assert.equal(h.b.loading.value, true);
  assert.deepEqual(h.notices, []);
  newRead.resolve([trace('scan-a').summary]);
  await newLoad;
  assert.equal(h.b.loading.value, false);
});

test('an off-task polling response retains the last verified detail and list summary', async (t) => {
  let reads = 0;
  const h = await mountTrace({ getAgentTrace: (id) =>
    ++reads === 1 ? trace(id, 'verified') : trace('wrong-task', 'unrelated'),
  });
  t.after(h.unmount);
  await h.tick();
  assert.equal(h.b.detail.value.summary.model, 'verified');
  assert.equal(h.b.traces.value[0].model, 'verified');
  assert.deepEqual(h.notices, []);
});

test('a polling response arriving after unmount cannot change any visible task data', async (t) => {
  const pending = deferred();
  let reads = 0;
  const h = await mountTrace({ getAgentTrace: (id) =>
    ++reads === 1 ? trace(id, 'before-unmount') : pending.promise,
  });
  t.after(h.unmount);
  await h.tick();
  h.unmount();
  pending.resolve(trace('scan-a', 'after-unmount'));
  await flush();
  assert.equal(h.b.detail.value.summary.model, 'before-unmount');
  assert.equal(h.b.traces.value[0].model, 'before-unmount');
  assert.equal(h.timers.size, 0);
});
