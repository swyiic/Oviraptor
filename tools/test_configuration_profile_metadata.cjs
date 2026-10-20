const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { transpile } = require('./shell_module_harness.cjs');
const filename = 'src/features/configuration/components/ConfigProfileMetadataEditor.vue';
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'profile-metadata' });

function harness(t, isDefault = false) {
  const calls = [], events = [], disposers = [];
  let response = async input => input.profileId;
  const exports = {};
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return { ...vue, onUnmounted: fn => disposers.push(fn) };
    if (name === '@lucide/vue') return {};
    if (name.endsWith('ModalShell.vue')) return { default: {} };
    if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: (_zh, en) => en }) };
    if (name === '../api') return { configurationApi: {
      saveProfileMetadata: async input => { calls.push(structuredClone(input)); return response(input); },
      saveProfile: () => { throw new Error('Full settings save must not be used'); },
    } };
    throw new Error(`Unexpected metadata dialog import: ${name}`);
  }, exports);
  const props = vue.reactive({ profile: { id: 7, name: 'Original name', description: 'Original description', isDefault, settings: { modelProfiles: [{ id: 'fixture' }], unknown: 'retained' } } });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.default.setup(props, { expose() {}, emit: (...event) => events.push(event) }));
  t.after(() => { for (const dispose of disposers) dispose(); scope.stop(); });
  return { state, props, calls, events, setResponse: fn => { response = fn; } };
}

test('rename saves only explicit metadata and the original metadata expectation', async t => {
  const { state, props, calls, events } = harness(t);
  state.form.name = '  Renamed  '; state.form.description = '  Updated  ';
  await state.save();
  assert.deepEqual(calls, [{ profileId: 7, name: 'Renamed', description: 'Updated', isDefault: false, expected: { name: 'Original name', description: 'Original description', isDefault: false } }]);
  assert.equal(Object.hasOwn(calls[0], 'settings'), false);
  assert.deepEqual(props.profile.settings, { modelProfiles: [{ id: 'fixture' }], unknown: 'retained' });
  assert.deepEqual(events, [['saved', 7]]);
});

test('a non-default profile becomes default only after the checkbox is explicitly changed', async t => {
  const { state, calls } = harness(t);
  assert.equal(state.form.isDefault, false);
  await state.save();
  assert.equal(calls[0].isDefault, false);
  state.form.isDefault = true;
  await state.save();
  assert.equal(calls[1].isDefault, true);
  assert.equal(calls[1].expected.isDefault, false);
});

test('existing default state is retained without a checkbox change', async t => {
  const { state, calls } = harness(t, true);
  state.form.name = 'Renamed default'; await state.save();
  assert.equal(calls[0].isDefault, true);
  assert.equal(calls[0].expected.isDefault, true);
});

test('a default profile cannot be unset without a replacement', async t => {
  const { state, calls, events } = harness(t, true);
  state.form.isDefault = false; await state.save();
  assert.equal(calls.length, 0);
  assert.deepEqual(events, []);
  assert.match(state.error.value, /replacement default/);
});

test('save errors remain visible without reporting success or discarding the draft', async t => {
  const { state, calls, events, setResponse } = harness(t);
  setResponse(async () => { throw new Error('fixture metadata conflict'); });
  state.form.name = 'Keep this draft'; await state.save();
  assert.equal(calls.length, 1);
  assert.equal(state.error.value, 'fixture metadata conflict');
  assert.equal(state.form.name, 'Keep this draft');
  assert.equal(state.busy.value, false);
  assert.deepEqual(events, []);
});

test('blank names cannot submit and duplicate in-flight saves make one request', async t => {
  const { state, calls, setResponse } = harness(t);
  state.form.name = '  '; await state.save();
  assert.equal(calls.length, 0);
  let resolve; setResponse(() => new Promise(done => { resolve = done; }));
  state.form.name = 'One request'; const pending = state.save(); await state.save();
  assert.equal(calls.length, 1); resolve(7); await pending;
});

test('changing the selected profile prevents a late save from closing the new dialog', async t => {
  const { state, props, calls, events, setResponse } = harness(t);
  let resolve; setResponse(() => new Promise(done => { resolve = done; }));
  const pending = state.save();
  props.profile = { ...props.profile, id: 8, name: 'Other profile', isDefault: true };
  await vue.nextTick(); resolve(7); await pending;
  assert.equal(calls[0].profileId, 7);
  assert.equal(state.form.name, 'Other profile');
  assert.equal(state.form.isDefault, true);
  assert.deepEqual(events, []);
});

test('inline metadata editor template compiles', () => {
  const compiled = compileTemplate({ filename, id: 'profile-metadata', source: descriptor.template.content, compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(compiled.errors, []);
});
