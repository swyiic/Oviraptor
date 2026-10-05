// Real AgentDialog setup/render regressions: events.
const { assert, test, deferred, flush, eventRefreshClock, status, mount, event } = require('./agent_dialog_harness.cjs');
require('./agent_dialog/status_lifecycle.cjs');

for (const phase of ['scanning', 'pausing', 'active-followup']) {
  test(`connected ${phase} chat uses event updates and only slow missed-event reconciliation`, async t => {
    const clock = eventRefreshClock(); let notify;
    const taskStatus = phase === 'active-followup' ? 'completed' : phase;
    const task = { id: 'A', projectId: 1, updatedAt: 'A', status: taskStatus };
    const snapshot = { ...status('A'), status: taskStatus, latestSequence: 1,
      timelineBeforeSequence: 1, timeline: [event('initial', 'mailbox_message', 1)],
      ...(phase === 'active-followup' ? { followup: { tasks: [{ scanId: 'child', status: 'scanning' }] } } : {}) };
    const h = await mount({ listSentinelScans: async () => [task], getAgentDialogTask: async () => task,
      getNativeScanStatus: async () => snapshot,
    }, { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
    t.after(h.unmount);
    assert.deepEqual([...clock.timers.values()].map(timer => timer.delay), [15000]);
    const reads = [];
    h.api.getNativeScanStatus = async (...args) => {
      reads.push(args); return { ...snapshot, isIncremental: true, latestSequence: 2,
        timelineBeforeSequence: 2, timeline: [event('committed', 'mailbox_message', 2)] };
    };
    notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 2 } });
    await clock.tick(50);
    assert.deepEqual(reads, [['A', 1, 1]]);
    assert.equal(h.b.state.value.timeline.at(-1).id, 'committed');
    assert.deepEqual([...clock.timers.values()].map(timer => timer.delay), [15000]);
  });
}

test('browser fixture intercepts split chat modules and refuses other native transports', async () => {
  const path = require('node:path');
  const { chatDomTransport } = await import('./serve_chat_dom.mjs');
  const repo = path.resolve(__dirname, '..');
  const mock = path.join(repo, 'tools/chat_dom/transport.mjs');
  for (const file of ['components/AgentDialog.vue', ...['Tasks', 'Status', 'Reading', 'History', 'Directives', 'Followup']
    .map((name) => `composables/useAgentDialog${name}.ts`)]) {
    const importer = path.join(repo, 'src/features/sentinel', file);
    assert.equal(chatDomTransport.resolveId('../api', importer), mock, file);
    assert.equal(chatDomTransport.resolveId('@tauri-apps/api/event', `${importer}?v=fixture`), mock, file);
    assert.equal(chatDomTransport.resolveId('vue', importer), undefined);
    assert.throws(() => chatDomTransport.resolveId('@tauri-apps/api/core', importer), /fixture_native_transport_forbidden/);
  }
  assert.throws(() => chatDomTransport.resolveId('@tauri-apps/api/event', path.join(repo, 'src/other.ts')),
    /fixture_native_transport_forbidden/);
});

test('browser fixture feeds real chat state, bounded history and a fresh attempt without execution', async (t) => {
  const fixture = await import('./chat_dom/transport.mjs?event-contract');
  const h = await mount(fixture.sentinelApi, { listen: fixture.listen });
  t.after(h.unmount);
  assert.equal(h.b.state.value?.attemptNumber, 1);
  assert.equal(h.b.error.value, '');
  fixture.burst('A', 400);
  for (const after of [1, 101, 201, 301]) {
    await h.b.loadStatus(after);
    assert.equal(h.b.error.value, '');
    assert.equal(h.b.state.value.latestSequence, after + 100);
    assert.equal(h.b.state.value.timelineHasMore, after < 301);
  }
  assert.equal(h.b.state.value.latestSequence, 401);
  assert.equal(h.b.state.value.timeline.length, 300);
  const page = await fixture.sentinelApi.getNativeScanTimelinePage('A', 1, 102);
  assert.equal(page.beforeSequence, 102);
  assert.equal(page.timelineBeforeSequence, 2);
  assert.equal(page.hasEarlierTimeline, true);
  assert.equal(page.timeline.length, 100);
  fixture.newAttempt('A');
  await h.b.loadStatus(401);
  assert.equal(h.b.state.value.attemptNumber, 2);
  assert.equal(h.b.state.value.timeline.length, 1);
  await assert.rejects(fixture.sentinelApi.getNativeScanTimelinePage('A', 1, 102), /fixture_attempt_changed/);
  for (const action of ['draftScanDirective', 'confirmScanDirective', 'cancelScanDirective',
    'reconcileScanDirectiveReceipt', 'reconcileHistoricalScanDirectiveReceipt', 'previewAgentGapFollowup']) {
    await assert.rejects(fixture.sentinelApi[action](), /fixture_action_unavailable_no_execution/);
  }
});

test('late event listener installation is immediately disposed after unmount', async () => {
  const pending = deferred();
  let stopped = 0;
  const { unmount } = await mount({}, { listen: () => pending.promise });
  unmount();
  pending.resolve(() => { stopped++; });
  await flush();
  assert.equal(stopped, 1);
});

test('a connected listener still catches up a dropped terminal-task event from the database', async () => {
  const clock = eventRefreshClock(); let notify;
  const original = { ...status('A'), latestSequence: 4, timelineBeforeSequence: 4,
    timeline: [event('original', 'mailbox_message', 4)] };
  const { b, api, unmount } = await mount({ getNativeScanStatus: async () => original },
    { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  try {
    assert.equal(typeof notify, 'function');
    const calls = [];
    api.getNativeScanStatus = async (...args) => {
      calls.push(args);
      return { ...status('A'), isIncremental: true, latestSequence: 5, timelineBeforeSequence: 5,
        timeline: [event('missed', 'mailbox_message', 5)] };
    };
    // No event is delivered. The visible, terminal task must still reconcile
    // its persisted sequence without reloading or marking any message read.
    assert.equal([...clock.timers.values()].filter((timer) => timer.delay === 15000).length, 1);
    await clock.tick(15000);
    assert.deepEqual(calls, [['A', 4, 1]]);
    assert.deepEqual(b.state.value.timeline.map((item) => item.id), ['original', 'missed']);
    assert.equal(b.statusSync.latestSequence.value, 5);
  } finally { unmount(); }
  assert.equal(clock.timers.size, 0);
});

test('an empty terminal timeline uses an incremental watermark for periodic checks', async () => {
  const clock = eventRefreshClock();
  const { api, unmount } = await mount({}, { ...clock });
  try {
    const calls = [];
    api.getNativeScanStatus = async (...args) => {
      calls.push(args);
      return { ...status('A'), isIncremental: true };
    };
    await clock.tick(15000);
    assert.deepEqual(calls, [['A', 0, 1]], 'watermark zero is not an instruction to reread full history');
  } finally { unmount(); }
});

test('collaboration event bursts have one active read and at most one trailing catch-up', async (t) => {
  const clock = eventRefreshClock();
  let notify;
  const { b, api, unmount } = await mount({ getNativeScanStatus: async (id) => ({ ...status(id), latestSequence: 1 }) },
    { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const pending = deferred(), calls = [];
  api.getNativeScanStatus = (...args) => { calls.push(args); return calls.length === 1 ? pending.promise
    : Promise.resolve({ ...status('A'), isIncremental: true, latestSequence: 400,
      timelineBeforeSequence: 400, timeline: [event('last', 'mailbox_message', 400)] }); };
  for (let sequence = 2; sequence <= 200; sequence++) notify({ payload: { scanId: 'A', attemptNumber: 1, sequence } });
  assert.equal(calls.length, 0, 'notifications schedule a bounded batch, not immediate IPC reads');
  assert.equal(clock.timers.size, 1);
  await clock.tick();
  assert.deepEqual(calls, [['A', 1, 1]]);
  for (let sequence = 201; sequence <= 400; sequence++) notify({ payload: { scanId: 'A', attemptNumber: 1, sequence } });
  await clock.tick();
  assert.equal(calls.length, 1, 'no overlapping event reads while transport is unresolved');
  pending.resolve({ ...status('A'), isIncremental: true, latestSequence: 200,
    timelineBeforeSequence: 200, timeline: [event('first', 'mailbox_message', 200)] });
  await flush(); await clock.tick();
  assert.deepEqual(calls, [['A', 1, 1], ['A', 200, 1]]);
  assert.deepEqual(b.state.value.timeline.map((item) => item.id), ['first', 'last']);
  await clock.tick();
  assert.equal(calls.length, 2);
});

test('a response covering queued event watermarks avoids a redundant trailing read', async (t) => {
  const clock = eventRefreshClock(); let notify;
  const { api, unmount } = await mount({}, { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const pending = deferred(); let reads = 0;
  api.getNativeScanStatus = () => { reads++; return pending.promise; };
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 2 } }); await clock.tick();
  for (let i = 0; i < 100; i++) notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 3 } });
  pending.resolve({ ...status('A'), latestSequence: 3 }); await flush(); await clock.tick();
  assert.equal(reads, 1);
});

test('event hints validate identity and compare attempt before sequence', async (t) => {
  const clock = eventRefreshClock(); let notify;
  const { b, api, unmount } = await mount({ getNativeScanStatus: async (id) => ({ ...status(id), latestSequence: 90 }) },
    { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const calls = [];
  api.getNativeScanStatus = async (...args) => { calls.push(args); return { ...status('A'), attemptNumber: 2, latestSequence: 1 }; };
  for (const payload of [null, undefined, {}, { scanId: 'A', attemptNumber: 2, sequence: NaN },
    { scanId: 'A', attemptNumber: 0, sequence: 100 }, { scanId: 'A', attemptNumber: 1, sequence: Number.MAX_SAFE_INTEGER + 1 },
    { scanId: 'A', attemptNumber: '2', sequence: 100 }, { scanId: 'A', attemptNumber: 2, sequence: -1 }]) {
    assert.doesNotThrow(() => notify({ payload }));
  }
  await clock.tick(); assert.equal(calls.length, 0);
  notify({ payload: { scanId: 'A', attemptNumber: 2, sequence: 1 } });
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 999 } }); await clock.tick();
  assert.deepEqual(calls, [['A', undefined, undefined]]);
  assert.equal(b.state.value.attemptNumber, 2);
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 999 } }); await clock.tick();
  assert.equal(calls.length, 1, 'old attempts cannot cause a refresh storm');
});

test('event refresh queues are discarded on task change and unmount', async () => {
  const clock = eventRefreshClock(); let notify;
  const { b, api, unmount } = await mount({}, { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  const calls = [];
  api.getNativeScanStatus = async (id, after) => { calls.push([id, after]); return status(id); };
  try {
    notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 5 } });
    b.scanId.value = 'B'; await flush();
    const count = calls.length; await clock.tick();
    assert.equal(calls.length, count, 'queued A notification must not read B');
    notify({ payload: { scanId: 'B', attemptNumber: 1, sequence: 5 } });
    assert.equal(clock.timers.size, 1);
  } finally { unmount(); }
  assert.equal(clock.timers.size, 0);
  const count = calls.length;
  notify({ payload: { scanId: 'B', attemptNumber: 1, sequence: 6 } }); await clock.tick();
  assert.equal(calls.length, count);
});

test('an event read failure on a terminal task retains bounded database polling', async (t) => {
  const clock = eventRefreshClock(); let notify;
  const { b, api, unmount } = await mount({}, { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  api.getNativeScanStatus = async () => { throw Error('temporary database error'); };
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 5 } }); await clock.tick();
  assert.equal(b.statusError.value, '任务状态读取失败，请重试。');
  assert.equal([...clock.timers.values()].filter((timer) => timer.delay === 3000).length, 1);
  api.getNativeScanStatus = async (id) => ({ ...status(id), latestSequence: 5 });
  await clock.tick(3000);
  assert.equal(b.statusSync.latestSequence.value, 5);
  assert.equal(b.statusError.value, '');
  assert.equal([...clock.timers.values()].filter((timer) => timer.delay === 15000).length, 1,
    'successful terminal recovery switches to a low-frequency missed-event check');
});

test('a stale task event read cannot release the new task single-flight queue', async (t) => {
  const clock = eventRefreshClock(); let notify;
  const { b, api, unmount } = await mount({}, { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const old = deferred(), current = deferred(), calls = []; let bReads = 0;
  api.getNativeScanStatus = (id, after) => {
    calls.push([id, after]);
    if (id === 'A') return old.promise;
    bReads++;
    if (bReads === 2) return current.promise;
    return Promise.resolve({ ...status(id), latestSequence: bReads === 1 ? 0 : 3 });
  };
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 2 } }); await clock.tick();
  b.scanId.value = 'B'; await flush();
  notify({ payload: { scanId: 'B', attemptNumber: 1, sequence: 2 } }); await clock.tick();
  assert.equal(calls.length, 3, 'B refresh does not wait on the obsolete A IPC');
  old.resolve({ ...status('A'), latestSequence: 999 }); await flush();
  assert.equal(b.state.value.scanId, 'B');
  notify({ payload: { scanId: 'B', attemptNumber: 1, sequence: 3 } }); await clock.tick();
  assert.equal(calls.length, 3, 'obsolete finally cannot unlock B and create an overlapping read');
  current.resolve({ ...status('B'), latestSequence: 2 }); await flush(); await clock.tick();
  assert.deepEqual(calls[3], ['B', 2]);
  assert.equal(b.statusSync.latestSequence.value, 3);
});

test('queued events rebuild full history after receipt integrity invalidates the cache', async (t) => {
  const clock = eventRefreshClock(); let notify;
  const { b, api, unmount } = await mount({ getNativeScanStatus: async (id) => ({ ...status(id), latestSequence: 5 }) },
    { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const pending = deferred(), calls = [];
  api.getNativeScanStatus = (...args) => {
    calls.push(args);
    return calls.length === 1 ? pending.promise : Promise.resolve({ ...status('A'), latestSequence: 7, timelineBeforeSequence: 1,
      timeline: [event('historical', 'mailbox_message', 1), event('recovered', 'request_review', 7)] });
  };
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 6 } }); await clock.tick();
  notify({ payload: { scanId: 'A', attemptNumber: 1, sequence: 7 } });
  pending.reject(Error('request_review_event_integrity')); await flush();
  assert.equal(b.state.value, undefined);
  await clock.tick();
  assert.deepEqual(calls, [['A', 5, 1], ['A', undefined, undefined]]);
  assert.deepEqual(b.state.value.timeline.map((item) => item.id), ['historical', 'recovered']);
  assert.equal(b.statusError.value, '');
  assert.equal([...clock.timers.values()].filter((timer) => timer.delay === 15000).length, 1);
});

test('events load committed deltas without duplication and reset the timeline for a new attempt', async (t) => {
  const clock = eventRefreshClock();
  let notify;
  const { b, api, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1, timeline: [event('same-id', 'assignment', 1)],
  }) }, { ...clock, listen: async (_name, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const calls = [];
  api.getNativeScanStatus = async (...args) => {
    calls.push(args);
    return { ...status('A'), isIncremental: true, latestSequence: 3, timelineBeforeSequence: 2, timeline: [
      event('same-id', 'assignment', 2, 'completed'), event('same-id', 'mailbox_message', 3),
    ] };
  };
  notify({ payload: { scanId: 'unrelated', sequence: 2, attemptNumber: 1 } });
  notify({ payload: { scanId: 'A', sequence: 1, attemptNumber: 1 } });
  assert.equal(calls.length, 0);
  notify({ payload: { scanId: 'A', sequence: 3, attemptNumber: 1 } });
  await clock.tick();
  assert.deepEqual(calls, [['A', 1, 1]]);
  assert.equal(b.state.value.timeline.length, 2);
  assert.equal(b.state.value.timeline[0].summary, 'completed');
  notify({ payload: { scanId: 'A', sequence: 3, attemptNumber: 1 } });
  assert.equal(calls.length, 1, 'duplicate notification causes no duplicate request');
  api.getNativeScanStatus = async (...args) => {
    calls.push(args);
    return { ...status('A'), attemptNumber: 2, latestSequence: 4, timelineBeforeSequence: 4, timeline: [event('new-run', 'agent_run', 4)] };
  };
  notify({ payload: { scanId: 'A', sequence: 4, attemptNumber: 2 } });
  await clock.tick();
  assert.deepEqual(calls[1], ['A', undefined, undefined]);
  assert.deepEqual(b.state.value.timeline.map((row) => row.id), ['new-run']);
});

test('failed event transport really polls the database and cancels polling on unmount', async () => {
  const timers = new Map();
  let nextTimer = 0;
  const { b, api, unmount } = await mount({}, {
    listen: async () => { throw new Error('event transport unavailable'); },
    setTimeout: (callback) => { const id = ++nextTimer; timers.set(id, callback); return id; },
    clearTimeout: (id) => timers.delete(id),
  });
  try {
    assert.equal(timers.size, 1);
    api.getNativeScanStatus = async (id) => ({ ...status(id), latestSequence: 5, timelineBeforeSequence: 5,
      timeline: [event('missed', 'mailbox_message', 5)] });
    const [id, poll] = timers.entries().next().value;
    timers.delete(id);
    poll();
    await flush();
    assert.equal(b.statusSync.latestSequence.value, 5);
    assert.equal(b.state.value.timeline[0].id, 'missed');
    assert.equal(timers.size, 1);
  } finally { unmount(); }
  assert.equal(timers.size, 0);
});
