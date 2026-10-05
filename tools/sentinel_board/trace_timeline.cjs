const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan } = require('./harness.cjs');
const { trace } = require('../agent_trace_harness.cjs');
const { renderResultComponent } = require('../result_component_harness.cjs');
const event = (id, targetUrl = '', extra = {}) => ({ id, targetUrl, sessionId: 'session-a',
  callId: '', eventType: 'function_call', name: 'fixture_tool', status: 'recorded', role: 'tool',
  detail: `body-${id}`, detailSize: 1, detailTruncated: false, createdAt: 'recorded-time', ...extra });
const render = props => renderResultComponent('SentinelTraceTimeline.vue', props, {
  '@lucide/vue': new Proxy({}, { get: () => () => null }),
});

test('Board historical timeline describes recorded evidence, never live execution', async t => {
  const m = await mount(); t.after(m.unmount);
  m.b.tab.value = 'results'; m.b.resultTab.value = 'summary';
  m.b.selected.value = { ...scan('A', 'scanning'), scanType: 'web' };
  m.b.selectedUrl.value = '*';
  const detail = trace('A'); detail.summary.sourceAuthority = 'historical_external';
  detail.events = [event('historical')]; m.b.liveTrace.value = detail;
  const html = await m.render();
  assert.match(html, /历史外部记录/);
  assert.match(html, /最近记录/);
  assert.doesNotMatch(html, /\bLIVE\b|正在执行 fixture_tool|模型正在判断/);
});

test('timeline displays task totals separately from target-filtered records', async () => {
  const detail = trace('A'); detail.summary.llmRequests = 17;
  detail.events = [event('match', 'https://a.invalid/path/?x=1'), event('global'),
    event('other', 'https://b.invalid/path'), event('path-case', 'https://a.invalid/Path')];
  const before = JSON.stringify(detail);
  const html = await render({ detail, busy: false, targetUrl: 'https://a.invalid/path', taskStatus: 'completed' });
  assert.match(html, /Native 账本记录/);
  assert.match(html, /任务级累计/);
  assert.match(html, /body-match/); assert.match(html, /body-global/);
  assert.doesNotMatch(html, /body-other|body-path-case/);
  assert.equal(JSON.stringify(detail), before);
});

test('timeline preserves returned order, limits recent events, and escapes displayed text', async () => {
  const detail = trace('A');
  detail.events = Array.from({ length: 35 }, (_, i) => event(`entry-${i}-end`));
  detail.events[34].detail = '<img src=x onerror=bad()>public';
  const html = await render({ detail, busy: false, targetUrl: '*', taskStatus: 'completed' });
  assert.doesNotMatch(html, /body-entry-[0-4]-end|<img/);
  assert.match(html, /body-entry-5-end/); assert.match(html, /&lt;img/);
  assert.ok(html.indexOf('body-entry-33-end') < html.indexOf('body-entry-32-end'));
  assert.equal((html.match(/<article class="function_call">/g) || []).length, 30);
});

test('timeline distinguishes loading and missing evidence without promising automatic stops', async () => {
  const loading = await render({ busy: true, targetUrl: '*', taskStatus: 'scanning' });
  assert.match(loading, /正在读取/); assert.doesNotMatch(loading, /LIVE|正在执行/);
  const empty = await render({ busy: false, targetUrl: '*', taskStatus: 'scanning' });
  assert.match(empty, /尚无可展示的结构化事件/);
  assert.doesNotMatch(empty, /自动停止|无进展|LIVE/);
  assert.doesNotMatch(await render({ busy: false, targetUrl: '*', taskStatus: 'completed' }), /<section/);
});

test('timeline refreshing a snapshot does not label it as current agent activity', async () => {
  const detail = trace('A'); detail.events = [event('return', '', { eventType: 'function_call_output' })];
  const html = await render({ detail, busy: true, targetUrl: '*', taskStatus: 'scanning' });
  assert.match(html, /刷新中/); assert.match(html, /fixture_tool 返回结果/);
  assert.doesNotMatch(html, /模型正在判断|当前步骤|LIVE/);
});

test('timeline preserves unknown event labels and truncation without inventing tool calls', async () => {
  const detail = trace('A'); detail.events = [event('review', '', {
    eventType: 'review_recorded', name: 'reviewer-note', detailTruncated: true,
  })];
  const html = await render({ detail, busy: false, targetUrl: '*', taskStatus: 'completed' });
  assert.match(html, /review_recorded/); assert.match(html, /限长预览/);
  assert.match(html, /reviewer-note/); assert.doesNotMatch(html, /调用 reviewer-note/);
});
