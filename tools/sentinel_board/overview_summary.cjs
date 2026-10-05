const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan, flush, TaskOverview, mountView } = require('./harness.cjs');

async function fixture(t) {
  const m = await mount(); t.after(m.unmount);
  m.b.tab.value = 'overview';
  m.b.stats.value = { ...m.b.emptyOverviewStats(), readyOpportunityCount: 7, opportunityCount: 11,
    taskCount: 999, fingerprintCount: 5, apiCount: 10, endpointCount: 15,
    reviewerConfirmedCount: 20, reviewerHighRiskCount: 3, validatedCount: 25,
    sourceReviewerConfirmedCount: 12, sourceReviewAuditedTaskCount: 4,
    sourceReviewUnavailableTaskCount: 2, sourceReviewUnverifiedTaskCount: 1,
    otherVulnerabilityCount: 8 };
  m.b.scans.value = [scan('a', 'scanning'), scan('b', 'completed'), scan('c', 'completed')];
  return m;
}
function summary(html) {
  const start = html.indexOf('<div class="sentinel-kpis sentinel-kpis-v2">');
  const end = html.indexOf('<section class="panel sentinel-token-overview">', start);
  assert.ok(start >= 0 && end > start, 'render actual summary through the Board');
  return html.slice(start, end);
}

test('overview summary preserves KPI counts and reviewer evidence qualifications', async t => {
  const m = await fixture(t);
  const html = summary(await m.render());
  for (const text of ['<strong>7</strong>', '<strong>1</strong>', '999 个历史任务', '<strong>25</strong>',
    '<strong>20</strong>', '源码确认 12 条', '已核验 4 个源码任务', '尚无可用审查 2', '无法核验 1',
    '其他来源记录（未纳入原生确认）', '候选审查不代表整体覆盖完成或系统无漏洞。']) assert.ok(html.includes(text), text);
});

test('overview distribution preserves record types and proportional scale', async t => {
  const m = await fixture(t);
  const html = summary(await m.render());
  for (const text of ['指纹', 'API', '端点', '原生确认', '人工验证记录', '不代表风险。']) assert.ok(html.includes(text));
  for (const width of [20, 40, 60, 80, 100]) assert.ok(html.includes(`width:${width}%`));
  m.b.stats.value = m.b.emptyOverviewStats();
  const empty = summary(await m.render());
  assert.ok(!empty.includes('NaN') && !empty.includes('Infinity'));
});

test('task status chart counts loaded rows instead of the project task total', async t => {
  const m = await fixture(t);
  let html = summary(await m.render());
  assert.match(html, /task-dot completed[\s\S]*?<strong>2<\/strong>/);
  assert.match(html, /task-dot scanning[\s\S]*?<strong>1<\/strong>/);
  m.b.scans.value = [scan('new', 'failed')];
  html = summary(await m.render());
  assert.match(html, /task-dot completed[\s\S]*?<strong>0<\/strong>/);
  assert.match(html, /task-dot failed[\s\S]*?<strong>1<\/strong>/);
});

for (const state of ['loading', 'error']) test(`overview ${state} does not present old statistics as current`, async t => {
  const m = await fixture(t);
  m.b.overviewLoading.value = state === 'loading';
  m.b.overviewError.value = state === 'error';
  const before = m.b.stats.value;
  const html = summary(await m.render());
  assert.ok(!html.includes('class="bar-row"'), 'do not chart stale project statistics');
  assert.ok(!html.includes('<strong>7</strong>') && !html.includes('<strong>25</strong>'));
  assert.ok(!html.includes('999 个历史任务'));
  assert.ok(html.includes(state === 'loading' ? '核验中' : '暂不可用'));
  assert.ok(html.includes('已加载任务状态'));
  assert.equal(m.b.stats.value, before, 'display state must not erase the retained snapshot');
  m.b.overviewLoading.value = false; m.b.overviewError.value = false;
  assert.ok(summary(await m.render()).includes('class="bar-row"'));
});

function sidebar(html) {
  const start = html.indexOf('<aside class="investigation-sidebar">');
  const end = html.indexOf('</aside>', start);
  assert.ok(start >= 0 && end > start, 'render actual sidebar through the Board');
  return html.slice(start, end);
}

test('overview workflow is a guide, not fabricated completed task stages', async t => {
  const m = await fixture(t);
  const html = sidebar(await m.render());
  assert.ok(html.includes('流程说明（非执行进度）'));
  assert.ok(!html.includes('class="done"'));
  assert.ok(html.includes('以任务详情中的实际记录为准'));
});

test('overview sidebar separates loaded usage, project counts and retained investigation data', async t => {
  const m = await fixture(t);
  m.b.tokenScope.value = 'cloud';
  m.b.investigationStats.value = { averageInformationGain: 31, tokenWorthyCount: 2,
    targetCount: 8, factCount: 17, promotedStrategyCount: 4 };
  const html = sidebar(await m.render());
  for (const text of ['已加载任务 · 云端 AI', '项目可验证机会', '已加载调查摘要（非实时状态）',
    '<dd>7</dd>', '<dd>31/100</dd>', '<dd>2/8</dd>', '<dd>17</dd>']) assert.ok(html.includes(text), text);
});

for (const state of ['loading', 'error']) test(`overview sidebar ${state} does not show stale opportunity counts`, async t => {
  const m = await fixture(t);
  const retained = m.b.stats.value;
  m.b.overviewLoading.value = state === 'loading';
  m.b.overviewError.value = state === 'error';
  const html = sidebar(await m.render());
  assert.ok(!html.includes('<dd>7</dd>'));
  assert.ok(html.includes(state === 'loading' ? '核验中' : '暂不可用'));
  assert.ok(html.includes('已加载活跃任务'));
  assert.equal(m.b.stats.value, retained);
  m.b.overviewLoading.value = false; m.b.overviewError.value = false;
  assert.ok(sidebar(await m.render()).includes('<dd>7</dd>'));
});

test('overview sidebar rendering performs no additional IPC', async t => {
  const m = await fixture(t);
  const before = m.calls.slice();
  sidebar(await m.render());
  m.b.tokenScope.value = 'local';
  assert.ok(sidebar(await m.render()).includes('已加载任务 · 本地模型'));
  assert.deepEqual(m.calls, before);
});

function tasks(html) {
  const start = html.search(/<span class="eyebrow">(?:ALL|LOADED) TASKS/);
  assert.ok(start >= 0, 'render the actual task overview through the Board');
  return html.slice(start);
}

test('task overview explicitly describes loaded and filtered rows, not the whole database', async t => {
  const m = await fixture(t);
  const html = tasks(await m.render());
  assert.ok(html.includes('已加载任务'));
  assert.ok(html.includes('仅展示已加载且符合当前项目与搜索条件的任务，不代表全部历史任务。'));
  assert.equal((html.match(/class="sentinel-task-card"/g) || []).length, 3);
  assert.ok(!html.includes('999 个任务'));
});

test('task overview groups loaded rows by date then type without mutating source order', async t => {
  const m = await fixture(t);
  const rows = ['cicd', 'code', 'greybox', 'web'].map(type => ({ ...scan(`kind-${type}`),
    scanType: type, createdAt: '2026-09-29 09:00:00' }));
  rows.push({ ...scan('older'), createdAt: '2026-09-28 09:00:00' });
  m.b.scans.value = rows;
  const before = m.calls.slice();
  const html = tasks(await m.render());
  const ids = ['kind-web', 'kind-code', 'kind-greybox', 'kind-cicd', 'older'];
  for (let i = 1; i < ids.length; i++) assert.ok(html.indexOf(ids[i - 1]) < html.indexOf(ids[i]));
  assert.deepEqual(m.b.scans.value.map(row => row.id), rows.map(row => row.id));
  assert.deepEqual(m.calls, before, 'grouping/rendering performs no extra IPC');
});

test('task overview preserves escaped titles, fallback dates and row replacement', async t => {
  const m = await fixture(t);
  m.b.scans.value = [{ ...scan('fallback'), taskName: '<script>task-card</script>',
    createdAt: '', updatedAt: '' }];
  let html = tasks(await m.render());
  assert.ok(html.includes('未标注日期'));
  assert.ok(html.includes('&lt;script&gt;task-card&lt;/script&gt;'));
  assert.ok(!html.includes('<script>task-card</script>'));
  m.b.scans.value = [];
  html = tasks(await m.render());
  assert.ok(html.includes('暂无任务'));
  assert.ok(!html.includes('sentinel-task-card'));
});

test('task overview keeps terminal retention and busy action presentation', async t => {
  const m = await fixture(t);
  for (const [status, label] of [['scanning', '停止并保留'], ['pausing', '正在停止'], ['paused', '继续扫描']]) {
    m.b.scans.value = [scan('busy', status)]; m.b.scanControlBusy.value = 'busy';
    const html = tasks(await m.render());
    assert.match(html, new RegExp(`disabled[^>]*>[\\s\\S]*?${label}`));
  }
  m.b.scans.value = [{ ...scan('retained'), administrativeClosureRecorded: true }];
  const html = tasks(await m.render());
  assert.ok(html.includes('查看结果'));
  assert.ok(!html.includes('删除'));
  assert.ok(!html.includes('button danger compact'));
});

test('task overview forwards only selected-row events through the real Board bindings', async t => {
  const m = await fixture(t), emitted = [];
  for (const name of ['openScan', 'pauseScan', 'resumeScan', 'rescan', 'askRemove']) {
    m.b[name] = row => emitted.push([name, row]);
  }
  const findPanel = node => {
    if (node?.type === TaskOverview) return node;
    for (const child of Array.isArray(node?.children) ? node.children : []) {
      const found = findPanel(child); if (found) return found;
    }
  };
  const view = mountView(() => {
    const panel = findPanel(m.vnode()); assert.ok(panel, 'actual Board task component is wired');
    return panel;
  });
  t.after(view.unmount);
  const before = m.calls.slice();
  for (const [status, action] of [['scanning', 'pauseScan'], ['pausing', 'pauseScan'],
    ['paused', 'resumeScan'], ['completed', 'rescan']]) {
    const target = scan(`event-${status}`, status); m.b.scans.value = [target]; await flush();
    const buttons = view.findAll('button');
    let stopped = 0;
    buttons[1].props.onClick({ stopPropagation() { stopped++; } });
    assert.equal(stopped, 1, 'child action does not also open the card');
    assert.deepEqual(emitted.pop(), [action, target]);
    buttons[2].props.onClick({ stopPropagation() {} });
    assert.deepEqual(emitted.pop(), ['askRemove', target], 'deletion stays a parent confirmation request');
    buttons[0].props.onClick({ stopPropagation() {} });
    assert.deepEqual(emitted.pop(), ['openScan', target]);
    const card = view.findAll('article')[0];
    card.props.onClick(); assert.deepEqual(emitted.pop(), ['openScan', target]);
    card.props.onKeydown({ key: 'Enter' }); assert.deepEqual(emitted.pop(), ['openScan', target]);
    card.props.onKeydown({ key: 'Escape' }); assert.equal(emitted.length, 0);
  }
  m.b.scans.value = []; await flush();
  assert.equal(view.findAll('button').length, 0, 'removed rows leave no mounted action');
  assert.deepEqual(m.calls, before, 'the view cannot directly invoke backend operations');
});
