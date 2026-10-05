// Characterization and interaction checks for the real Board usage feature.
const assert = require('node:assert/strict');
const test = require('node:test');
const vue = require('vue');
const { mount, scan, flush, TokenOverview, mountView } = require('./harness.cjs');

const rows = () => [
  { ...scan('cloud-reviewed'), llmDeployment: 'cloud', inputTokens: 100, cachedTokens: 40,
    outputTokens: 20, totalTokens: 120, llmRequests: 2 },
  { ...scan('local-partial', 'partial'), scanType: 'web', llmDeployment: 'local', inputTokens: 60,
    cachedTokens: 0, outputTokens: 40, totalTokens: 0, llmRequests: 3 },
  { ...scan('cloud-running', 'scanning'), scanType: 'greybox', llmDeployment: 'cloud', inputTokens: 20,
    cachedTokens: 10, outputTokens: 10, totalTokens: 30, llmRequests: 1 },
  { ...scan('unknown-failed', 'failed'), scanType: 'cicd', llmDeployment: '', inputTokens: 10,
    cachedTokens: 0, outputTokens: 0, totalTokens: 10, llmRequests: 1 },
];
async function fixture(t) {
  const m = await mount(); t.after(m.unmount);
  m.b.scans.value = rows();
  m.b.vulnerabilityScanIds.value = ['cloud-reviewed'];
  m.b.stats.value = { ...m.b.stats.value, reviewerConfirmedCount: 2 };
  return m;
}
const summary = b => ({ total: b.totalTokenUsage.value, requests: b.totalRequestUsage.value,
  cachedRate: b.cacheHitRate.value, zeroYield: b.zeroYieldScans.value.map(row => row.id),
  zeroYieldTokens: b.zeroYieldTokenUsage.value });
const panelHtml = async m => {
  m.b.tab.value = 'overview';
  const html = await m.render();
  const start = html.indexOf('<section class="panel sentinel-token-overview">');
  assert.ok(start >= 0, 'render the actual token panel through the Board template');
  return html.slice(start, html.indexOf('</section>', start));
};

for (const [scope, label] of [['all', '全部部署'], ['cloud', '云端 AI'], ['local', '本地模型']]) {
  test(`token ${scope} scope never divides loaded costs by unrelated project confirmations`, async t => {
    const m = await fixture(t); m.b.tokenScope.value = scope;
    for (const count of [2, 0, 100]) {
      m.b.stats.value = { ...m.b.stats.value, reviewerConfirmedCount: count };
      const html = await panelHtml(m);
      assert.match(html, /确认记录单位成本<\/span><strong>暂不可计算<\/strong>/);
      assert.match(html, /消耗与确认数缺少同范围、同快照的聚合/);
      assert.match(html, /仅统计已加载任务/);
      assert.match(html, /未关联漏洞记录/);
      assert.match(html, /不代表无漏洞或执行完整/);
      assert.doesNotMatch(html, /零漏洞产出|Token \/ Reviewer 确认记录/);
      const boardHtml = await m.render();
      assert.ok(boardHtml.includes(`已加载任务 · ${label}`));
    }
    m.b.scans.value = [rows()[1]];
    m.b.overviewError.value = 'stats unavailable';
    assert.match(await panelHtml(m), /确认记录单位成本<\/span><strong>暂不可计算<\/strong>/);
  });
}

for (const [scope, expected] of Object.entries({
  all: { total: 260, requests: 7, cachedRate: 26,
    zeroYield: ['local-partial', 'unknown-failed'], zeroYieldTokens: 110 },
  cloud: { total: 150, requests: 3, cachedRate: 42, zeroYield: [], zeroYieldTokens: 0 },
  local: { total: 100, requests: 3, cachedRate: 0, zeroYield: ['local-partial'], zeroYieldTokens: 100 },
})) {
  test(`token accounting preserves ${scope} deployment filtering and settled-task classification`, async t => {
    const m = await fixture(t); m.b.tokenScope.value = scope;
    assert.deepEqual(summary(m.b), expected);
    const html = await panelHtml(m);
    assert.match(html, /TOKEN ACCOUNTING/);
    assert.match(html, new RegExp(`<strong>${expected.total}</strong>`));
  });
}

test('loaded token accounting reacts to replaced task rows and evidence IDs without reloading', async t => {
  const m = await fixture(t);
  const calls = m.calls.length;
  m.b.scans.value = [rows()[1]];
  assert.equal(m.b.zeroYieldTokenUsage.value, 100);
  m.b.vulnerabilityScanIds.value = ['local-partial'];
  assert.deepEqual(summary(m.b), { total: 100, requests: 3, cachedRate: 0, zeroYield: [], zeroYieldTokens: 0 });
  assert.equal(m.calls.length, calls, 'usage aggregation must not invoke an API');
});

test('token panel preserves input/cache/output breakdown, type cards and escaped task titles', async t => {
  const m = await fixture(t);
  m.b.scans.value = rows().map((row, index) => ({ ...row,
    taskName: index === 0 ? '<script>task-title</script>' : row.taskName }));
  const html = await panelHtml(m);
  for (const value of [190, 50, 140, 70, 260, 7]) assert.ok(html.includes(`<strong>${value}</strong>`));
  assert.equal((html.match(/<header>/g) || []).length, 4);
  assert.ok(html.includes('&lt;script&gt;task-title&lt;/script&gt;'));
  assert.ok(!html.includes('<script>task-title</script>'));
  assert.match(html, /查看任务证据/);
});

test('an empty token scope renders zeros without a highest-cost action', async t => {
  const m = await fixture(t);
  m.b.scans.value = [];
  assert.deepEqual(summary(m.b), { total: 0, requests: 0, cachedRate: 0, zeroYield: [], zeroYieldTokens: 0 });
  const html = await panelHtml(m);
  assert.doesNotMatch(html, /highest-cost|NaN|Infinity|查看任务证据/);
});

test('token component buttons update scope, reflect selection and emit only the current highest-cost task', async t => {
  const m = await fixture(t), opened = [];
  const view = mountView(() => vue.h(TokenOverview, {
    scope: m.b.tokenScope.value, usage: m.b.tokenUsage,
    'onUpdate:scope': scope => { m.b.tokenScope.value = scope; },
    onOpen: row => opened.push(row.id),
  }));
  t.after(view.unmount);
  const buttons = () => view.findAll('button');
  assert.deepEqual(buttons().slice(0, 3).map(button => button.props['aria-pressed']), [true, false, false]);
  for (const [index, scope, highest] of [[1, 'cloud', 'cloud-reviewed'], [2, 'local', 'local-partial'], [0, 'all', 'cloud-reviewed']]) {
    buttons()[index].props.onClick(); await flush();
    assert.equal(m.b.tokenScope.value, scope);
    assert.equal(buttons()[index].props['aria-pressed'], true);
    assert.equal(buttons().filter(button => button.props['aria-pressed']).length, 1);
    buttons().at(-1).props.onClick();
    assert.equal(opened.at(-1), highest);
  }
  m.b.scans.value = []; await flush();
  assert.equal(buttons().length, 3, 'no stale task action remains after clearing the source rows');
});
