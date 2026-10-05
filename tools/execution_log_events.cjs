const assert = require('node:assert/strict');
const test = require('node:test');

module.exports = ({ mountDetails, flush, deferred }) => {
  async function open(overrides = {}, options = {}) {
    const calls = [];
    const h = await mountDetails({ readSentinelRunnerLog: async (scanId, attempt) => {
      calls.push([scanId, attempt]);
      return { scanId, attempt, status: 'ready', lines: [`snapshot-${calls.length}`] };
    }, ...overrides }, options);
    h.state.detailTab.value = 'execution'; await flush();
    h.live.fire(); await flush();
    return { ...h, calls };
  }
  const hint = (scanId = 'A', attempt = 1) => ({ scanId, attempt, line: 'untrusted-event-content' });

  test('task log events reconcile only selected scope and coalesce bursts without displaying event content', async t => {
    const h = await open(); t.after(h.unmount); const before = h.calls.length;
    for (const invalid of [null, {}, hint('B'), hint('A', 2), hint('A', 0), hint('A', '1')]) h.live.emit(invalid);
    assert.equal(h.live.timers, 0);
    for (let i = 0; i < 25; i++) h.live.emit(hint());
    assert.equal(h.live.timers, 1);
    h.live.fire(); await flush();
    assert.equal(h.calls.length, before + 1);
    assert.deepEqual(h.state.runnerLog.value.lines, [`snapshot-${before + 1}`]);
  });
  test('event arriving during a manual log read is replayed after it finishes', async t => {
    const pending = deferred(); let reads = 0;
    const h = await open({ readSentinelRunnerLog: async (scanId, attempt) => {
      reads++; if (reads === 3) return pending.promise;
      return { scanId, attempt, lines: [`read-${reads}`] };
    } }); t.after(h.unmount);
    const manual = h.state.loadAttemptLog('A', 1);
    h.live.emit(hint()); h.live.fire(); await flush();
    assert.equal(reads, 3);
    pending.resolve({ scanId: 'A', attempt: 1, lines: ['manual'] }); await manual; await flush();
    assert.equal(h.live.timers, 1); h.live.fire(); await flush();
    assert.equal(reads, 4); assert.deepEqual(h.state.runnerLog.value.lines, ['read-4']);
  });
  test('event during live refresh queues one follow-up and retains the previous snapshot', async t => {
    const pending = deferred(); let reads = 0;
    const h = await open({ readSentinelRunnerLog: async (scanId, attempt) => {
      reads++; if (reads === 3) return pending.promise;
      return { scanId, attempt, lines: [`read-${reads}`] };
    } }); t.after(h.unmount);
    h.live.emit(hint()); h.live.fire(); await flush();
    assert.deepEqual(h.state.runnerLog.value.lines, ['read-2']);
    h.live.emit(hint()); h.live.emit(hint());
    pending.resolve({ scanId: 'A', attempt: 1, lines: ['read-3'] }); await flush();
    h.live.fire(); await flush(); assert.equal(reads, 4);
  });
  test('task switch cannot publish the previous task log or use its queued hint', async t => {
    const pending = deferred(); let reads = 0;
    const h = await open({ readSentinelRunnerLog: async (scanId, attempt) => {
      reads++; if (reads === 3) return pending.promise;
      return { scanId, attempt, lines: [scanId] };
    } }); t.after(h.unmount);
    h.live.emit(hint()); h.live.fire(); await flush();
    h.props.preview = { id: 'B', updatedAt: '1' }; await flush();
    pending.resolve({ scanId: 'A', attempt: 1, lines: ['old-task'] }); await flush();
    assert.equal(h.state.runnerLog.value, undefined);
    h.state.detailTab.value = 'execution'; await flush(); h.live.fire(); await flush();
    assert.equal(h.state.runnerLog.value.scanId, 'B');
    const before = reads; h.live.emit(hint()); h.live.fire(); await flush(); assert.equal(reads, before);
  });
  test('attempt switch fences queued events and refreshes only the chosen attempt', async t => {
    const h = await open({ listSentinelScanAttempts: async scanId => [1, 2].map(attemptNumber => ({ scanId, attemptNumber })) });
    t.after(h.unmount); h.live.emit(hint());
    h.state.chooseAttempt(2); await flush(); h.live.fire(); await flush();
    assert.equal(h.state.runnerLog.value.attempt, 2);
    const before = h.calls.length; h.live.emit(hint()); h.live.fire(); await flush();
    assert.equal(h.calls.length, before);
    h.live.emit(hint('A', 2)); h.live.fire(); await flush(); assert.equal(h.calls.length, before + 1);
  });
  test('hidden document and inactive detail tab defer reads and catch up when visible', async t => {
    const h = await open(); t.after(h.unmount); const before = h.calls.length;
    h.live.visibility(true); h.live.emit(hint()); h.live.reconcile(); h.live.fire(); await flush();
    assert.equal(h.calls.length, before);
    h.live.visibility(false); h.live.fire(); await flush(); assert.equal(h.calls.length, before + 1);
    h.state.detailTab.value = 'overview'; await flush(); h.live.emit(hint()); h.live.reconcile(); h.live.fire();
    await flush(); assert.equal(h.calls.length, before + 1);
    h.state.detailTab.value = 'execution'; await flush(); h.live.fire(); await flush();
    assert.equal(h.calls.length, before + 2);
  });
  test('late event registration catches up after an earlier snapshot', async t => {
    const registration = deferred(); const h = await open({}, { registration: registration.promise }); t.after(h.unmount);
    const before = h.calls.length; registration.resolve(); await flush(); h.live.fire(); await flush();
    assert.equal(h.calls.length, before + 1);
  });
  test('subscription failure is sanitized and periodic reconciliation still works', async t => {
    const h = await open({}, { failure: true }); t.after(h.unmount);
    assert.equal(h.state.runnerLogConnectionError.value, '实时日志连接不可用，暂以定时校对恢复');
    const before = h.calls.length; h.live.reconcile(); h.live.fire(); await flush();
    assert.equal(h.calls.length, before + 1);
  });
  test('live read failure retains snapshot and a later event clears the read error', async t => {
    let reads = 0; const h = await open({ readSentinelRunnerLog: async (scanId, attempt) => {
      if (++reads === 3) throw Error('private-reader-error');
      return { scanId, attempt, lines: [`read-${reads}`] };
    } }); t.after(h.unmount);
    h.live.emit(hint()); h.live.fire(); await flush();
    assert.deepEqual(h.state.runnerLog.value.lines, ['read-2']);
    assert.equal(h.state.runnerLogError.value, '此轮日志暂时不可读取');
    h.live.emit(hint()); h.live.fire(); await flush();
    assert.equal(h.state.runnerLogError.value, ''); assert.deepEqual(h.state.runnerLog.value.lines, ['read-4']);
  });
  test('failed registration retries on reconciliation and returns to event updates', async t => {
    const options = { failure: true };
    const h = await open({}, options); t.after(h.unmount);
    assert.equal(h.live.listening, false); options.failure = false;
    h.live.reconcile(); await flush(); h.live.fire(); await flush();
    assert.equal(h.live.listening, true); assert.equal(h.state.runnerLogConnectionError.value, '');
    const before = h.calls.length; h.live.emit(hint()); h.live.fire(); await flush();
    assert.equal(h.calls.length, before + 1);
  });
  test('unmount releases event, visibility and timers including late registration', async () => {
    const h = await open(); h.live.emit(hint()); h.unmount();
    assert.equal(h.live.timers, 0); assert.equal(h.live.intervals, 0);
    assert.equal(h.live.listening, false); assert.equal(h.live.observing, false); assert.equal(h.live.releases, 1);
    const registration = deferred(); const late = await open({}, { registration: registration.promise });
    late.unmount(); registration.resolve(); await flush();
    assert.equal(late.live.releases, 1); assert.equal(late.live.timers, 0);
  });
};
