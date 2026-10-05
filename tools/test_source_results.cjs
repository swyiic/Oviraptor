// Mount both production SFCs with their real render functions and lifecycle.
// Only IPC, icons, and locale are substituted. This is not a desktop WebView test.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const transpile = text => ts.transpileModule(text, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const presentation = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../src/features/sentinel/presentation.ts'), 'utf8')))(require, presentation);
function component(relative, imports) {
  const filename = path.join(__dirname, '../src/features/sentinel/components', relative);
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id: relative });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: relative,
    compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, []);
  const exports = {}, rendered = {};
  new Function('require', 'exports', transpile(script.content))(imports, exports);
  new Function('require', 'exports', transpile(template.code))(require, rendered);
  exports.default.render = rendered.render;
  return exports.default;
}
const node = (type, text = '') => ({ type, text, children: [], props: {}, parent: null });
function remove(child) {
  if (!child.parent) return;
  const children = child.parent.children, index = children.indexOf(child);
  if (index >= 0) children.splice(index, 1);
  child.parent = null;
}
// v-model's select directive needs these minimal host DOM methods/properties.
const baseCreateElement = type => {
  const n = node(type); n.listeners = {}; n.addEventListener = (event, callback) => { n.listeners[event] = callback; };
  Object.defineProperty(n, 'options', { get: () => n.children.filter(child => child.type === 'option') });
  return n;
};
const hostRenderer = vue.createRenderer({
  createElement: baseCreateElement, createText: text => node('#text', text), createComment: text => node('#comment', text),
  insert(child, parent, anchor) { remove(child); const index = anchor ? parent.children.indexOf(anchor) : -1;
    parent.children.splice(index < 0 ? parent.children.length : index, 0, child); child.parent = parent; },
  remove, setText: (n, text) => { n.text = text; },
  setElementText(n, text) { n.children = []; n.text = text; },
  patchProp(n, key, _, value) { n.props[key] = value; if (key === 'value') { n.value = value; n._value = value; } },
  parentNode: n => n.parent, nextSibling: n => n.parent?.children[n.parent.children.indexOf(n) + 1] ?? null,
});
const flush = async () => { for (let i = 0; i < 15; i++) await vue.nextTick(); };
const text = n => (n.type === '#comment' ? '' : n.text) + n.children.map(text).join(' ');
const find = (n, predicate) => predicate(n) ? n : n.children.map(child => find(child, predicate)).find(Boolean);
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
const report = (scanId, attemptNumber, status = 'audited') => ({ schemaVersion: 1, scanId, attemptNumber, status,
  rootRunId: 'root', materialDigest: status === 'audited' ? 'digest' : null,
  counts: status === 'audited' ? { decisions: 1, confirmed: 1, rejected: 0, insufficient: 0 } : null,
  independentCandidateReviewCompleted: status === 'audited', independentReviewCompleted: false,
  offset: 0, nextOffset: null, findings: status === 'audited' ? [{ id: 'decision', sourceDecisionId: 'decision',
    scanId, attemptNumber, rootRunId: 'root', materialDigest: 'digest', reviewerRunId: 'reviewer',
    reviewState: 'confirmed', title: `canonical:${scanId}:${attemptNumber}`, severity: 'high', path: 'app.py',
    line: 3, cwe: '', rationale: 'real review evidence', evidenceRefs: ['receipt:one'] }] : [] });
async function mount(read = async (scan, attempt) => report(scan, attempt)) {
  const calls = [], exports = [];
  const evidence = component('SourceReviewEvidence.vue', name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: {
      getNativeSourceFindings: (...args) => { calls.push(args); return read(...args); },
      exportNativeSourceFindings: async (...args) => { exports.push(args); return '/exports/scoped.json'; },
    } };
    throw new Error(`Unexpected import ${name}`);
  });
  const parent = component('results/SentinelSourceResults.vue', name => {
    if (name === 'vue') return vue;
    if (name === '../SourceReviewEvidence.vue') return { default: evidence };
    if (name === '../../presentation') return presentation;
    if (name === '../../../../i18n') return { useI18n: () => ({ tr: zh => zh }) };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    throw new Error(`Unexpected import ${name}`);
  });
  const selected = { id: 'A', taskName: 'A task', projectName: 'project', status: 'completed_with_gaps', scanType: 'code', attemptCount: 2 };
  const historical = { id: 5, title: 'historical-only-title', recordJson: '{}', recordKey: 'old-rule', severity: 'high' };
  const props = vue.reactive({ selected, detailBusy: false, scanControlBusy: '',
    scanAttempts: [1, 2, 2].map(attemptNumber => ({ scanId: 'A', attemptNumber })), visibleScanAttempts: [],
    attemptModeLabel: () => 'initial', showAttemptHistory: false, resultTab: 'overview', selectedFindingId: 5,
    isGreyboxScan: false, isCicdScan: false, appsecVulnerabilities: [], appsecSourceCounts: {}, cicdBlockingFindings: [],
    greyboxCorrelated: {}, focusedSourceFindingRows: [], sourceFindingRows: [historical], sourceDependencies: [],
    sourceFrameworks: [], sourceInventory: {}, sourceIssueGroups: [], sourceLanguageRows: [],
    sourceManifests: [], sourceSeverityCounts: {}, sourceStats: { findings: 1 }, appsecSourcesFor: () => [],
    authTypeLabel: value => value, ciProviderLabel: value => value, correlationParts: () => ({}), editValidation() {},
    effectiveSeverity: () => 'high', exportProject() {}, gateStatusLabel: value => value, pauseScan() {}, rescan() {},
    resumeScan() {}, sourceLocations: () => [], sourceTypeLabel: value => value, validationFor: () => undefined,
  });
  const root = node('root'), app = hostRenderer.createApp({ setup: () => () => vue.h(parent, props) });
  app.mount(root); await flush();
  return { props, calls, exports, root, text: () => text(root), find: predicate => find(root, predicate),
    select: async attempt => { find(root, n => n.props['aria-label'] === '源码结果审查轮次').props['onUpdate:modelValue'](attempt); await flush(); },
    unmount: () => app.unmount() };
}

test('source result page mounts real review, selects attempts, and exports exact selected scope', async (t) => {
  const h = await mount(); t.after(h.unmount);
  assert.deepEqual(h.calls, [['A', 2, 0]]);
  assert.ok(h.text().includes('canonical:A:2'));
  const history = h.find(n => n.type === 'details' && n.props.class === 'source-historical-results');
  assert.ok(text(history).includes('historical-only-title'));
  assert.ok(!text(history).includes('canonical:A:2'));
  assert.ok(!history.props.open, 'reference materials start collapsed');
  assert.equal(h.find(n => n.type === 'select').options.length, 2, 'duplicate attempt records do not duplicate options');
  await h.select(1);
  assert.deepEqual(h.calls, [['A', 2, 0], ['A', 1, 0]]);
  assert.ok(h.text().includes('canonical:A:1')); assert.ok(!h.text().includes('canonical:A:2'));
  await h.find(n => n.type === 'button' && text(n).includes('导出本轮审查 JSON')).props.onClick(); await flush();
  assert.deepEqual(h.exports, [['A', 1]]);
  assert.ok(h.text().includes('/exports/scoped.json'));
});

test('switching source tasks rejects stale attempt lists and late real child responses', async (t) => {
  const pending = deferred();
  const h = await mount(async (scan, attempt) => scan === 'A' ? pending.promise : report(scan, attempt));
  t.after(h.unmount);
  h.props.selected = { ...h.props.selected, id: 'B', attemptCount: 1 }; await flush();
  assert.ok(h.text().includes('尚无可选择的执行轮次'));
  assert.deepEqual(h.calls, [['A', 2, 0]], 'old A attempt list cannot trigger B reads');
  h.props.scanAttempts = [{ scanId: 'B', attemptNumber: 1 }, { scanId: 'A', attemptNumber: 2 },
    { scanId: 'B', attemptNumber: 0 }, { scanId: 'B', attemptNumber: 1.5 }]; await flush();
  pending.resolve(report('A', 2)); await flush();
  assert.deepEqual(h.calls, [['A', 2, 0], ['B', 1, 0]]);
  assert.ok(h.text().includes('canonical:B:1')); assert.ok(!h.text().includes('canonical:A:2'));
});

test('legacy references remain separate when real review is unavailable or fails', async (t) => {
  for (const read of [async (scan, attempt) => report(scan, attempt, 'unverified'), async () => { throw new Error('private'); }]) {
    const h = await mount(read); t.after(h.unmount);
    assert.ok(h.text().includes('historical-only-title'));
    assert.ok(h.text().includes('不代表所选轮次独立确认'));
    assert.ok(!h.text().includes('独立审查已确认'));
    assert.ok(!h.find(n => n.type === 'button' && text(n).includes('导出本轮审查 JSON')));
  }
});

const locValues = h => h.find(n => n.props.class === 'source-loc-grid').children
  .filter(n => n.type === 'article').map(n => text(find(n, child => child.type === 'strong')));
function inventory(h, value) {
  h.props.sourceInventoryFinding = { id: 8, kind: 'source_inventory', recordJson: JSON.stringify(value) };
  h.props.sourceInventory = value;
}

test('saved inventory without line statistics shows unknown instead of fabricated zero', async (t) => {
  const h = await mount(); t.after(h.unmount);
  inventory(h, { architecture: 'Historical repo' }); await flush();
  assert.deepEqual(locValues(h), ['未记录', '未记录', '未记录', '未记录']);
  assert.ok(h.text().includes('查看历史记录不会重新读取当前源码目录'));
  assert.deepEqual(h.calls, [['A', 2, 0]], 'displaying legacy inventory must not initiate new review or execution');
});

test('saved line statistics preserve genuine zero and update when inventory changes', async (t) => {
  const h = await mount(); t.after(h.unmount);
  inventory(h, { lineStats: { physical: 8, code: 8, comments: 0, blank: 0, skippedLargeFiles: 2 } }); await flush();
  assert.deepEqual(locValues(h), ['8', '8', '0', '0']);
  assert.match(text(h.find(n => n.props.class === 'source-loc-rule')), /2.*个超大源码文件/s);
  inventory(h, {}); await flush();
  assert.deepEqual(locValues(h), ['未记录', '未记录', '未记录', '未记录']);
  assert.ok(!text(h.find(n => n.props.class === 'source-loc-rule')).includes('2 个'));
  inventory(h, { lineStats: { physical: 0, code: 0, comments: 0, blank: 0 } }); await flush();
  assert.deepEqual(locValues(h), ['0', '0', '0', '0']);
});

test('malformed saved counts and missing per-language counts remain unknown', async (t) => {
  const h = await mount(); t.after(h.unmount);
  for (const value of [-1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, '42', true, null, undefined, {}, []]) {
    inventory(h, { lineStats: { physical: value, code: value, comments: value, blank: value, skippedLargeFiles: value } });
    h.props.sourceLanguageRows = [{ name: 'Rust', files: 1, bytes: 10, percent: 100,
      codeLines: value, commentLines: value, blankLines: value, lines: value }];
    await flush();
    assert.deepEqual(locValues(h), ['未记录', '未记录', '未记录', '未记录'], String(value));
    const language = h.find(n => n.props.class === 'source-language-table');
    assert.equal((text(language).match(/未记录/g) || []).length, 4, `language counts: ${String(value)}`);
  }
});
