const assert = require('node:assert/strict');
const test = require('node:test');
const { mountTrace, trace, flush, deferred } = require('./agent_trace_harness.cjs');
const event = (scanId = 'scan-a', sequence = 1) => ({ scanId, sequence, attemptNumber: 1 });
const reads = h => h.calls.filter(([name]) => name === 'getAgentTrace').length;

test('trace events refresh the selected task without waiting for the reconciliation interval', async t => {
  let version = 0;
  const h = await mountTrace({ getAgentTrace: id => trace(id, `version-${++version}`) });
  t.after(h.unmount);
  await h.drain(); const before = reads(h);
  h.emit(event()); await h.drain();
  assert.equal(reads(h), before + 1);
  assert.equal(h.b.detail.value.summary.model, `version-${version}`);
});

test('unrelated or malformed event hints cannot trigger trace reads or publish their text', async t => {
  const h = await mountTrace(); t.after(h.unmount); await h.drain();
  const before = reads(h);
  for (const payload of [null, {}, event('scan-b'), event(''), event('scan-a', 0),
    event('scan-a', 1.5), { ...event(), attemptNumber: -1 },
    { ...event(), sequence: 'private-secret' }]) h.emit(payload);
  await h.drain();
  assert.equal(reads(h), before);
  assert.deepEqual(h.notices, []);
});

test('event bursts coalesce and hints during a pending read cause one catch-up read', async t => {
  const pending = deferred(); let delay = false;
  const h = await mountTrace({ getAgentTrace: id => delay ? pending.promise : trace(id) });
  t.after(h.unmount); await h.drain(); const before = reads(h); delay = true;
  for (let i = 1; i <= 100; i++) h.emit(event('scan-a', i));
  assert.equal(h.scheduled.size, 1); await h.drain();
  for (let i = 101; i <= 200; i++) h.emit(event('scan-a', i));
  await h.drain(); assert.equal(reads(h), before + 1);
  delay = false; pending.resolve(trace('scan-a')); await flush(); await h.drain();
  assert.equal(reads(h), before + 2);
});

test('terminal trace receives events and reconciles missed final notifications', async t => {
  const h = await mountTrace({ getAgentTrace: id => trace(id, '', 'completed') });
  t.after(h.unmount); await h.drain(); const before = reads(h);
  await h.tick(); assert.equal(reads(h), before + 1);
  h.emit(event()); await h.drain(); assert.equal(reads(h), before + 2);
});

test('missed notifications cannot leave terminal or failed initial detail snapshots permanently stale', async t => {
  for (const status of ['completed', 'failed', 'cancelled', 'paused', 'missing-detail']) {
    let version = 1;
    const h = await mountTrace({ getAgentTrace: id => {
      if (status === 'missing-detail' && version === 1) throw Error('private-transport-error');
      return trace(id, `revision-${version}`, status === 'missing-detail' ? 'completed' : status);
    } });
    t.after(h.unmount);
    await h.drain();
    version = 2;
    // Deliberately drop the event; the bounded fallback must read stored state.
    await h.tick();
    assert.equal(h.b.detail.value?.summary.model, 'revision-2', status);
    assert.doesNotMatch(await h.render(), /private-transport-error/);
    h.unmount();
  }
});

test('hidden event work waits until visibility resumes and then catches up immediately', async t => {
  const h = await mountTrace(); t.after(h.unmount); await h.drain(); const before = reads(h);
  await h.visibility(true); h.emit(event()); await h.drain(); await h.tick();
  assert.equal(reads(h), before);
  await h.visibility(false); await h.drain(); assert.equal(reads(h), before + 1);
});

test('pending listener registration does not block initial trace reads and is released after unmount', async t => {
  const pending = deferred(); let released = 0;
  const h = await mountTrace({}, { listen: () => pending.promise }); t.after(h.unmount);
  assert.equal(reads(h), 1); h.unmount();
  pending.resolve(() => { released++; }); await flush();
  assert.equal(released, 1); assert.equal(h.timers.size, 0);
  assert.equal(h.scheduled.size, 0); assert.equal(h.visibilityListeners.size, 0);
});

test('listener failure retains reconciliation with a fixed privacy-safe warning', async t => {
  const h = await mountTrace({}, { listen: async () => { throw new Error('/private/token=secret'); } });
  t.after(h.unmount);
  assert.deepEqual(h.notices, [['info', '轨迹实时通知连接失败，已使用定期补查']]);
  const before = reads(h); await h.tick(); assert.equal(reads(h), before + 1);
});

test('failed trace subscription recovers and catches up terminal rows without waiting for another event', async t => {
  let attempts = 0, callback, released = 0, version = 0;
  const h = await mountTrace({ getAgentTrace: id => trace(id, `v${++version}`, 'completed') }, {
    listen: async (_channel, handler) => {
      if (++attempts === 1) throw new Error('private retry error');
      callback = handler;
      return () => { released++; };
    },
  });
  t.after(h.unmount);
  const before = reads(h);
  await h.tick(); await h.drain();
  assert.equal(attempts, 2);
  // One fallback read plus a catch-up after registration closes the subscribe gap.
  assert.equal(reads(h), before + 2);
  callback({ payload: event() }); await h.drain();
  assert.equal(reads(h), before + 3);
  await h.tick(); assert.equal(attempts, 2);
  h.unmount(); assert.equal(released, 1);
});

test('trace subscription retries stay single-flight and a late retry is released after unmount', async t => {
  const pending = deferred(); let attempts = 0, released = 0;
  const h = await mountTrace({}, { listen: () => {
    if (++attempts === 1) return Promise.reject(new Error('first failure'));
    return pending.promise;
  } });
  t.after(h.unmount);
  await h.tick(); await h.tick();
  await h.visibility(true); await h.visibility(false); await h.tick();
  assert.equal(attempts, 2);
  h.unmount(); pending.resolve(() => { released++; }); await flush();
  assert.equal(released, 1);
  assert.equal(h.timers.size, 0); assert.equal(h.scheduled.size, 0);
  assert.equal(h.visibilityListeners.size, 0);
});

test('trace retry pauses while hidden and repeated failures do not spam warnings', async t => {
  let attempts = 0;
  const h = await mountTrace({}, { listen: async () => {
    attempts++; throw new Error('private failure');
  } });
  t.after(h.unmount);
  await h.visibility(true); await h.tick(); assert.equal(attempts, 1);
  await h.visibility(false); assert.equal(attempts, 2);
  await h.tick(); assert.equal(attempts, 3);
  assert.deepEqual(h.notices, [['info', '轨迹实时通知连接失败，已使用定期补查']]);
});

test('unmount cancels coalesced work and releases the event listener', async t => {
  const h = await mountTrace(); t.after(h.unmount); await h.drain();
  h.emit(event()); const callbacks = [...h.scheduled.values()], before = reads(h);
  h.unmount(); for (const callback of callbacks) callback(); await h.drain();
  assert.equal(reads(h), before); assert.equal(h.listeners.size, 0);
  assert.equal(h.visibilityListeners.size, 0); assert.equal(h.scheduled.size, 0);
});

test('late registration catches up rows committed after the first snapshot', async t => {
  const registration = deferred();
  const h = await mountTrace({}, { listen: () => registration.promise }); t.after(h.unmount);
  assert.equal(reads(h), 1);
  registration.resolve(() => {}); await flush(); await h.drain();
  assert.equal(reads(h), 2);
});

test('a queued hint for the previous selection cannot start a read for the new task', async t => {
  const h = await mountTrace(); t.after(h.unmount); await h.drain();
  h.emit(event()); await h.b.selectTrace('scan-b'); const before = reads(h);
  await h.drain(); assert.equal(reads(h), before);
  assert.equal(h.b.detail.value.summary.scanId, 'scan-b');
});

test('events during a manual snapshot cause a catch-up even when that snapshot is terminal', async t => {
  const pending = deferred(); let delay = false;
  const h = await mountTrace({ getAgentTrace: id => delay ? pending.promise : trace(id, 'fresh', 'completed') });
  t.after(h.unmount); await h.drain(); delay = true;
  const selecting = h.b.selectTrace('scan-a'); const before = reads(h);
  h.emit(event()); await h.drain(); assert.equal(reads(h), before);
  delay = false; pending.resolve(trace('scan-a', 'old', 'completed')); await selecting; await flush();
  assert.equal(reads(h), before + 1); assert.equal(h.b.detail.value.summary.model, 'fresh');
});

test('a committed hint can recover a failed initial detail read without rendering raw errors', async t => {
  let failing = true;
  const h = await mountTrace({ getAgentTrace: id => {
    if (failing) throw new Error('private-token');
    return trace(id);
  } });
  t.after(h.unmount); assert.equal(h.b.detail.value, undefined);
  failing = false; h.emit(event()); await h.drain();
  assert.equal(h.b.detail.value.summary.scanId, 'scan-a');
  assert.deepEqual(h.notices, [['error', '运行轨迹读取失败，请重试']]);
});

test('a second hint during post-selection catch-up is retained for a completed task', async t => {
  const selection = deferred(), catchUp = deferred(); let count = 0;
  const h = await mountTrace({ getAgentTrace: id => {
    count++;
    if (count === 2) return selection.promise;
    if (count === 3) return catchUp.promise;
    return trace(id, 'newest', 'completed');
  } });
  t.after(h.unmount); await h.drain();
  const work = h.b.selectTrace('scan-a');
  h.emit(event()); await h.drain();
  selection.resolve(trace('scan-a', 'old', 'completed')); await work; await flush();
  assert.equal(count, 3);
  h.emit(event('scan-a', 2)); await h.drain();
  catchUp.resolve(trace('scan-a', 'intermediate', 'completed')); await flush();
  assert.equal(count, 4); assert.equal(h.b.detail.value.summary.model, 'newest');
});
