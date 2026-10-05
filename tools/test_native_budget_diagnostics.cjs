const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript } = require('@vue/compiler-sfc');

const filename = path.join(__dirname, '../src/features/sentinel/components/NativeBudgetDiagnostics.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'native-budget-test' });
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});
const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };

async function mount(read) {
  const calls = [];
  let refresh;
  const exports = {};
  new Function('require', 'exports', compiled)((name) => {
    if (name === 'vue') return vue;
    if (name === '../../../api') return { api: { getNativeBudgetDiagnostics: async (...args) => {
      calls.push(args); return read(...args);
    } } };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '../../../composables/useCommittedRefresh') return { useCommittedRefresh: (...args) => { refresh = args; } };
    if (name === '../timeline/eventContract') return { validCollaborationEvent: () => true };
    if (name === './NativeBudgetVector.vue') return { default: () => null };
    throw Error(`Unexpected import: ${name}`);
  }, exports);
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) {
    bindings = setup(props, ctx); return () => null;
  } };
  const props = vue.reactive({ scanId: 'scan-a', attempt: 1 });
  const app = renderer.createApp({ setup: () => () => vue.h(component, props) });
  app.mount({});
  await flush();
  return { b: bindings, props, calls, refresh, unmount: () => app.unmount() };
}

test('budget panel is on demand and never displays raw transport errors', async () => {
  const view = await mount(async () => { throw Error('Bearer private-budget-secret /private/db.sqlite'); });
  assert.equal(view.calls.length, 0);
  view.b.onToggle({ target: { open: true } });
  await view.b.load();
  assert.deepEqual(view.calls, [['scan-a', 1]]);
  assert.match(view.b.error.value, /读取失败/);
  assert.doesNotMatch(view.b.error.value, /private-budget-secret|db\.sqlite/);
  assert.equal(view.b.report.value, undefined);
  view.unmount();
});

test('budget panel rejects malformed receipts and ignores late attempt responses', async () => {
  let resolve;
  const pending = new Promise(done => { resolve = done; });
  const view = await mount(async () => pending);
  view.b.onToggle({ target: { open: true } });
  const loading = view.b.load();
  view.props.attempt = 2;
  resolve({ schema: 'summary_gap_v1', authoritative: false, roots: [], totalRoots: 0, truncated: false });
  await loading;
  await flush();
  assert.equal(view.b.report.value, undefined);
  view.unmount();

  const bad = await mount(async () => ({ schema: 'unknown', authoritative: true, roots: [] }));
  bad.b.onToggle({ target: { open: true } });
  await bad.b.load();
  assert.equal(bad.b.report.value, undefined);
  assert.match(bad.b.error.value, /读取失败/);
  bad.unmount();
});

test('budget panel rejects malformed journal costs without exposing their raw values', async () => {
  const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
    roots: [{ rootRunId: 'root-a', gaps: { appendJournal: [{ dimension: 'target_requests',
      hardLimit: 20, reserved: 0, consumed: 0, indeterminate: 'Bearer private-vector-secret', coverage: 'multi_agent_broker' }] } }],
    totalRoots: 1, truncated: false }));
  view.b.onToggle({ target: { open: true } }); await view.b.load();
  assert.equal(view.b.report.value, undefined);
  assert.match(view.b.error.value, /读取失败/); assert.doesNotMatch(view.b.error.value, /private-vector-secret/);
  view.unmount();
});

test('actual vector component renders unknown costs and keeps coverage gaps visible', async () => {
  const { renderWorkspaceComponent } = require('./result_component_harness.cjs');
  const html = await renderWorkspaceComponent('src/features/sentinel/components/NativeBudgetVector.vue', {
    dimensions: [{ dimension: 'target_requests', hardLimit: 20, reserved: 0, consumed: 2,
      indeterminate: 1, coverage: 'multi_agent_broker' }],
  }, { '../../../i18n': { useI18n: () => ({ tr: zh => zh }) } });
  assert.match(html, /目标请求/); assert.match(html, /未决/); assert.match(html, /unresolved/);
  assert.match(html, /单智能体未接入/); assert.match(html, /不代表十维完整验收/);
});

test('budget panel rejects invalid unclosed call counts and preserves zero and missing distinctions', async () => {
  for (const count of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1, 'Bearer private-call-secret']) {
    const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
      roots: [{ rootRunId: 'root-a', gaps: { unclosedWebModelCalls: count } }], totalRoots: 1, truncated: false }));
    view.b.onToggle({ target: { open: true } }); await view.b.load();
    assert.equal(view.b.report.value, undefined);
    assert.match(view.b.error.value, /读取失败/);
    assert.doesNotMatch(view.b.error.value, /private-call-secret/);
    view.unmount();
  }
  for (const count of [undefined, 0, 3]) {
    const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
      roots: [{ rootRunId: 'root-a', gaps: { unclosedWebModelCalls: count } }], totalRoots: 1, truncated: false }));
    view.b.onToggle({ target: { open: true } }); await view.b.load();
    assert.equal(view.b.report.value.roots[0].gaps.unclosedWebModelCalls, count);
    view.unmount();
  }
});
