// Real AgentDialog setup/render regressions: status.
require('./agent_dialog/status_window.cjs');
require('./agent_dialog/status_privacy.cjs');
const { assert, test, renderDialog, deferred, flush, scan, status, directive,
  mount, pendingReceipt, event, eventRefreshClock } = require('./agent_dialog_harness.cjs');

test('a newer dispatched status request cannot roll back the database event cursor', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  const first = deferred(), second = deferred();
  let call = 0;
  api.getNativeScanStatus = () => (++call === 1 ? first.promise : second.promise);
  const one = b.loadStatus(), two = b.loadStatus();
  first.resolve({ ...status('A'), latestSequence: 20 });
  await one;
  second.resolve({ ...status('A'), latestSequence: 10 });
  await two;
  assert.equal(b.statusSync.latestSequence.value, 20);
});

test('an old attempt response cannot replace a resumed attempt', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  const first = deferred(), second = deferred();
  let call = 0;
  api.getNativeScanStatus = () => (++call === 1 ? first.promise : second.promise);
  const one = b.loadStatus(), two = b.loadStatus();
  first.resolve({ ...status('A'), attemptNumber: 2, latestSequence: 20 });
  await one;
  second.resolve({ ...status('A'), attemptNumber: 1, latestSequence: 10 });
  await two;
  assert.equal(b.state.value.attemptNumber, 2);
  assert.equal(b.statusSync.latestSequence.value, 20);
});

test('a retry scopes the incremental cursor and replaces the previous attempt chat', async (t) => {
  const oldEvent = event('old-run', 'agent_run', 4);
  const newEvent = event('new-run', 'agent_run', 5);
  const cached = { ...status('A'), latestSequence: 4, timelineBeforeSequence: 4, timeline: [oldEvent] };
  const calls = [];
  let current = cached;
  const { b, unmount } = await mount({ getNativeScanStatus: async (...args) => {
    calls.push(args);
    return current;
  } });
  t.after(unmount);
  current = { ...status('A'), attemptNumber: 2,
    isIncremental: false, latestSequence: 5, timelineBeforeSequence: 5, timeline: [newEvent] };
  await b.loadStatus(4);
  assert.deepEqual(calls.at(-1), ['A', 4, 1]);
  assert.equal(b.state.value.attemptNumber, 2);
  assert.deepEqual(b.state.value.timeline.map((event) => event.id), ['new-run']);
});

test('a full snapshot replaces stale cached messages even within the same attempt', async (t) => {
  let current = { ...status('A'), latestSequence: 4, timelineBeforeSequence: 4,
    timeline: [event('removed', 'agent_run', 4)] };
  const { b, unmount } = await mount({ getNativeScanStatus: async () => current });
  t.after(unmount);
  current = { ...status('A'), isIncremental: false, latestSequence: 5, timelineBeforeSequence: 5,
    timeline: [event('persisted', 'agent_run', 5)] };
  await b.loadStatus(4);
  assert.deepEqual(b.state.value.timeline.map((event) => event.id), ['persisted']);
});

test('status error clears on recovery without erasing an action error', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  api.getNativeScanStatus = async () => { throw new Error('temporary database read failure'); };
  await b.loadStatus();
  assert.equal(b.error.value, '任务状态读取失败，请重试。');
  api.getNativeScanStatus = async (id) => status(id);
  await b.loadStatus();
  assert.equal(b.error.value, '');
});

test('review integrity errors invalidate cached chat and fence older successful reads', async (t) => {
  const cached={...status('A'),latestSequence:5,timelineBeforeSequence:5,timeline:[{id:'review',eventType:'request_review',sequence:5,
    timestamp:'stamp',summary:'cached operator attestation',fromRole:'operator',threadKey:'team'}]};
  const {b,api,unmount}=await mount({getNativeScanStatus:async()=>cached}); t.after(unmount);
  assert.equal(b.state.value.timeline.length,1);
  const stale=deferred(); api.getNativeScanStatus=()=>stale.promise;
  const old=b.loadStatus(5);
  api.getNativeScanStatus=async()=>{throw Error('request_review_event_integrity');};
  await b.loadStatus(5);
  assert.equal(b.state.value,undefined); assert.equal(b.statusSync.latestSequence.value,0);
  assert.equal(b.statusError.value,'任务回执校验失败，已清除缓存，请重新读取。');
  assert.ok(!(await renderDialog(b)).includes('cached operator attestation'));
  stale.resolve({...cached,latestSequence:100}); await old;
  assert.equal(b.state.value,undefined); assert.equal(b.statusError.value,'任务回执校验失败，已清除缓存，请重新读取。');
  const calls=[];
  api.getNativeScanStatus=async(...args)=>{calls.push(args);return {...status('A'),latestSequence:6};};
  await b.loadStatus(5);
  assert.equal(calls[0][1],undefined); // An invalidated cache requires a full snapshot.
  assert.equal(b.statusError.value,''); assert.equal(b.state.value.latestSequence,6);
});

test('administrative closure is a real operator message and corrupt receipts invalidate cached history', async (t) => {
  const cached = { ...status('A'), latestSequence: 9, timelineBeforeSequence: 9,
    stopDiagnostic: { code: 'administratively_closed_unsettled', obligations: [] }, findingCandidateCount: 0, timeline: [{
    id: 'closure', eventType: 'administrative_closure', sequence: 9, timestamp: 'stamp',
    summary: '未知结果与费用占用保留；旧任务不能恢复', fromRole: 'operator', threadKey: 'team',
    deliveryState: 'persisted', status: 'closed_unsettled',
  }] };
  const { b, api, unmount } = await mount({ getNativeScanStatus: async () => cached });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /人工结案 · 未决结果保留/);
  assert.match(html, /未知结果与费用占用保留/);
  const stale = deferred();
  api.getNativeScanStatus = () => stale.promise;
  const old = b.loadStatus(9);
  api.getNativeScanStatus = async () => { throw Error('administrative_closure_event_integrity'); };
  await b.loadStatus(9);
  assert.equal(b.state.value, undefined);
  assert.equal(b.statusSync.latestSequence.value, 0);
  stale.resolve(cached); await old;
  assert.equal(b.state.value, undefined);
  assert.doesNotMatch(await renderDialog(b), /未知结果与费用占用保留/);
});

test('closure handoff renders draft-only provenance and invalidates corrupt cached events', async (t) => {
  const cached = { ...status('A'), latestSequence: 10, timelineBeforeSequence: 10,
    stopDiagnostic: { code: 'administratively_closed_unsettled', obligations: [] }, findingCandidateCount: 0, timeline: [{
    id: 'handoff', eventType: 'closure_handoff', sequence: 10, timestamp: 'stamp',
    summary: '已创建独立草稿；未启动扫描，未结清原任务', fromRole: 'operator', threadKey: 'team',
    deliveryState: 'persisted', status: 'draft_created',
  }] };
  const { b, api, unmount } = await mount({ getNativeScanStatus: async () => cached });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /独立任务交接 · 仅创建草稿/);
  assert.match(html, /未启动扫描，未结清原任务/);
  const stale = deferred();
  api.getNativeScanStatus = () => stale.promise;
  const old = b.loadStatus(10);
  api.getNativeScanStatus = async () => { throw Error('closure_handoff_event_integrity'); };
  await b.loadStatus(10);
  assert.equal(b.state.value, undefined);
  assert.equal(b.statusSync.latestSequence.value, 0);
  stale.resolve(cached); await old;
  assert.equal(b.state.value, undefined);
  assert.doesNotMatch(await renderDialog(b), /未启动扫描，未结清原任务/);
  const calls = [];
  api.getNativeScanStatus = async (...args) => { calls.push(args); return cached; };
  await b.loadStatus(10);
  assert.equal(calls[0][1], undefined);
  assert.equal(b.statusError.value, '');
  assert.match(await renderDialog(b), /独立任务交接 · 仅创建草稿/);
});

test('a newer in-flight delta cannot rebuild a chat cache invalidated by a receipt error', async (t) => {
  const {b,api,unmount}=await mount({getNativeScanStatus:async()=>({...status('A'),latestSequence:5})}); t.after(unmount);
  const failed=deferred(), delta=deferred(); let count=0; const calls=[];
  api.getNativeScanStatus=(...args)=>{
    calls.push(args);
    if(++count===1) return failed.promise;
    if(count===2) return delta.promise;
    return Promise.resolve({...status('A'),latestSequence:7,timelineBeforeSequence:4,timeline:[event('full-history','agent_run',4)]});
  };
  const failure=b.loadStatus(5), newer=b.loadStatus(5);
  failed.reject(Error('request_review_history_integrity')); await failure;
  assert.equal(b.state.value,undefined);
  delta.resolve({...status('A'),isIncremental:true,latestSequence:7,timeline:[]}); await newer;
  assert.equal(calls.length,3); assert.equal(calls[2][1],undefined);
  assert.deepEqual(b.state.value.timeline.map(e=>e.id),['full-history']);
  assert.equal(b.statusError.value,'');
});

test('late project pagination failure cannot write into another project', async (t) => {
  const { b, api, rootProps, unmount } = await mount();
  t.after(unmount);
  const pending = deferred();
  api.listSentinelScans = (projectId) => projectId === 1 ? pending.promise : Promise.resolve([scan('B', projectId)]);
  b.hasMoreScans.value = true;
  const page = b.loadMoreScans();
  rootProps.projectId = 2;
  await flush();
  pending.reject(new Error('old page failed'));
  await page;
  assert.equal(b.error.value, '');
});

test('refreshing recent tasks must not discard an in-flight historical page', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  const pending = deferred();
  api.listSentinelScans = (_project, _limit, cursor) => cursor ? pending.promise : Promise.resolve([scan('A'), scan('B')]);
  b.hasMoreScans.value = true;
  const page = b.loadMoreScans();
  await b.loadScans();
  pending.resolve([scan('older')]);
  await page;
  assert.ok(b.scans.value.some((item) => item.id === 'older'));
});

test('a full recent-page refresh does not reopen exhausted pagination', async (t) => {
  const rows = Array.from({ length: 300 }, (_, i) => scan(i === 0 ? 'A' : `row-${i}`));
  const { b, unmount } = await mount({ listSentinelScans: async (_project, _limit, cursor) => cursor ? [] : rows });
  t.after(unmount);
  assert.equal(b.hasMoreScans.value, true);
  await b.loadMoreScans();
  assert.equal(b.hasMoreScans.value, false);
  await b.loadScans();
  assert.equal(b.hasMoreScans.value, false);
});

test('list refresh failure is handled and recoverable', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  api.listSentinelScans = async () => { throw new Error('list failure'); };
  await b.loadScans();
  assert.equal(b.error.value, '任务列表读取失败，请重试。');
  api.listSentinelScans = async () => [scan('A')];
  await b.loadScans();
  assert.equal(b.error.value, '');
});

test('listener failure is handled without disabling database status reads', async (t) => {
  const { b, unmount } = await mount({}, { listen: async () => { throw new Error('event transport unavailable'); } });
  t.after(unmount);
  await b.loadStatus();
  assert.equal(b.state.value.scanId, 'A');
  assert.equal(b.error.value, '实时事件连接失败，使用数据库轮询。');
});

test('database watermark wins over request order when the earlier call finishes last', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  const first = deferred(), second = deferred();
  let call = 0;
  api.getNativeScanStatus = () => (++call === 1 ? first.promise : second.promise);
  const one = b.loadStatus(), two = b.loadStatus();
  second.resolve({ ...status('A'), latestSequence: 10 });
  await two;
  first.resolve({ ...status('A'), latestSequence: 20 });
  await one;
  assert.equal(b.statusSync.latestSequence.value, 20);
});

const invalidStatusCursors = {
  'negative attempt': { attemptNumber: -1 },
  'fractional attempt': { attemptNumber: 1.5 },
  'string attempt': { attemptNumber: '2' },
  'unsafe attempt': { attemptNumber: Number.MAX_SAFE_INTEGER + 1 },
  'negative watermark': { latestSequence: -1 },
  'fractional watermark': { latestSequence: 5.5 },
  'string watermark': { latestSequence: '6' },
  'unsafe watermark': { latestSequence: Number.MAX_SAFE_INTEGER + 1 },
  'negative history cursor': { timelineBeforeSequence: -1 },
  'fractional history cursor': { timelineBeforeSequence: 1.5 },
  'history cursor past watermark': { timelineBeforeSequence: 7 },
  'nonboolean incremental flag': { isIncremental: 'true' },
  'missing incremental flag': { isIncremental: undefined },
  'nonboolean history flag': { hasEarlierTimeline: 'false' },
  'nonboolean continuation flag': { timelineHasMore: 'false' },
  'continuation on a full snapshot': { timelineHasMore: true },
  'history without a cursor': { hasEarlierTimeline: true, timelineBeforeSequence: 0 },
};

for (const [label, fields] of Object.entries(invalidStatusCursors)) {
  test(`invalid status ${label} cannot publish or poison recovery`, async (t) => {
    const { b, api, unmount } = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 5 }) });
    t.after(unmount);
    const previous = b.state.value;
    api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 6, ...fields });
    await b.loadStatus(5);
    assert.equal(b.statusError.value, '任务状态读取失败，请重试。');
    assert.equal(b.state.value, previous);
    assert.equal(b.statusSync.latestSequence.value, 5);
    api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 6 });
    await b.loadStatus(5);
    assert.equal(b.statusError.value, '');
    assert.equal(b.statusSync.latestSequence.value, 6);
  });
}

test('a malformed first snapshot cannot restore reading preferences before a valid retry', async (t) => {
  let reads = 0;
  const { b, api, unmount } = await mount({
    getNativeScanStatus: async () => ({ ...status('A'), attemptNumber: '1', latestSequence: 5 }),
    getAgentDialogView: async () => { reads++; throw Error('unexpected reading request'); },
  }); t.after(unmount);
  assert.equal(b.state.value, undefined);
  assert.equal(reads, 0);
  assert.equal(b.statusError.value, '任务状态读取失败，请重试。');
  api.getNativeScanStatus = async () => status('A');
  await b.loadStatus(); await flush();
  assert.equal(b.state.value.attemptNumber, 1);
  assert.equal(reads, 1);
});

for (const mode of ['unsolicited', 'different attempt', 'nonadvancing continuation']) {
  test(`an incremental response with ${mode} retains the verified snapshot`, async (t) => {
    const { b, api, unmount } = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 5 }) });
    t.after(unmount);
    const previous = b.state.value;
    api.getNativeScanStatus = async () => ({ ...status('A'), isIncremental: true,
      latestSequence: mode === 'nonadvancing continuation' ? 5 : 6,
      attemptNumber: mode === 'different attempt' ? 2 : 1,
      timelineHasMore: mode === 'nonadvancing continuation' });
    await b.loadStatus(mode === 'unsolicited' ? undefined : 5);
    assert.equal(b.statusError.value, '任务状态读取失败，请重试。');
    assert.equal(b.state.value, previous);
    assert.equal(b.statusSync.latestSequence.value, 5);
    api.getNativeScanStatus = async () => ({ ...status('A'), attemptNumber: 2, latestSequence: 7 });
    await b.loadStatus(5);
    assert.equal(b.statusError.value, '');
    assert.equal(b.state.value.attemptNumber, 2);
    assert.equal(b.statusSync.latestSequence.value, 7);
  });
}

test('a not-yet-started task may have attempt zero and an empty full snapshot', async (t) => {
  const calls = [];
  const { b, unmount } = await mount({ getNativeScanStatus: async (...args) => {
    calls.push(args); return { ...status('A'), attemptNumber: 0 };
  } });
  t.after(unmount);
  assert.equal(b.statusError.value, '');
  assert.equal(b.state.value.attemptNumber, 0);
  assert.equal(b.statusSync.latestSequence.value, 0);
  await b.loadStatus(0);
  assert.deepEqual(calls.at(-1), ['A', undefined, undefined], 'attempt zero is not an incremental cursor scope');
});

for (const failure of ['transport failure', 'invalid cursor']) {
  test(`a catch-up ${failure} backs off instead of retaining the 50ms continuation loop`, async (t) => {
    const clock = eventRefreshClock();
    const { b, api, unmount } = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 5 }) },
      { ...clock, listen: async () => () => {} }); t.after(unmount);
    api.getNativeScanStatus = async () => ({ ...status('A'), isIncremental: true,
      latestSequence: 6, timelineHasMore: true });
    await b.loadStatus(5);
    assert.deepEqual([...clock.timers.values()].map((timer) => timer.delay), [50]);
    api.getNativeScanStatus = async () => {
      if (failure === 'transport failure') throw Error('temporary read failure');
      return { ...status('A'), isIncremental: true, latestSequence: 6, timelineHasMore: true };
    };
    await clock.tick();
    assert.ok(b.statusError.value);
    assert.deepEqual([...clock.timers.values()].map((timer) => timer.delay), [3000]);
    const calls = [];
    api.getNativeScanStatus = async (...args) => {
      calls.push(args); return { ...status('A'), isIncremental: true, latestSequence: 7 };
    };
    await clock.tick(3000);
    assert.deepEqual(calls, [['A', 6, 1]]);
    assert.equal(b.statusError.value, '');
    assert.equal(b.statusSync.latestSequence.value, 7);
    assert.deepEqual([...clock.timers.values()].map((timer) => timer.delay), [15000]);
  });
}
