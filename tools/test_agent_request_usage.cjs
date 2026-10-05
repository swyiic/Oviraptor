// Render the actual production SFC; no source-text substitute assertions.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const filename = path.join(__dirname, '../src/features/sentinel/components/AgentRequestUsage.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'agent-request-usage-test' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'agent-request-usage-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const load = (code) => {
  const exports = {};
  const js = ts.transpileModule(code, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText;
  new Function('require', 'exports', js)(require, exports);
  return exports;
};
const component = { ...load(script.content).default, render: load(template.code).render };
const usage = (override = {}) => ({ available: true, scope: 'agent_budget_lineage',
  includesDeterministicRecon: false, automaticReplayAllowed: false,
  attemptNumber: 3, includedAttempts: [3, 2, 1], executorRecordedRequests: 7,
  externalSurfaceReceivedRequests: 2, externalSurfaceUnresolvedClaims: 1,
  authorizationReceivedRequests: 3, authorizationUnresolvedClaims: 1,
  recordedRequests: 12, budgetCommittedRequests: 14, ...override });
const render = (accounting) => renderToString(vue.createSSRApp(component, { accounting }));

test('actual view separates recorded, received, unresolved and budget counts', async () => {
  const html = await render(usage());
  for (const text of ['已记录合计 12', '执行器账本 7', '公开面已收到响应 2', '公开面未决占用 1',
    '授权对照已记录响应 3', '授权对照未决占用 1',
    '预算已占用 14', '尝试 3、2、1', '不含前置确定性侦察', '不自动重试']) assert.ok(html.includes(text), text);
  assert.ok(!html.includes('已记录合计 14'));
});
test('missing, unavailable and legacy values never render a false zero', async () => {
  for (const value of [undefined, null, {}, { available: false, reasonCode: 'unreadable' }, { targetRequests: 42 }]) {
    const html = await render(value);
    assert.ok(html.includes('请求统计暂不可核验'));
    assert.ok(!html.includes('已记录合计'));
  }
});
test('malformed counts and conflicting lineage are unavailable', async () => {
  for (const override of [{ recordedRequests: 10 }, { budgetCommittedRequests: 11 },
    { includesDeterministicRecon: true }, { automaticReplayAllowed: true },
    { authorizationReceivedRequests: undefined }, { authorizationUnresolvedClaims: -1 },
    { executorRecordedRequests: -1 }, { externalSurfaceReceivedRequests: '2' },
    { recordedRequests: Number.MAX_SAFE_INTEGER + 1 }, { scope: 'all_traffic' },
    { includedAttempts: [3, 3] }, { includedAttempts: [3, 4] }, { includedAttempts: [] },
    { attemptNumber: 1 }, { includedAttempts: [3, '<script>'] }]) {
    const html = await render(usage(override));
    assert.ok(html.includes('请求统计暂不可核验'), JSON.stringify(override));
    assert.ok(!html.includes('<script>'));
  }
});
test('verified zero and fresh attempts are distinguishable from missing accounting', async () => {
  const html = await render(usage({ attemptNumber: 4, includedAttempts: [4],
    executorRecordedRequests: 0, externalSurfaceReceivedRequests: 0,
    authorizationReceivedRequests: 0, authorizationUnresolvedClaims: 0,
    externalSurfaceUnresolvedClaims: 0, recordedRequests: 0, budgetCommittedRequests: 0 }));
  assert.ok(html.includes('已记录合计 0'));
  assert.ok(html.includes('尝试 4'));
  assert.ok(!html.includes('可能已发送'));
});

test('executor unresolved claims are an included subset, never a second charge', async () => {
  const html = await render(usage({ executorUnresolvedClaims: 2 }));
  for (const text of ['执行器未决记录 2', '已包含在执行器账本和预算中', '已记录合计 12', '预算已占用 14'])
    assert.ok(html.includes(text), text);
  for (const n of [-1, null, '1', 8, Number.MAX_SAFE_INTEGER + 1])
    assert.ok((await render(usage({ executorUnresolvedClaims: n }))).includes('请求统计暂不可核验'));
});

test('unresolved claims expose honest reconciliation guidance without a replay control', async () => {
  const html = await render(usage({ executorUnresolvedClaims: 1 }));
  for (const text of ['未决请求核对说明', '本统计不表示任务已经停止', '不要通过重复发送来确认',
    '收到响应头也不代表', '可在请求核对记录中保存人工判断和依据', '没有结算退款或忽略后继续的入口', '新任务也不等于旧请求已解决'])
    assert.ok(html.includes(text), text);
  assert.ok(!html.includes('<button'));
});

test('no reconciliation panel is invented when all available unresolved counts are zero', async () => {
  for (const executorUnresolvedClaims of [undefined, 0]) {
    const html = await render(usage({ executorUnresolvedClaims, externalSurfaceUnresolvedClaims: 0,
      authorizationUnresolvedClaims: 0, budgetCommittedRequests: 12 }));
    assert.ok(!html.includes('请求统计暂不可核验'));
    assert.ok(!html.includes('未决请求核对说明'));
  }
});
