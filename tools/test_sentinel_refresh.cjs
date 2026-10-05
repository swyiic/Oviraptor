const assert = require('node:assert/strict');
const test = require('node:test');
require('./sentinel_board/token_usage.cjs');
require('./sentinel_board/overview_summary.cjs');
require('./sentinel_board/investigation_summary.cjs');
require('./sentinel_board/trace_reads.cjs');
require('./sentinel_board/trace_timeline.cjs');
const { mount, scan, flush } = require('./sentinel_board/harness.cjs');

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('pending event registration cannot block the initial local DB load', async (t) => {
  const registration = deferred();
  const m = await mount(undefined, {}, { listen: () => registration.promise });
  t.after(m.unmount);
  try {
    assert.equal(m.calls.filter(([name]) => name === 'listSentinelScans').length, 1);
  } finally { registration.resolve(() => {}); await flush(); }
});

test('late event registration after unmount is released without starting DB reads', async (t) => {
  const registration = deferred(); let releases = 0;
  const m = await mount(undefined, {}, { listen: () => registration.promise });
  t.after(m.unmount);
  m.unmount(); const reads = m.calls.length;
  registration.resolve(() => { releases++; }); await flush();
  assert.equal(releases, 1);
  assert.equal(m.calls.length, reads);
});

test('event registration failure does not block DB load and emits only a fixed warning', async (t) => {
  const registration = deferred(), errors = [];
  const m = await mount(undefined, {}, { listen: () => registration.promise,
    onError: error => errors.push(error) });
  t.after(m.unmount);
  registration.reject(new Error('/private/token=secret')); await flush();
  assert.deepEqual(errors, []);
  assert.equal(m.calls.filter(([name]) => name === 'listSentinelScans').length, 1);
  assert.deepEqual(m.events, [['error', '身份状态通知连接失败，本地任务数据仍可查看']]);
});

test('registration failure after unmount stays silent', async (t) => {
  const registration = deferred(), errors = [];
  const m = await mount(undefined, {}, { listen: () => registration.promise,
    onError: error => errors.push(error) });
  t.after(m.unmount); m.unmount();
  registration.reject(new Error('/private/token=secret')); await flush();
  assert.deepEqual(errors, []); assert.deepEqual(m.events, []);
});

test('unmount during initial search cannot recreate the live polling timer', async (t) => {
  const search = deferred(); let intervals = 0, searches = 0;
  const m = await mount(undefined, { searchSentinelScanIds: () => { searches++; return search.promise; } }, {
    props: { search: 'fixture' },
    timers: { setInterval: () => ++intervals, clearInterval() {}, setTimeout: () => 1, clearTimeout() {} },
  });
  t.after(m.unmount);
  assert.equal(searches, 1); m.unmount(); search.resolve(['late']); await flush();
  assert.equal(intervals, 0);
  assert.deepEqual(m.b.matchedScanIds.value, []);
});

test('synchronous event bridge failure is contained without losing local data', async (t) => {
  const errors = [];
  const m = await mount(undefined, {}, { listen: () => { throw new Error('private-bridge-detail'); },
    onError: error => errors.push(error) });
  t.after(m.unmount);
  assert.deepEqual(errors, []);
  assert.equal(m.calls.filter(([name]) => name === 'listSentinelScans').length, 1);
  assert.deepEqual(m.events, [['error', '身份状态通知连接失败，本地任务数据仍可查看']]);
});

test('unmount during initial DB load releases listener and prevents polling or another load', async (t) => {
  const read = deferred(); let reads = 0, intervals = 0, releases = 0;
  const m = await mount(undefined, { listSentinelScans: () => { reads++; return read.promise; } }, {
    listen: async () => () => { releases++; },
    timers: { setInterval: () => ++intervals, clearInterval() {}, setTimeout: () => 1, clearTimeout() {} },
  });
  t.after(m.unmount); m.unmount(); read.resolve([]); await flush();
  await m.b.load(); await m.b.liveSync();
  assert.equal(reads, 1); assert.equal(intervals, 0); assert.equal(releases, 1);
});

test('initial search rejection after unmount cannot notify or start polling', async (t) => {
  const search = deferred(); let intervals = 0;
  const m = await mount(undefined, { searchSentinelScanIds: () => search.promise }, {
    props: { search: 'fixture' },
    timers: { setInterval: () => ++intervals, clearInterval() {}, setTimeout: () => 1, clearTimeout() {} },
  });
  t.after(m.unmount); m.unmount(); search.reject(new Error('private-search-detail')); await flush();
  assert.deepEqual(m.events, []); assert.equal(intervals, 0);
});

test('mounted event subscription still updates only an already displayed identity', async (t) => {
  let handler;
  const m = await mount(undefined, {}, { listen: async (name, callback) => {
    assert.equal(name, 'browser-auth-session-updated'); handler = callback; return () => {};
  } });
  t.after(m.unmount);
  m.b.authRecoverySessions.value = [{ id: 'fixture', status: 'pending', name: 'Original' }];
  await handler({ payload: { id: 'unrelated', status: 'pending', name: 'Unrelated' } });
  assert.equal(m.b.authRecoverySessions.value[0].name, 'Original');
  await handler({ payload: { id: 'fixture', status: 'pending', name: 'Current' } });
  assert.equal(m.b.authRecoverySessions.value[0].name, 'Current');
  assert.deepEqual(m.events, []);
});

test('queued listener callback after unmount cannot replace identity display state', async (t) => {
  let handler, releases = 0;
  const m = await mount(undefined, {}, { listen: async (_, callback) => {
    handler = callback; return () => { releases++; };
  } });
  t.after(m.unmount);
  m.b.authRecoverySessions.value = [{ id: 'fixture', status: 'pending', name: 'Original' }];
  m.unmount(); await handler({ payload: { id: 'fixture', status: 'pending', name: 'Late' } });
  assert.equal(releases, 1);
  assert.equal(m.b.authRecoverySessions.value[0].name, 'Original');
  assert.deepEqual(m.events, []);
});

test('mounted listener and polling timer are released once on unmount', async (t) => {
  let releases = 0, refresh; const cleared = [];
  const m = await mount(undefined, {}, {
    listen: async () => () => { releases++; },
    timers: { setInterval: (callback, delay) => { assert.equal(delay, 12000); refresh = callback; return 42; },
      clearInterval: id => cleared.push(id), setTimeout: () => 1, clearTimeout() {} },
  });
  t.after(m.unmount);
  m.props.active = true; await flush(); m.b.scans.value = [scan('active', 'scanning')];
  assert.equal(typeof refresh, 'function'); m.unmount();
  const reads = m.calls.length; await refresh();
  assert.equal(m.calls.length, reads);
  assert.equal(releases, 1); assert.deepEqual(cleared, [42]);
});

test('startup and reactivation read local data without scheduling historical discovery', async (t) => {
  const callbacks = new Map(), writes = [];
  let imports = 0, sequence = 0;
  const m = await mount(undefined, {
    syncSentinelResults: async () => { imports++; throw new Error('retired importer invoked'); },
  }, {
    storage: { getItem: () => null, setItem: (...args) => writes.push(args) },
    timers: {
      setInterval: () => ++sequence, clearInterval() {},
      setTimeout: callback => { callbacks.set(++sequence, callback); return sequence; },
      clearTimeout: id => callbacks.delete(id),
    },
  });
  t.after(m.unmount);
  const initialReads = m.calls.filter(([name]) => name === 'listSentinelScans').length;
  for (let i = 0; i < 2; i++) {
    m.props.active = true; await flush();
    m.props.active = false; await flush();
  }
  assert.ok(m.calls.filter(([name]) => name === 'listSentinelScans').length > initialReads);
  assert.equal(callbacks.size, 0, 'no deferred directory discovery');
  assert.equal(imports, 0);
  assert.deepEqual(writes, []);
  assert.deepEqual(m.events, []);
});

test('overview displays audited source confirmations separately from unavailable and unverified tasks', async () => {
  const m = await mount();
  try {
    m.b.tab.value = 'overview';
    m.b.stats.value = { ...m.b.stats.value, reviewerConfirmedCount: 7, reviewerHighRiskCount: 3,
      otherVulnerabilityCount: 11, sourceReviewerConfirmedCount: 5, sourceReviewAuditedTaskCount: 2,
      sourceReviewUnavailableTaskCount: 4, sourceReviewUnverifiedTaskCount: 1 };
    const html = await m.render();
    assert.match(html, /源码确认 5 条/);
    assert.match(html, /已核验 2 个源码任务/);
    assert.match(html, /尚无可用审查 4/);
    assert.match(html, /无法核验 1/);
    assert.match(html, /候选审查不代表整体覆盖完成或系统无漏洞/);
  } finally { m.unmount(); }
});

test('overview ignores an old project response and clears its previous KPI while loading the new project', async () => {
  const pending = [];
  const m = await mount(undefined, { sentinelOverviewStats: project => new Promise(resolve => pending.push({ project, resolve })) });
  try {
    m.b.stats.value = { reviewerConfirmedCount: 99 };
    m.props.projectId = 2; await flush();
    assert.equal(pending.length, 2);
    assert.equal(m.b.stats.value.reviewerConfirmedCount, 0);
    pending[1].resolve({ reviewerConfirmedCount: 2 }); await flush();
    pending[0].resolve({ reviewerConfirmedCount: 99 }); await flush();
    assert.equal(m.b.stats.value.reviewerConfirmedCount, 2);
  } finally { pending.forEach(p => p.resolve({})); m.unmount(); }
});

test('overview latest same-project refresh wins and unmounted replies do not publish', async () => {
  let delay = false;
  const pending = [];
  const m = await mount(undefined, { sentinelOverviewStats: () => delay
    ? new Promise(resolve => pending.push(resolve)) : Promise.resolve({ reviewerConfirmedCount: 0 }) });
  delay = true;
  const first = m.b.load(); const second = m.b.load();
  try {
    pending[1]({ reviewerConfirmedCount: 2 }); await second;
    pending[0]({ reviewerConfirmedCount: 1 }); await first;
    assert.equal(m.b.stats.value.reviewerConfirmedCount, 2);
    const third = m.b.load(); m.unmount();
    pending[2]({ reviewerConfirmedCount: 3 }); await third;
    assert.equal(m.b.stats.value.reviewerConfirmedCount, 2);
  } finally { pending.forEach(resolve => resolve({})); m.unmount(); }
});

test('overview suppresses old project errors and marks a current read failure unavailable, not clean', async () => {
  const pending = [];
  const m = await mount(undefined, { sentinelOverviewStats: () => new Promise((resolve, reject) => pending.push({ resolve, reject })) });
  try {
    m.props.projectId = 2; await flush();
    pending[1].resolve({ reviewerConfirmedCount: 2 }); await flush();
    pending[0].reject(new Error('old-project-failure')); await flush();
    assert.equal(m.b.stats.value.reviewerConfirmedCount, 2);
    assert.equal(m.b.overviewError.value, false);
    assert.equal(m.events.some(event => String(event).includes('old-project-failure')), false);
    const refresh = m.b.load();
    pending[2].reject(new Error('current-read-failure')); await refresh;
    assert.equal(m.b.overviewError.value, true);
    m.b.tab.value = 'overview';
    const html = await m.render();
    assert.match(html, /暂不可用/);
    assert.doesNotMatch(html, /已核验 \d+ 个源码任务/);
  } finally { pending.forEach(p => p.resolve({})); m.unmount(); }
});

test('overview slow audit prevents periodic polling from stacking more audit requests', async () => {
  let resolve, reads = 0, delay = false;
  const m = await mount(undefined, { sentinelOverviewStats: () => {
    reads++;
    return delay ? new Promise(done => { resolve = done; }) : Promise.resolve({});
  } });
  try {
    m.props.active = true; await flush();
    m.b.scans.value = [scan('running', 'scanning')];
    delay = true;
    const refresh = m.b.load();
    const started = reads;
    for (let i = 0; i < 3; i++) await m.b.liveSync();
    assert.equal(reads, started);
    m.b.tab.value = 'overview';
    assert.match(await m.render(), /核验中/);
    resolve({ reviewerConfirmedCount: 1 }); await refresh;
  } finally { resolve?.({}); m.unmount(); }
});

test('live Native views refresh from the database without polling the historical importer', async (t) => {
  for (const state of ['scanning', 'pausing']) {
    let imports = 0, reads = 0;
    const m = await mount(undefined, {
      syncSentinelResults: async () => { imports++; throw new Error('historical-directory-unavailable'); },
      listSentinelScans: async () => [{ ...scan('native', state), taskName: `snapshot-${++reads}` }],
    });
    t.after(m.unmount);
    m.props.active = true;
    await flush();
    const before = reads;
    for (let i = 1; i <= 2; i++) {
      await m.b.liveSync();
      assert.equal(imports, 0, `${state}: no directory import in the live refresh path`);
      assert.equal(reads, before + i, `${state}: repeated DB reads still run`);
      assert.equal(m.b.scans.value[0].taskName, `snapshot-${before + i}`);
    }
    assert.deepEqual(m.events, [], 'unrelated historical IO cannot fail the current task refresh');
  }
});
