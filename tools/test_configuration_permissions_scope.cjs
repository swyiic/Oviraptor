const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { transpile } = require('./shell_module_harness.cjs');
const filename = 'src/features/configuration/sections/ConfigDataPermissionsSection.vue';
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'permission-scope' });
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

function harness(t) {
  const bindings = new Map(), writes = [], notices = [], disposers = [];
  let grantResult = async () => ({});
  const app = { id: 'fixture.app', enabled: true, manifest: { permissions: [{ id: 'inventory.read' }, { id: 'inventory.write' }, { id: 'workspace.run' }] }, grants: [{ projectId: 101, permissions: ['inventory.read'], revision: 1 }, { projectId: 202, permissions: [], revision: 2 }] };
  const client = {
    settings: async () => ({ runtime: { id: 'xai-api', identity: 'xai-api:fixture' } }),
    listApps: async () => ({ items: [app] }),
    ensureProject: (_name, id) => { const pending = deferred(); bindings.set(id, pending); return pending.promise; },
    setGrants: async (appId, input) => { writes.push({ appId, input: structuredClone(input) }); return grantResult(); },
  };
  const exports = {};
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return { ...vue, onMounted: () => {}, onUnmounted: fn => disposers.push(fn) };
    if (name === '@lucide/vue') return {};
    if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: (_zh, en) => en }) };
    if (name.endsWith('/agentHost/api')) return { resolveAgentHostEndpoint: async () => ({ origin: 'http://fixture.test', session: 'fixture' }) };
    if (name.endsWith('/agentHost/chatClient')) return { createAgentHostChatClient: () => client };
    throw new Error(`Unexpected permission import: ${name}`);
  }, exports);
  const props = vue.reactive({ projects: [{ id: 1, name: 'A', status: 'active' }, { id: 2, name: 'B', status: 'active' }], selectedProjectId: 1 });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.default.setup(props, { expose() {}, emit: (...event) => notices.push(event) }));
  const dispose = () => { for (const fn of disposers) fn(); scope.stop(); };
  t.after(dispose);
  return { state, props, bindings, writes, notices, app, dispose, setGrantResult: fn => { grantResult = fn; } };
}

test('late binding cannot replace the selected project or allow cross-project grants', async t => {
  const { state, props, bindings, writes, app } = harness(t);
  const initial = state.load(); await flush();
  props.selectedProjectId = 2; await flush();
  assert.equal(state.hostProjectId.value, undefined);
  assert.equal(state.canSave.value, false);
  state.disclosureAccepted.value = true;
  await state.saveApp(app);
  assert.equal(writes.length, 0);
  bindings.get(2).resolve(202); await flush();
  assert.equal(state.hostProjectId.value, 202);
  bindings.get(1).resolve(101); await initial; await flush();
  assert.equal(state.desktopProjectId.value, 2);
  assert.equal(state.hostProjectId.value, 202);
  state.togglePermission(app.id, 'inventory.write', true);
  state.disclosureAccepted.value = true;
  const save = state.saveApp(app); await flush();
  assert.deepEqual(writes.map(item => item.input.projectId), [202]);
  assert.deepEqual(writes[0].input.permissions, ['inventory.write']);
  bindings.get(2).resolve(202); await save;
});

test('switching project immediately clears old grants, disclosure and binding', async t => {
  const { state, props, bindings, app, writes } = harness(t);
  const initial = state.load(); await flush(); bindings.get(1).resolve(101); await initial;
  state.disclosureAccepted.value = true;
  props.selectedProjectId = 2;
  assert.equal(state.hostProjectId.value, undefined);
  assert.deepEqual(state.selected.value, {});
  assert.equal(state.disclosureAccepted.value, false);
  assert.equal(state.canSave.value, false);
  await state.saveApp(app);
  assert.equal(writes.length, 0);
  bindings.get(2).resolve(202); await flush();
  props.projects[1].status = 'archived';
  assert.equal(state.hostProjectId.value, undefined);
  assert.equal(state.canSave.value, false);
});

test('an in-flight save retains its captured project and cannot overwrite a later selection', async t => {
  const { state, props, bindings, writes, notices, app, setGrantResult } = harness(t);
  const initial = state.load(); await flush(); bindings.get(1).resolve(101); await initial;
  const pending = deferred(); setGrantResult(() => pending.promise);
  state.togglePermission(app.id, 'workspace.run', true); state.disclosureAccepted.value = true;
  const save = state.saveApp(app);
  props.selectedProjectId = 2; await flush(); bindings.get(2).resolve(202); await flush();
  pending.resolve({}); await save;
  assert.equal(writes[0].input.projectId, 101);
  assert.equal(state.hostProjectId.value, 202);
  assert.equal(state.disclosureAccepted.value, false);
  assert.deepEqual(notices, []);
});

test('failed or disposed binding never enables grant writes', async t => {
  const { state, bindings, dispose } = harness(t);
  const initial = state.load(); await flush(); bindings.get(1).reject(Error('fixture failed')); await initial;
  assert.equal(state.canSave.value, false);
  assert.equal(state.hostProjectId.value, undefined);
  const retry = state.bindProject({ id: 1, name: 'A' });
  dispose(); bindings.get(1).resolve(101); await retry;
  assert.equal(state.hostProjectId.value, undefined);
});

test('manifest write and execution permissions have accurate disclosure and template compiles', async t => {
  const { state } = harness(t);
  assert.equal(state.permissionEffect('inventory.read'), 'Read');
  assert.equal(state.permissionEffect('inventory.write'), 'Write / modify');
  assert.equal(state.permissionEffect('workspace.run'), 'Execute');
  const compiled = compileTemplate({ filename, id: 'permission-scope', source: descriptor.template.content, compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(compiled.errors, []);
  assert.doesNotMatch(descriptor.template.content, /read-only workspace data|不会自动授予写权限/);
});
