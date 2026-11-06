import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';
// Actual SFC + real Vue reactivity, mocked IPC only. No app, DB or network.
// Run: node --test tools/test_fingerprint_pack_ui.mjs
const require = createRequire(new URL('../package.json', import.meta.url));
const Vue = require('vue');
const ts = require('typescript');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const filename = fileURLToPath(new URL('../src/features/configuration/sections/ConfigFingerprintLibrarySection.vue', import.meta.url));
const { descriptor, errors } = parse(readFileSync(filename, 'utf8'), { filename });
assert.deepEqual(errors, []);
const script = compileScript(descriptor, { id: 'fingerprint-boolean-test' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'fingerprint-boolean-test', compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const compiled = ts.transpileModule(script.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
const rule = (id, name = id, extras = {}) => ({ id, name, bodyRegex: [name], ...extras });
const treeRule = (id = 'boolean-app') => ({ id, name: 'Condition App', condition: {
  type: 'all', children: [
    { type: 'body', pattern: 'required-brand' },
    { type: 'any', children: [{ type: 'header', name: 'server', pattern: 'specific-server' }, { type: 'body', pattern: 'alternative-marker' }] },
  ],
} });
const ids = entries => Array.from(entries, entry => entry.fingerprint.id);
function setup(handlers = {}) {
  const calls = [];
  const mounted = [];
  const exports = {};
  const library = {
    fingerprints: { fingerprints: [rule('builtin', 'Builtin')] },
    user: { path: '/synthetic/fingerprints.user.json', booleanPath: '/synthetic/fingerprints.boolean.json', fingerprints: [], error: '' },
    exposures: { checks: [{ id: 'separate-check', title: 'Exposure', severity: 'info', path: '/separate' }] },
    booleanRuntime: 'condition-tree-v1',
  };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (handlers[command]) return handlers[command](args, { library, calls });
    if (command === 'scan_rule_library') return library;
    if (command === 'scan_fingerprint_template') return rule('template-product', 'Template Product');
    if (command === 'scan_export_user_fingerprints') return JSON.stringify({ schemaVersion: 1, fingerprints: library.user.fingerprints });
    throw new Error(`Unexpected IPC: ${command}`);
  };
  vm.runInNewContext(compiled, { exports, require(name) {
    if (name === 'vue') return { ...Vue, defineComponent: component => component, onMounted: callback => mounted.push(callback) };
    if (name === '@tauri-apps/api/core') return { invoke };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: chinese => chinese }) };
    throw new Error(`Unexpected module: ${name}`);
  } });
  const state = exports.default.setup({}, { expose() {} });
  return { state, library, calls, mount: async () => { await mounted[0](); await Vue.nextTick(); } };
}

test('initial counts exclude active exposures and use exact ID dedup with custom override', async () => {
  const { state, library, mount } = setup();
  library.fingerprints.fingerprints = [rule('same', 'Old'), rule('builtin-only'), rule('builtin-only')];
  library.user.fingerprints = [rule('same', 'Custom replacement'), rule('custom-only'), rule('custom-only')];
  await mount();
  assert.equal(state.effectiveCount.value, 3);
  assert.equal(state.builtinCount.value, 1);
  assert.equal(state.customCount.value, 2);
  assert.equal(state.builtinCount.value + state.customCount.value, state.effectiveCount.value);
  const same = state.catalogEntries.value.find(entry => entry.fingerprint.id === 'same');
  assert.equal(same.fingerprint.name, 'Custom replacement');
  assert.equal(same.source, 'custom');
});

test('same names and case-distinct IDs remain separate products', async () => {
  const { state, library, mount } = setup();
  library.fingerprints.fingerprints = [rule('Product', 'Same name'), rule('product', 'Same name')];
  await mount();
  assert.deepEqual(ids(state.catalogEntries.value).sort(), ['Product', 'product']);
  assert.equal(state.effectiveCount.value, 2);
});

test('authoritative effective catalog wins and explicit empty catalog never falls back', async () => {
  const { state, library, mount } = setup();
  library.effective = { fingerprints: [rule('effective'), rule('effective')], error: '' };
  await mount();
  assert.deepEqual(ids(state.catalogEntries.value), ['effective']);
  library.effective.fingerprints = [];
  await state.reloadLibrary();
  assert.equal(state.effectiveCount.value, 0);
  assert.equal(state.pagedEntries.value.length, 0);
});

test('search covers Chinese names case width ID and both legacy and nested features locally', async () => {
  const { state, library, calls, mount } = setup();
  library.fingerprints.fingerprints = [rule('wordpress-core', 'WordPress', { bodyRegex: ['wp-content/'] }), rule('ruoyi', 'RuoYi 若依'), treeRule()];
  await mount();
  for (const [query, expected] of [
    ['  WoRdPrEsS  ', ['wordpress-core']], ['ＷｏｒｄＰｒｅｓｓ', ['wordpress-core']], ['若依', ['ruoyi']],
    ['wordpress-core', ['wordpress-core']], ['wp-content/', ['wordpress-core']], ['specific-server', ['boolean-app']], ['required-brand', ['boolean-app']],
    ['no-such-framework', []],
  ]) {
    state.query.value = query; await Vue.nextTick();
    assert.deepEqual(ids(state.filteredEntries.value), expected);
    assert.equal(state.effectiveCount.value, 3);
  }
  state.clearSearch(); await Vue.nextTick();
  assert.equal(state.query.value, '');
  assert.equal(state.filteredEntries.value.length, 3);
  assert.equal(calls.length, 1, 'search does not reload data or invoke a probe');
});

test('source filter partitions effective rules while search leaves global count unchanged', async () => {
  const { state, library, mount } = setup();
  library.fingerprints.fingerprints = [rule('shared', 'Builtin shared'), rule('builtin-only')];
  library.user.fingerprints = [rule('shared', 'Custom shared'), treeRule()];
  await mount();
  state.sourceFilter.value = 'custom'; await Vue.nextTick();
  assert.deepEqual(ids(state.filteredEntries.value).sort(), ['boolean-app', 'shared']);
  state.sourceFilter.value = 'builtin'; await Vue.nextTick();
  assert.deepEqual(ids(state.filteredEntries.value), ['builtin-only']);
  state.query.value = 'required-brand'; await Vue.nextTick();
  assert.equal(state.filteredEntries.value.length, 0);
  assert.equal(state.effectiveCount.value, 3);
});

test('pagination is bounded and resets/clamps after search filter and reload', async () => {
  const { state, library, mount } = setup();
  library.effective = { fingerprints: Array.from({ length: 61 }, (_, index) => rule(`framework-${index}`, `Framework ${index}`)), error: '' };
  await mount();
  assert.equal(state.pageSize, 25);
  assert.equal(state.totalPages.value, 3);
  assert.equal(state.pagedEntries.value.length, 25);
  state.nextPage(); state.nextPage(); await Vue.nextTick();
  assert.equal(state.currentPage.value, 3);
  assert.equal(state.pagedEntries.value.length, 11);
  state.nextPage(); assert.equal(state.currentPage.value, 3);
  state.query.value = 'Framework 60'; await Vue.nextTick();
  assert.equal(state.page.value, 1);
  assert.deepEqual(ids(state.pagedEntries.value), ['framework-60']);
  state.clearSearch(); await Vue.nextTick(); state.nextPage();
  state.sourceFilter.value = 'builtin'; await Vue.nextTick(); assert.equal(state.page.value, 1);
  state.nextPage(); state.nextPage(); await Vue.nextTick();
  library.effective.fingerprints = [rule('remaining')];
  await state.reloadLibrary(); await Vue.nextTick();
  assert.equal(state.currentPage.value, 1);
  assert.deepEqual(ids(state.pagedEntries.value), ['remaining']);
  state.previousPage(); assert.equal(state.page.value, 1);
});

test('validation errors hide stale catalog and successful counts until recovery', async () => {
  const { state, library, mount } = setup(); await mount();
  assert.equal(state.effectiveCount.value, 1);
  library.user.error = 'invalid boolean file';
  await state.reloadLibrary(); await Vue.nextTick();
  assert.equal(state.catalogReady.value, false);
  assert.equal(state.catalogEntries.value.length, 0);
  assert.match(state.catalogError.value, /invalid boolean file/);
  library.user.error = '';
  library.effective = { fingerprints: [], error: 'cannot read effective rules' };
  await state.reloadLibrary();
  assert.equal(state.catalogReady.value, false);
  library.effective = { fingerprints: [rule('recovered')], error: '' };
  await state.reloadLibrary();
  assert.equal(state.catalogReady.value, true);
  assert.deepEqual(ids(state.catalogEntries.value), ['recovered']);
});

test('failed IPC reload cannot leave old rows labelled current', async () => {
  let fail = false;
  const { state, mount } = setup({ scan_rule_library: (_, { library }) => { if (fail) throw Error('loading failed'); return library; } });
  await mount(); fail = true; await state.reloadLibrary();
  assert.equal(state.catalogReady.value, false);
  assert.equal(state.filteredEntries.value.length, 0);
  assert.match(state.error.value, /loading failed/);
});

test('condition details preserve nested parentheses and AND/OR instead of flattening leaves', () => {
  const { state } = setup();
  assert.equal(state.conditionText(treeRule().condition), 'ALL (\n  Body ~ required-brand\n  AND\n  ANY (\n    Header server ~ specific-server\n    OR\n    Body ~ alternative-marker\n  )\n)');
  assert.equal(state.conditionText({ type: 'cookie', pattern: 'APP_SESSION=' }), 'Set-Cookie ~ APP_SESSION=');
});

test('legacy feature details retain independent signals and path internal AND', () => {
  const { state } = setup();
  const fp = rule('legacy', 'Legacy', { headerRegex: { server: 'legacy-server' }, setCookieRegex: ['LEGACY='], pathSignals: [{ path: '/login', status: [200, 401], bodyRegex: 'legacy-brand' }] });
  const text = Array.from(state.signals(fp)).join('\n');
  assert.match(text, /Header server ~ legacy-server/);
  assert.match(text, /Set-Cookie ~ LEGACY=/);
  assert.match(text, /\/login \[200,401\] AND Body ~ legacy-brand/);
});

test('old backend marker blocks saving a condition rule without submitting IPC', async () => {
  const { state, library, calls, mount } = setup(); library.booleanRuntime = ''; await mount();
  state.mode.value = 'add'; state.editorText.value = JSON.stringify(treeRule());
  await state.saveAdd();
  assert.equal(calls.some(call => call.command === 'scan_add_user_fingerprint'), false);
  assert.match(state.actionError.value, /条件树/);
  assert.equal(state.busy.value, false);
  assert.ok(state.editorText.value);
});

test('untrusted imported marker cannot enable condition support on an old backend', async () => {
  const { state, library, calls, mount } = setup(); library.booleanRuntime = ''; await mount();
  for (const document of [[treeRule()], { schemaVersion: 2, booleanRuntime: 'condition-tree-v1', fingerprints: [treeRule()] }]) {
    state.mode.value = 'import'; state.editorText.value = JSON.stringify(document);
    await state.doImport();
    assert.match(state.actionError.value, /条件树/);
  }
  assert.equal(calls.some(call => call.command === 'scan_import_user_fingerprints'), false);
  assert.equal(state.busy.value, false);
});

test('unsupported flat matchMode cannot silently downgrade to legacy OR', async () => {
  for (const marker of ['', 'condition-tree-v1']) {
    const { state, library, calls, mount } = setup(); library.booleanRuntime = marker; await mount();
    state.mode.value = 'add'; state.editorText.value = JSON.stringify({ ...rule('flat'), matchMode: 'all' });
    await state.saveAdd(); assert.match(state.actionError.value, /condition/);
    state.mode.value = 'import'; state.editorText.value = JSON.stringify({ fingerprints: [{ ...rule('flat'), matchMode: null }] });
    await state.doImport(); assert.match(state.actionError.value, /matchMode/);
    assert.equal(calls.some(call => ['scan_add_user_fingerprint','scan_import_user_fingerprints'].includes(call.command)), false);
  }
});

test('AND/OR example template is enabled only by actual backend capability', async () => {
  const { state, library, mount } = setup(); library.booleanRuntime = ''; await mount();
  state.startBooleanAdd(); assert.equal(state.mode.value, '');
  library.booleanRuntime = 'condition-tree-v1'; await state.reloadLibrary(); state.startBooleanAdd();
  const fp = JSON.parse(state.editorText.value);
  assert.equal(state.mode.value, 'add');
  assert.equal(fp.condition.type, 'all');
  assert.equal(fp.condition.children[1].type, 'any');
  assert.equal(fp.bodyRegex, undefined);
});

test('saving a legacy rule on old backend preserves CRUD and refreshes authoritative count', async () => {
  const { state, library, calls, mount } = setup({ scan_add_user_fingerprint: ({ fingerprint }, { library }) => {
    library.user.fingerprints = [fingerprint];
    library.effective = { fingerprints: [rule('builtin', 'Builtin'), fingerprint], error: '' };
    return library.user.fingerprints;
  } });
  library.booleanRuntime = ''; await mount(); await state.startAdd();
  assert.equal(state.mode.value, 'add'); await state.saveAdd();
  assert.equal(calls.filter(call => call.command === 'scan_rule_library').length, 2);
  assert.equal(state.effectiveCount.value, 2);
  assert.equal(state.mode.value, '');
  assert.equal(state.busy.value, false);
});

test('saving nested condition forwards the entire tree and reloads combined catalog', async () => {
  const { state, calls, mount } = setup({ scan_add_user_fingerprint: ({ fingerprint }, { library }) => {
    library.user.fingerprints = [fingerprint];
    library.effective = { fingerprints: [rule('builtin'), fingerprint], error: '' };
    return library.user.fingerprints;
  } });
  await mount(); state.startBooleanAdd(); await state.saveAdd();
  const command = calls.find(call => call.command === 'scan_add_user_fingerprint');
  assert.equal(command.args.fingerprint.condition.type, 'all');
  assert.equal(command.args.fingerprint.condition.children[1].type, 'any');
  assert.equal(state.effectiveCount.value, 2);
  assert.equal(state.customCount.value, 1);
  assert.equal(state.mode.value, '');
});

test('import requests merge without replace flags and displays retained existing rule', async () => {
  const existing = rule('existing', 'Original', { bodyRegex: ['original-brand'] });
  const { state, library, calls, mount } = setup({ scan_import_user_fingerprints: (_, { library }) => {
    library.user.fingerprints = [existing, treeRule('new-tree')];
    library.effective = { fingerprints: [rule('builtin'), ...library.user.fingerprints], error: '' };
    return library.user.fingerprints;
  } });
  library.user.fingerprints = [existing]; await mount();
  const text = JSON.stringify({ schemaVersion: 2, fingerprints: [rule('existing', 'Wrong replacement'), treeRule('new-tree')] });
  state.startImport(); state.editorText.value = text; await state.doImport();
  const command = calls.find(call => call.command === 'scan_import_user_fingerprints');
  assert.equal(command.args.text, text);
  assert.deepEqual(Object.keys(command.args), ['text']);
  assert.equal(state.effectiveCount.value, 3);
  assert.equal(state.catalogEntries.value.find(entry => entry.fingerprint.id === 'existing').fingerprint.name, 'Original');
  assert.equal(state.mode.value, '');
});

test('delete is scoped to selected custom ID and reloads authoritative data', async () => {
  const { state, library, calls, mount } = setup({ scan_delete_user_fingerprint: ({ id }, { library }) => {
    assert.equal(id, 'custom');
    library.user.fingerprints = [];
    library.effective = { fingerprints: [rule('builtin')], error: '' };
    return [];
  } });
  library.user.fingerprints = [rule('custom')]; await mount(); await state.remove('custom');
  assert.equal(calls.filter(call => call.command === 'scan_rule_library').length, 2);
  assert.equal(state.effectiveCount.value, 1);
  assert.equal(state.customCount.value, 0);
  assert.equal(state.busy.value, false);
});

test('invalid editor JSON never reaches add/import commands and stays editable', async () => {
  const { state, calls, mount } = setup(); await mount();
  state.mode.value = 'add'; state.editorText.value = '{invalid'; await state.saveAdd();
  assert.match(state.actionError.value, /JSON/); assert.equal(state.editorText.value, '{invalid');
  state.mode.value = 'import'; await state.doImport();
  assert.match(state.actionError.value, /JSON/);
  assert.equal(calls.some(call => /scan_(add|import)_user/.test(call.command)), false);
  assert.equal(state.busy.value, false);
});

test('failed import retains selection and previous catalog without claiming success', async () => {
  const { state, calls, mount } = setup({ scan_import_user_fingerprints: () => { throw Error('validation failed'); } });
  await mount(); state.startImport(); state.editorText.value = JSON.stringify({ fingerprints: [treeRule()] });
  const original = state.editorText.value; await state.doImport();
  assert.equal(state.editorText.value, original);
  assert.equal(state.mode.value, 'import');
  assert.match(state.actionError.value, /validation failed/);
  assert.equal(state.effectiveCount.value, 1);
  assert.equal(calls.filter(call => call.command === 'scan_rule_library').length, 1);
});

test('export uses current backend command and preserves its whole JSON document', async () => {
  const document = JSON.stringify({ schemaVersion: 2, booleanRuntime: 'condition-tree-v1', fingerprints: [treeRule()] });
  const { state, mount } = setup({ scan_export_user_fingerprints: () => document });
  await mount(); await state.startExport();
  assert.equal(state.editorText.value, document);
  assert.equal(state.mode.value, 'export');
  assert.equal(state.busy.value, false);
});
