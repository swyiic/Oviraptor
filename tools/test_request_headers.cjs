const assert = require('node:assert/strict');
const test = require('node:test');
const { renderResultComponent } = require('./result_component_harness.cjs');

const render = props => renderResultComponent('SentinelRequestHeaders.vue', {
  observedRequestHeaderRows: [], declaredRequestHeaderRows: [],
  possibleRequestHeaderRows: [], requestHeaderIntelligence: {},
  headerDisplayValue: row => row.value || '已隐藏', ...props,
}, { '@lucide/vue': { Network: () => null } });

test('request header evidence distinguishes observed, declared and possible data', async () => {
  const html = await render({
    observedRequestHeaderRows: [{ name: 'x-trace', value: 'value-1',
      sources: ['browser-extra-info'], occurrences: 2 }],
    declaredRequestHeaderRows: [{ name: 'x-client', value: 'declared', sources: ['js'] }],
    possibleRequestHeaderRows: [{ name: 'cookie', reason: '由浏览器管理' }],
    requestHeaderIntelligence: { summary: { extraInfoHeaderCount: 1 } },
  });
  for (const value of ['已观察 1', '仅声明 1', 'ExtraInfo 1', '运行时真实生效',
    '隐藏补全', 'value-1', 'JS 明确声明', '待确认', '可能存在，不算证据'])
    assert.ok(html.includes(value), value);
});

test('historical records do not claim current request header evidence', async () => {
  const html = await render({});
  assert.ok(html.includes('当前是旧扫描记录'));
  assert.ok(!html.includes('运行时真实生效'));
});
