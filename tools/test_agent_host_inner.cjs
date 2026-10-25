const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { loadPure, transpile } = require('./shell_module_harness.cjs');

// Exercise the production component, its actual event handlers and its compiled
// template. These are isolated UI fixtures: no Native, model or target calls.
const filename = 'src/features/agentHost/components/AgentHostChat.vue';
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'compact-chat-activity' });
const template = compileTemplate({ source: descriptor.template.content, filename,
  id: 'compact-chat-activity', compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const rendered = {};
// VNodes are also inspected outside a mounted DOM. Refresh feedback is unrelated
// to activity disclosure; omit directive attachment in that inspection path.
new Function('require', 'exports', transpile(template.code))(name => name === 'vue'
  ? { ...vue, withDirectives: node => node } : require(name), rendered);
const flush = async () => { for (let i = 0; i < 4; i++) await new Promise(setImmediate); };
const assistant = (id = 'answer-1', runId = 'run-1', status = 'running') => ({ id, runId, role: 'assistant',
  status, content: `Actual response ${id}`, createdAt: '2026-10-25T08:00:00Z' });
const innerEvent = (extra = {}) => ({ type: 'runtime.inner', conversationId: 'conversation-1',
  messageId: 'answer-1', runId: 'run-1', phase: 'model', goal: 'Read the authorized project',
  facts: ['Saved project evidence'], hypotheses: [], ...extra });

async function mount(t, rows = [assistant()], overrides = {}) {
  let listener;
  const cleanups = [], exports = {};
  const client = {
    ensureProject: async (_name, id) => id,
    conversations: async projectId => [{ id: `conversation-${projectId}`, projectId, title: 'UI fixture' }],
    messages: async () => rows, status: async () => ({ permissions: [], authorizations: [] }),
    events: callback => { listener = callback; return () => {}; },
    ...overrides,
  };
  const stub = { inheritAttrs: false, render: () => null };
  const scopedVue = { ...vue, onBeforeUnmount: callback => cleanups.push(callback) };
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return scopedVue;
    if (name === '@tauri-apps/api/core') return { invoke: async () => { throw Error('Native access forbidden in activity test'); } };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => stub });
    if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: zh => zh }) };
    if (name.endsWith('/shared/directives/refreshFeedback')) return loadPure('src/shared/directives/refreshFeedback.ts');
    if (name === '../useConversationFiles') {
      const files = {};
      new Function('require', 'exports', transpile(fs.readFileSync('src/features/agentHost/useConversationFiles.ts', 'utf8')))(dependency => {
        if (dependency === 'vue') return scopedVue;
        if (dependency === '@tauri-apps/api/core') return { invoke: async () => { throw Error('Native access forbidden'); } };
        throw Error(`Unexpected file dependency: ${dependency}`);
      }, files);
      return files;
    }
    if (name.endsWith('ConversationMessage.vue')) return { default: { props: ['message'], render() {
      return vue.h('div', { 'data-testid': 'actual-answer' }, this.message.content);
    } } };
    if (name.endsWith('.vue')) return { default: stub };
    if (name.startsWith('../')) return loadPure(`src/features/agentHost/${name.slice(3)}.ts`);
    throw Error(`Unexpected activity dependency: ${name}`);
  }, exports);
  const props = vue.reactive({ client, settings: { runtime: { id: 'xai-api' } }, projectId: 1, projectName: 'Fixture' });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.default.setup(props, { expose() {}, emit() {} }));
  const tree = () => rendered.render({}, [], props, vue.proxyRefs(state), {}, {});
  const html = () => renderToString(vue.createSSRApp({ render: tree }));
  t.after(() => { cleanups.forEach(callback => callback()); scope.stop(); });
  await flush();
  return { state, props, tree, html, event: event => listener(event), setRows(value) { rows = value; } };
}
function findNodes(tree, predicate, ancestors = []) {
  if (!tree || typeof tree !== 'object') return [];
  const found = predicate(tree) ? [{ node: tree, ancestors }] : [];
  for (const child of Array.isArray(tree.children) ? tree.children : []) found.push(...findNodes(child, predicate, [...ancestors, tree]));
  return found;
}
const panels = h => findNodes(h.tree(), node => node.props?.['data-testid'] === 'agent-host-inner');
const closed = node => !node.props?.open;
function nodeText(node) {
  if (typeof node === 'string') return node;
  if (!node || typeof node !== 'object') return '';
  return typeof node.children === 'string' ? node.children
    : (Array.isArray(node.children) ? node.children : []).map(nodeText).join('');
}
const toggle = (node, open) => {
  assert.equal(typeof node.props?.onToggle, 'function', 'A user toggle must have a controlled, per-reply state');
  node.props.onToggle({ target: { open }, currentTarget: { open } });
};

test('activity starts as one collapsed disclosure below the actual assistant response inside the transcript', async t => {
  const h = await mount(t);
  h.event(innerEvent());
  const [panel] = panels(h);
  assert.ok(panel, 'The current reply has an activity disclosure');
  assert.equal(panel.node.type, 'details');
  assert.ok(closed(panel.node), 'Running does not automatically expand the process');
  assert.ok(panel.ancestors.some(node => node.type === 'article' && node.props?.class?.includes('assistant')));
  assert.ok(panel.ancestors.some(node => node.props?.class?.includes('agent-host-chat-transcript')));
  const html = await h.html();
  assert.ok(html.indexOf('Actual response answer-1') < html.indexOf('data-testid="agent-host-inner"'));
});

test('streaming updates preserve explicit expansion and an explicit collapse is not reopened', async t => {
  const h = await mount(t);
  h.event(innerEvent());
  toggle(panels(h)[0].node, true);
  h.event(innerEvent({ phase: 'tool', goal: 'Next authorized read' }));
  h.event({ type: 'runtime.step', conversationId: 'conversation-1', messageId: 'answer-1', runId: 'run-1', title: 'Read inventory' });
  assert.equal(panels(h)[0].node.props.open, true);
  toggle(panels(h)[0].node, false);
  h.event(innerEvent({ phase: 'hold', goal: 'Compare saved results' }));
  assert.ok(closed(panels(h)[0].node));
});

test('sending a new turn starts with collapsed activity instead of inheriting the previous reply expansion', async t => {
  const h = await mount(t);
  h.event(innerEvent());
  toggle(panels(h)[0].node, true);
  h.setRows([assistant('answer-1', 'run-1', 'completed')]);
  h.event({ type: 'message.completed', conversationId: 'conversation-1', messageId: 'answer-1', runId: 'run-1', status: 'completed' });
  await flush();
  assert.equal(h.state.running.value, false);
  h.props.client.send = async () => { h.setRows([assistant('answer-1', 'run-1', 'completed'), assistant('answer-2', 'run-2')]); };
  h.state.draft.value = 'Read another saved project record';
  await h.state.send();
  h.event(innerEvent({ messageId: 'answer-2', runId: 'run-2', goal: 'New authorized read' }));
  const [panel] = panels(h);
  assert.ok(panel);
  assert.ok(closed(panel.node));
  assert.ok(panel.ancestors.some(node => node.type === 'article' && node.key === 'answer-2'));
});

test('a finished response says completed without automatically expanding the process', async t => {
  const h = await mount(t);
  h.event(innerEvent());
  h.setRows([assistant('answer-1', 'run-1', 'completed')]);
  h.event({ type: 'message.completed', conversationId: 'conversation-1', messageId: 'answer-1', runId: 'run-1', status: 'completed' });
  await flush();
  assert.ok(closed(panels(h)[0].node));
  assert.match(await h.html(), /已完成/);
});

test('a completed event from an older run cannot stop the current reply or overwrite its activity', async t => {
  const h = await mount(t, [assistant('old-answer', 'old-run', 'completed'), assistant()]);
  h.event(innerEvent());
  h.event(innerEvent({ messageId: 'old-answer', runId: 'old-run', goal: 'Stale earlier request' }));
  h.event({ type: 'message.completed', conversationId: 'conversation-1', messageId: 'old-answer', runId: 'old-run', status: 'completed' });
  await flush();
  assert.equal(h.state.running.value, true);
  const html = await h.html();
  assert.match(html, /Read the authorized project/);
  const current = panels(h).find(panel => panel.ancestors.some(node => node.type === 'article' && node.key === 'answer-1'));
  assert.ok(current);
  assert.ok(closed(current.node));
});

test('switching conversations clears the old process and ignores late activity for the previous chat', async t => {
  const h = await mount(t);
  h.event(innerEvent());
  toggle(panels(h)[0].node, true);
  h.setRows([]); h.state.selectedId.value = 'conversation-2';
  await flush();
  h.event(innerEvent({ goal: 'Late private previous-chat evidence' }));
  assert.equal(panels(h).length, 0);
  assert.doesNotMatch(await h.html(), /Late private previous-chat evidence|Saved project evidence/);
  h.setRows([assistant('answer-2', 'run-2')]); await h.state.loadMessages();
  h.event(innerEvent({ conversationId: 'conversation-2', messageId: 'answer-2', runId: 'run-2', goal: 'Current chat read' }));
  assert.ok(closed(panels(h)[0].node), 'Expansion does not carry to a different chat');
});

for (const status of ['failed', 'cancelled']) test(`${status} process does not claim successful completion`, async t => {
  const h = await mount(t);
  h.event(innerEvent());
  h.setRows([assistant('answer-1', 'run-1', status)]);
  h.event({ type: 'message.completed', conversationId: 'conversation-1', messageId: 'answer-1', runId: 'run-1', status });
  await flush();
  const panel = panels(h)[0].node;
  const phase = findNodes(panel, node => node.props?.class?.includes('ahc-inner-phase'))[0]?.node;
  assert.ok(phase, 'The process reports its actual terminal disposition');
  assert.doesNotMatch(nodeText(phase), /已完成|思考中/);
  assert.match(nodeText(phase), status === 'failed' ? /失败|出错|未完成/ : /已停止|已取消|中断/);
  assert.ok(closed(panel));
});

const savedEvidence = (runId, toolName, summary) => ({ id: `evidence-${runId}`, runId, toolName,
  summary, status: 'completed', provenance: 'host-mcp-tool-result',
  startedAt: '2026-10-25T08:00:00Z', finishedAt: '2026-10-25T08:00:01Z' });
const savedTask = evidence => ({ id: 'saved-task', conversationId: 'conversation-1', projectId: 1,
  revision: 2, goal: 'Read saved project records', status: 'completed', steps: [], evidence, artifacts: [],
  summary: '', createdAt: '2026-10-25T08:00:00Z', updatedAt: '2026-10-25T08:00:01Z' });

test('a reconnect with no runtime events restores collapsed completed activity from the exact saved run evidence', async t => {
  const currentEvidence = savedEvidence('run-1', 'inventory_stats', 'Read the saved asset count');
  const otherEvidence = savedEvidence('old-run', 'scan_list', 'Older unrelated tool result');
  const h = await mount(t, [assistant('old-answer', 'old-run', 'completed'), assistant('answer-1', 'run-1', 'completed')],
    { task: async () => savedTask([otherEvidence, currentEvidence]) });
  // Deliberately do not emit runtime.inner, runtime.step or runtime.heartbeat.
  assert.equal(h.state.inner.value, null);
  assert.deepEqual(h.state.steps.value, []);
  const [panel] = panels(h);
  assert.equal(panels(h).length, 1);
  assert.ok(closed(panel.node));
  assert.ok(panel.ancestors.some(node => node.type === 'article' && node.key === 'answer-1'));
  const summary = findNodes(panel.node, node => node.type === 'summary')[0].node;
  assert.match(nodeText(summary), /已完成/);
  assert.match(nodeText(summary), /1 项操作/);
  const body = findNodes(panel.node, node => node.props?.class?.includes('ahc-inner-body'))[0].node;
  assert.match(nodeText(body), /inventory_stats/);
  assert.match(nodeText(body), /Read the saved asset count/);
  assert.doesNotMatch(nodeText(body), /scan_list|Older unrelated tool result/);
  const html = await h.html();
  assert.ok(html.indexOf('Actual response answer-1') < html.indexOf('data-testid="agent-host-inner"'));
});

test('saved evidence from another run cannot appear as the latest assistant reply process', async t => {
  const h = await mount(t, [assistant('old-answer', 'old-run', 'completed'), assistant('answer-1', 'run-1', 'completed')],
    { task: async () => savedTask([savedEvidence('old-run', 'scan_list', 'Only an earlier run performed this tool')]) });
  assert.equal(panels(h).length, 0, 'The latest reply cannot borrow another run\'s tool evidence');
});
