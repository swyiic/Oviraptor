const assert = require('node:assert/strict');
const test = require('node:test');
const { renderResultComponent } = require('./result_component_harness.cjs');
const finding = (id, record) => ({ id, recordJson: JSON.stringify(record) });
const render = props => renderResultComponent('SentinelRuntimeExploration.vue', props, {
  '@lucide/vue': { Activity: () => null },
});

test('actual trace pane keeps observed writes separate from action outcomes', async () => {
  const html = await render({
    runtimeFeatureRows: [finding(1, { stateId: 'home', title: '首页', depth: 0, interactiveCount: 2 })],
    runtimeActionRows: [finding(2, { id: 'click-login', label: '登录', outcome: 'observed', requestCount: 1 })],
    observedMutationRows: [finding(3, { method: 'POST', url: '/save', bodyKeys: ['name'], actionId: 'click-login' })],
  });
  for (const value of ['自动探索轨迹', '首页', '登录', '1 个写请求', '被观察并中止', 'POST', '/save'])
    assert.ok(html.includes(value), value);
  assert.ok(!html.includes('已提交写操作'));
});

test('old records show an explicit absence, not invented execution', async () => {
  const html = await render({ runtimeFeatureRows: [], runtimeActionRows: [], observedMutationRows: [] });
  assert.ok(html.includes('旧扫描记录或页面没有可触发控件'));
  assert.ok(!html.includes('被观察并中止的写请求'));
});
