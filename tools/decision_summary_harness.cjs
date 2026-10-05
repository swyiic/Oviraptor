// Real trace SFC/selection and Board trace composable; substitute IPC/host only.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const { mountTrace, trace, deferred, flush } = require('./agent_trace_harness.cjs');
const root = path.join(__dirname, '../src');
const transpile = code => ts.transpileModule(code, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
function loader() {
  const cache = new Map();
  function load(file) {
    if (cache.has(file)) return cache.get(file);
    const exports = {}; cache.set(file, exports);
    const source = fs.readFileSync(file, 'utf8');
    const local = name => {
      if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
      if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: zh => zh }) };
      if (!name.startsWith('.')) return require(name);
      const next = path.resolve(path.dirname(file), name);
      return load(path.extname(next) ? next : `${next}.ts`);
    };
    if (file.endsWith('.vue')) {
      const { descriptor } = parse(source, { filename: file });
      const script = compileScript(descriptor, { id: file });
      const template = compileTemplate({ source: descriptor.template.content,
        filename: file, id: file, compilerOptions: { bindingMetadata: script.bindings } });
      assert.deepEqual(template.errors, []);
      new Function('require', 'exports', transpile(script.content))(local, exports);
      const rendered = {};
      new Function('require', 'exports', transpile(template.code))(local, rendered);
      exports.default.render = rendered.render;
    } else new Function('require', 'exports', transpile(source))(local, exports);
    return exports;
  }
  return load;
}
const renderTimeline = props => renderToString(vue.createSSRApp(loader()(path.join(root,
  'features/sentinel/components/results/SentinelTraceTimeline.vue')).default, props));
function round(overrides = {}, payloadOverrides = {}) {
  const payload = {
    rootTickVersion: 1, rootControl: 'control-original', callId: 'call-original', turns: 2,
    requestHash: 'a'.repeat(64), basisHash: 'b'.repeat(64), responseHash: 'c'.repeat(64),
    usage: { inputTokens: 80, cachedInputTokens: 20, outputTokens: 20,
      totalTokens: 100, modelRequests: 1 }, modelRequests: 1, totalTokens: 100,
    toolCalls: [], advisoryOnly: true,
    decisionSummary: { schemaVersion: 1, observed: ['映射回执已经保存'],
      missing: ['身份B尚缺证据'], suggestions: ['dispatch:web_executor'],
      costNotes: ['下轮费用需再核算'], risks: ['尚未独立验证'] },
    ...payloadOverrides,
  };
  const detail = JSON.stringify(payload);
  return { id: 'native:root-original:7', sessionId: 'root-original', callId: '',
    targetUrl: 'https://authorized.invalid/path', eventType: 'model_round_completed',
    role: 'coordinator', name: '', status: 'recorded', detail,
    detailSize: detail.length, detailTruncated: false, createdAt: '2026-10-02T01:00:00Z',
    ...overrides };
}
function snapshot(id, event = round()) { const value = trace(id); value.events = [event]; return value; }
function mountTask(read) {
  const { useTaskTrace } = loader()(path.join(root, 'features/sentinel/traces/useTaskTrace.ts'));
  const scan = vue.ref('scan-a'), project = vue.ref(1), notices = [];
  let b;
  const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}),
    createComment: () => ({}), insert() {}, remove() {}, setText() {}, setElementText() {},
    patchProp() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({ setup() {
    b = useTaskTrace({ scanId: () => scan.value, projectId: () => project.value,
      read, notify: value => notices.push(value) }); return () => null;
  } }); app.mount({});
  return { b, scan, project, notices, unmount: () => app.unmount() };
}
module.exports = { mountTrace, trace, round, snapshot, deferred, flush, renderTimeline, mountTask };
