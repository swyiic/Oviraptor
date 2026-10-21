const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const { loadPure, transpile, catalog } = require('./shell_module_harness.cjs');
const selection = loadPure('src/features/configuration/profileSelection.ts');
const filename = 'src/features/hackerone/components/HackerOneBoard.vue';
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'hackerone-accounts' });
const template = compileTemplate({ filename, id: 'hackerone-accounts', source: descriptor.template.content, compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const rendered = {};
new Function('require', 'exports', transpile(template.code))(require, rendered);
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); await vue.nextTick(); };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const profile = (id, isDefault = false, configured = true) => ({ id, name: `Profile ${id}`, isDefault, description: '', createdAt: '', updatedAt: '', settings: configured ? { hackerOneUsername: `fixture-private-user-${id}`, hackerOneToken: `fixture-private-token-${id}` } : {} });
const program = () => ({ id: 'p', handle: 'fixture-program', name: 'Fixture program', scopeCount: 0, iconUrl: '', programState: '', submissionState: '', bookmarked: false });

function harness(t, profiles, overrides = {}) {
  const calls = [], events = [], disposers = [];
  const methods = {
    listHackerOnePrograms: async () => [program()], listHackerOneEvents: async () => [],
    getHackerOneDetail: async () => ({ program: program(), scopes: [], exclusions: [] }),
    syncHackerOne: async () => 'fixture synchronized', setHackerOneBookmark: async () => {},
    addHackerOneScopesToProject: async () => 0, ...overrides,
  };
  const api = Object.fromEntries(Object.entries(methods).map(([name, fn]) => [name, (...args) => { calls.push([name, ...args]); return fn(...args); }]));
  const exports = {};
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return { ...vue, onMounted: () => {}, onUnmounted: fn => disposers.push(fn) };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => vue.h('svg') });
    if (name === '@tauri-apps/plugin-opener') return { openUrl: async () => { throw new Error('External URLs must not open in this fixture'); } };
    if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: (_zh, en) => en }) };
    if (name === '../api') return { hackerOneApi: api };
    if (name === '../../configuration/profileSelection') return selection;
    throw new Error(`Unexpected HackerOne dependency: ${name}`);
  }, exports);
  const props = vue.reactive({ profiles, projects: [{ id: 91, name: 'Fixture workspace', status: 'active' }] });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.default.setup(props, { expose() {}, emit: (...event) => events.push(event) }));
  t.after(() => { for (const dispose of disposers) dispose(); scope.stop(); });
  return { state, props, calls, events, render: () => renderToString(vue.createSSRApp({ render: () => rendered.render({}, [], props, vue.proxyRefs(state), {}, {}) })) };
}

test('default profile fallback matches Rust ordering independently of update/list order', () => {
  const profiles = [profile(9), profile(3), profile(7)];
  assert.equal(selection.defaultConfigProfile(profiles).id, 3);
  profiles[2].isDefault = true;
  assert.equal(selection.defaultConfigProfile(profiles).id, 7);
  profiles[0].isDefault = true;
  assert.equal(selection.defaultConfigProfile(profiles).id, 7);
  assert.deepEqual(profiles.map(item => item.id), [9, 3, 7]);
});

test('paused HackerOne route is absent even with credentials and a retained enabled preference', () => {
  for (const configured of [false, true]) {
    assert.ok(!catalog.visibleModules(configured, { asset: true, sentinel: true, hackerone: true }).some(item => item.id === 'hackerone'));
  }
});

// Historical component checks are retained for a future explicit reactivation.
// They do not represent enabled product functionality while HackerOne is paused.
const archivedTest = (name, run) => test.skip(`archived HackerOne: ${name} (feature suspended)`, run);

archivedTest('a configured non-default account is usable when the default has no credentials', async t => {
  const { state, calls } = harness(t, [profile(1, true, false), profile(3)]);
  assert.equal(state.selectedProfileId.value, 3);
  assert.equal(state.accountConfigured.value, true);
  await state.sync();
  assert.deepEqual(calls.find(call => call[0] === 'syncHackerOne'), ['syncHackerOne', 3, undefined]);
});

archivedTest('explicit account drives both program sync and detail refresh without changing defaults', async t => {
  const { state, props, calls } = harness(t, [profile(1, true), profile(3)]);
  state.selectedProfileId.value = 3; await flush();
  await state.sync();
  await state.show(program());
  assert.equal(calls.filter(call => call[0] === 'syncHackerOne').length, 1);
  await state.sync('fixture-program');
  assert.deepEqual(calls.filter(call => call[0] === 'syncHackerOne'), [['syncHackerOne', 3, undefined], ['syncHackerOne', 3, 'fixture-program']]);
  assert.equal(props.profiles[0].isDefault, true);
  assert.equal(props.profiles[1].isDefault, false);
});

archivedTest('missing credentials retain cached browsing and emit a configuration destination, without sync', async t => {
  const { state, calls, events, render } = harness(t, [profile(4, false, false)]);
  await state.load(); await state.show(program()); await state.sync(); state.configure();
  assert.equal(state.detail.value.program.handle, 'fixture-program');
  assert.equal(calls.filter(call => call[0] === 'syncHackerOne').length, 0);
  assert.deepEqual(events.at(-1), ['configure', 4]);
  const html = await render();
  assert.match(html, /Configure HackerOne account/);
  assert.match(html, /Cached records remain available/);
});

archivedTest('only safe profile metadata and availability are exposed to rendered account controls', async t => {
  const { state, render } = harness(t, [profile(1, true), profile(3)]);
  const html = await render();
  assert.match(html, /Profile 1/);
  assert.match(html, /Configured/);
  for (const token of ['fixture-private-user-1', 'fixture-private-user-3', 'fixture-private-token-1', 'fixture-private-token-3']) assert.ok(!html.includes(token));
  assert.deepEqual(Object.keys(state.accountOptions.value[0]).sort(), ['configured', 'id', 'isDefault', 'name']);
});

archivedTest('in-flight synchronization captures one account and drops stale UI completion after selection changes', async t => {
  const pending = deferred();
  const { state, calls, events } = harness(t, [profile(1, true), profile(3)], { syncHackerOne: () => pending.promise });
  const request = state.sync(); await state.sync();
  state.selectedProfileId.value = 3; await flush(); pending.resolve('done'); await request;
  assert.deepEqual(calls, [['syncHackerOne', 1, undefined]]);
  assert.deepEqual(events, []);
  assert.equal(state.selectedProfileId.value, 3);
  assert.equal(state.busy.value, false);
});

archivedTest('credential revocation during synchronization invalidates success without another network call', async t => {
  const pending = deferred();
  const { state, props, calls, events } = harness(t, [profile(1, true)], { syncHackerOne: () => pending.promise });
  const request = state.sync(); props.profiles[0].settings.hackerOneToken = ''; await flush();
  pending.resolve('done'); await request;
  assert.equal(state.accountConfigured.value, false);
  assert.deepEqual(calls, [['syncHackerOne', 1, undefined]]);
  assert.deepEqual(events, []);
});

archivedTest('choosing an unconfigured profile remains explicit and never falls back to another account for sync', async t => {
  const { state, calls, events } = harness(t, [profile(1, true), profile(3, false, false)]);
  state.selectedProfileId.value = 3; await flush(); await state.sync(); state.configure();
  assert.equal(state.selectedProfileId.value, 3);
  assert.equal(calls.filter(call => call[0] === 'syncHackerOne').length, 0);
  assert.deepEqual(events.at(-1), ['configure', 3]);
});
