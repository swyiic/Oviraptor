const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan, flush, investigationSnapshot: summary } = require('./harness.cjs');
async function fixture(t, extraApi = {}) {
  const requests = [];
  const m = await mount(undefined, { investigationOverview: project => new Promise((resolve, reject) => {
    requests.push({ project, resolve, reject });
  }), ...extraApi });
  t.after(() => { m.unmount(); requests.forEach(request => request.resolve(summary(0))); });
  m.b.tab.value = 'overview';
  requests[0].resolve(summary(31)); await flush();
  m.b.stats.value = m.b.emptyOverviewStats();
  return { m, requests };
}
function sidebar(html) {
  const start = html.indexOf('<aside class="investigation-sidebar">');
  const end = html.indexOf('</aside>', start);
  assert.ok(start >= 0 && end > start);
  return html.slice(start, end);
}

test('project switch hides and clears the previous investigation summary before the response', async t => {
  const { m, requests } = await fixture(t);
  assert.ok(sidebar(await m.render()).includes('<dd>31/100</dd>'));
  m.props.projectId = 2; await flush();
  assert.equal(requests[1].project, 2);
  assert.notEqual(m.b.investigationStats.value.averageInformationGain, 31);
  assert.ok(!sidebar(await m.render()).includes('<dd>31/100</dd>'));
  requests[1].resolve(summary(72)); await flush();
  assert.ok(sidebar(await m.render()).includes('<dd>72/100</dd>'));
});

test('failed new-project summary stays unavailable and never reveals raw read errors', async t => {
  const { m, requests } = await fixture(t);
  m.props.projectId = 2; await flush();
  requests[1].reject(new Error('/private/project-a?token=secret')); await flush();
  const html = sidebar(await m.render());
  assert.ok(!html.includes('<dd>31/100</dd>'));
  assert.ok(html.includes('调查摘要暂不可用'));
  assert.deepEqual(m.events, [['error', 'Error: 调查摘要读取失败，请重试']]);
});

test('same-project refresh retains its cache but hides it during pending and failed reads', async t => {
  const { m, requests } = await fixture(t);
  const retained = m.b.investigationStats.value;
  const work = m.b.load();
  const pending = sidebar(await m.render());
  assert.ok(!pending.includes('<dd>31/100</dd>'));
  assert.ok(pending.includes('调查摘要核验中'));
  requests[1].reject(new Error('private')); await work;
  assert.equal(m.b.investigationStats.value, retained);
  assert.ok(sidebar(await m.render()).includes('调查摘要暂不可用'));
  const retry = m.b.load(); requests[2].resolve(summary(55)); await retry;
  assert.ok(sidebar(await m.render()).includes('<dd>55/100</dd>'));
});

for (const failure of [false, true]) test(`obsolete summary ${failure ? 'failure' : 'success'} cannot replace a newer snapshot`, async t => {
  const { m, requests } = await fixture(t);
  const old = m.b.load(), current = m.b.load();
  requests[2].resolve(summary(72)); await current;
  if (failure) requests[1].reject(new Error('obsolete-secret'));
  else requests[1].resolve(summary(99));
  await old;
  assert.ok(sidebar(await m.render()).includes('<dd>72/100</dd>'));
  assert.deepEqual(m.events, []);
});

test('synchronous project round trip cannot adopt an earlier summary response', async t => {
  const { m, requests } = await fixture(t);
  const old = m.b.load();
  m.b.projectFilter.value = 2; m.b.projectFilter.value = undefined;
  requests[1].resolve(summary(99)); await old;
  const html = sidebar(await m.render());
  assert.ok(!html.includes('<dd>99/100</dd>') && !html.includes('<dd>31/100</dd>'));
  assert.ok(html.includes('调查摘要尚未加载'));
});

for (const failure of [false, true]) test(`unmount rejects late summary ${failure ? 'failure' : 'success'}`, async t => {
  const { m, requests } = await fixture(t);
  const retained = m.b.investigationStats.value;
  const work = m.b.load(); m.unmount();
  if (failure) requests[1].reject(new Error('late-secret'));
  else requests[1].resolve(summary(99));
  await work;
  assert.equal(m.b.investigationStats.value, retained);
  assert.deepEqual(m.events, []);
});

test('old read finishing cannot clear a newer summary loading indicator', async t => {
  const { m, requests } = await fixture(t);
  const old = m.b.load(), current = m.b.load();
  requests[1].resolve(summary(99)); await old;
  assert.equal(m.b.investigationSummaryState.value, 'loading');
  const html = sidebar(await m.render());
  assert.ok(html.includes('调查摘要核验中') && !html.includes('<dd>99/100</dd>'));
  requests[2].resolve(summary(72)); await current;
  assert.equal(m.b.investigationSummaryState.value, 'ready');
});

test('summary refresh after unmount does not start another read', async t => {
  const { m, requests } = await fixture(t);
  m.unmount(); await m.b.refreshInvestigationSummary();
  assert.equal(requests.length, 1);
});

test('background task refresh uses the same summary state and sanitized failure boundary', async t => {
  const { m, requests } = await fixture(t);
  m.props.active = true; await flush();
  requests[1].resolve(summary(31)); await flush();
  m.b.scans.value = [scan('active', 'scanning')];
  const work = m.b.liveSync();
  assert.equal(requests.length, 3);
  assert.equal(m.b.investigationSummaryState.value, 'loading');
  requests[2].reject(new Error('background-secret')); await work;
  assert.equal(m.b.investigationSummaryState.value, 'error');
  assert.ok(!sidebar(await m.render()).includes('<dd>31/100</dd>'));
  assert.deepEqual(m.events, []);
});

test('evidence refresh updates project summary without granting it task authority', async t => {
  const { m, requests } = await fixture(t, { listSentinelValidations: async () => [],
    listInvestigationValidations: async () => [], listAppSecScanResult: async () => ({ vulnerabilities: [], sources: [] }) });
  m.b.selected.value = scan('read-only');
  const work = m.b.refreshSelectedEvidenceAfterValidation();
  assert.equal(m.b.investigationSummaryState.value, 'loading');
  requests[1].resolve(summary(72)); await work;
  assert.equal(m.b.investigationStats.value.averageInformationGain, 72);
  assert.equal(m.b.selected.value.status, 'completed');
  assert.deepEqual(m.events, []);
});

test('summary is independently available even if another read-only panel fails', async t => {
  let failTargets = false;
  const { m, requests } = await fixture(t, { listSentinelTargets: async () => {
    if (failTargets) throw new Error('fixture target read failed');
    return [];
  } });
  failTargets = true;
  await m.b.load();
  requests[1].resolve(summary(72)); await flush();
  assert.equal(m.b.investigationSummaryState.value, 'ready');
  assert.equal(m.b.investigationStats.value.averageInformationGain, 72);
});

for (const [name, response] of [['null', null], ['array', []], ['empty', {}],
  ['string count', { ...summary(12), factCount: '17' }],
  ['negative count', { ...summary(12), targetCount: -1 }],
  ['fractional gain', { ...summary(12), averageInformationGain: 0.5 }],
  ['NaN', { ...summary(12), nodeCount: NaN }],
  ['infinity', { ...summary(12), edgeCount: Infinity }],
  ['unsafe integer', { ...summary(12), apiCount: Number.MAX_SAFE_INTEGER + 1 }]]) {
  test(`malformed ${name} summary is rejected without replacing the retained snapshot`, async t => {
    const { m, requests } = await fixture(t);
    const retained = m.b.investigationStats.value;
    const work = m.b.load(); requests[1].resolve(response); await work;
    assert.equal(m.b.investigationSummaryState.value, 'error');
    assert.equal(m.b.investigationStats.value, retained);
    const html = sidebar(await m.render());
    assert.ok(html.includes('调查摘要暂不可用') && !html.includes('<dd>31/100</dd>'));
    assert.deepEqual(m.events, [['error', 'Error: 调查摘要读取失败，请重试']]);
  });
}

test('every declared summary metric is required, including metrics not shown in the sidebar', async t => {
  const { m, requests } = await fixture(t);
  for (const key of Object.keys(summary(12))) {
    const response = summary(12); delete response[key];
    const work = m.b.load(); requests.at(-1).resolve(response); await work;
    assert.equal(m.b.investigationSummaryState.value, 'error', key);
    assert.equal(m.b.investigationStats.value.averageInformationGain, 31);
  }
});

test('zero-valued summary and unknown additional metadata remain compatible after failure', async t => {
  const { m, requests } = await fixture(t);
  const failed = m.b.load(); requests[1].resolve(null); await failed;
  const valid = Object.fromEntries(Object.keys(summary(0)).map(key => [key, 0]));
  valid.futureMetadata = { version: 2 };
  const retry = m.b.load(); requests[2].resolve(valid); await retry;
  assert.equal(m.b.investigationSummaryState.value, 'ready');
  assert.equal(m.b.investigationStats.value.averageInformationGain, 0);
  assert.ok(sidebar(await m.render()).includes('<dd>0/100</dd>'));
});
