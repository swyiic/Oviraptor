const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { loadPure, transpile } = require('./shell_module_harness.cjs');

const sections = loadPure('src/features/configuration/configProfileSections.ts');
const defaults = loadPure('src/features/configuration/configProfileDefaults.ts');
const filename = 'src/features/configuration/components/ProfileSectionEditor.vue';
const source = fs.readFileSync(filename, 'utf8');
const { descriptor } = parse(source, { filename });
const script = compileScript(descriptor, { id: 'profile-scopes' });

function harness(t, settings = {}, initialSection = 'assetApis', id = 7) {
  const calls = [], savedRules = new Map(), emitted = [];
  const mounted=[],unmounted=[];let ruleReads=0;
  let modelResult = async () => ({ ok: true, message: 'fixture connected' });
  const api = {
    saveProfile: async input => { calls.push(['profile', JSON.parse(JSON.stringify(input))]); return id || 8; },
    saveProfileSection: async input => { calls.push(['section', JSON.parse(JSON.stringify(input))]); return input.profileId; },
    adoptProfilePolicy: async input => { calls.push(['adopt-policy', JSON.parse(JSON.stringify(input))]); return input.profileId; },
    testModelProfile: async input => { calls.push(['model-test', input]); return modelResult(input); },
    testFofaApi: async input => { calls.push(['fofa-test', structuredClone(input)]); return { ok: true, message: 'fixture connected' }; },
  };
  const ruleApi = {
    listSecurityRulePacks: async () => { ruleReads++;return [...savedRules.values()]; },
    saveSecurityRulePack: async input => {
      calls.push(['rule-save', structuredClone(input)]);
      savedRules.set(input.key, { ...input, id: savedRules.size + 1, status: 'ready' });
      return savedRules.get(input.key).id;
    },
    syncSecurityRulePack: async packId => { calls.push(['rule-sync', packId]); },
  };
  const exports = {};
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return { ...vue, onMounted: fn => mounted.push(fn), onUnmounted:fn=>unmounted.push(fn) };
    if (name === '@lucide/vue') return {};
    if (name.endsWith('ModalShell.vue')) return { default: {} };
    if (name === '../api') return { configurationApi: api };
    if (name.endsWith('/rulePacksApi')) return { scanningRulePacksApi: ruleApi };
    if (name.endsWith('/i18n')) return { useI18n: () => ({ tr: (_zh, en) => en }) };
    if (name === '../configProfileDefaults') return defaults;
    if (name === '../configProfileSections') return sections;
    throw new Error(`Unexpected ProfileSectionEditor import: ${name}`);
  }, exports);
  const props = vue.reactive({ profile: { id, name: 'Fixture profile', description: 'fixture', isDefault: true, settings }, section: initialSection });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.default.setup(props, { expose() {}, emit: (...event) => emitted.push(event) }));
  for(const mount of mounted)mount();
  t.after(() => scope.stop());
  return { state, props, calls, emitted, api, unmount:()=>unmounted.forEach(fn=>fn()), getRuleReads:()=>ruleReads, setModelResult: fn => { modelResult = fn; } };
}

test('section ownership excludes absent and unrelated snapshot fields', () => {
  const snapshot = { fofaKey: 'fixture', unknown: { retained: true }, modelProfiles: [] };
  assert.deepEqual(sections.pickSectionSettings(snapshot, 'assetApis'), { fofaKey: 'fixture' });
  assert.deepEqual(sections.pickSectionSettings(snapshot, 'network'), {});
  assert.equal(sections.resolveConfigProfileSection('models'), 'models');
  assert.equal(sections.resolveConfigProfileSection('not-a-section'), 'assetApis');
  assert.deepEqual(sections.pickSectionSettings({ contentThreshold: 2, workers: 6 }, 'rules'), { contentThreshold: 2 });
  assert.deepEqual(sections.pickSectionSettings({ contentThreshold: 2, workers: 6 }, 'probe'), { workers: 6 });
});

test('copying a reactive profile preserves nested history without sharing editable values', () => {
  const profile = vue.ref({ settings: { modelProfiles: [{ id: 'fixture', apiKey: 'synthetic' }], unknown: { retained: ['value'] } } });
  assert.throws(() => structuredClone(profile.value.settings), { name: 'DataCloneError' });
  const copy = sections.cloneConfigProfileSettings(profile.value.settings);
  copy.modelProfiles[0].apiKey = 'changed'; copy.unknown.retained.push('copy');
  assert.equal(profile.value.settings.modelProfiles[0].apiKey, 'synthetic');
  assert.deepEqual([...profile.value.settings.unknown.retained], ['value']);
  assert.equal(vue.isProxy(copy), false);
  const app = fs.readFileSync('src/App.vue', 'utf8');
  assert.doesNotMatch(app, /<ConfigDialog|<ConfigProfileMetadataDialog|function openProfile\(/);
});

test('the dialog template compiles with independent model/API/network sections', () => {
  const compiled = compileTemplate({ filename, id: 'profile-scopes', source: descriptor.template.content, compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(compiled.errors, []);
  for (const key of ['models', 'assetApis', 'network']) assert.match(source, new RegExp(`tab === '${key}'`));
  for (const field of ['windowsRuntimeDirectory', 'scriptsDirectory', 'agentBatchSize', 'collectionMode', 'maxDerivedDomains', 'includeWeakFingerprints']) {
    assert.doesNotMatch(source, new RegExp(`v-model[^>]*form\\.settings\\.${field}`));
    assert.equal(Object.hasOwn(defaults.CONFIG_PROFILE_SETTING_DEFAULTS, field), false);
  }
});

test('business save patches one section without testing or rewriting model and rule settings', async t => {
  const model = { id: 'm', name: 'Fixture', llm: 'fixture-model', apiBase: '', apiKey: '', localApiKey: '', deployment: 'cloud', contextWindow: 12345 };
  const stored = { fofaKey: 'old-fixture', modelProfiles: [model], activeModelProfileId: 'm', nativeAnalyzers: { semgrep: { image: 'legacy:tag', unknown: true } }, unknown: { retained: true }, windowsRuntimeDirectory: 'historical' };
  const { state, calls, props } = harness(t, stored);
  state.form.settings.fofaKey = 'new-fixture';
  state.form.settings.modelProfiles[0].apiBase = 'https://unsaved.example.test/v1';
  await state.save();
  assert.equal(state.error.value, '');
  assert.deepEqual(calls.map(call => call[0]), ['adopt-policy']);
  assert.deepEqual(calls[0][1], { profileId: 7, section: 'assetApis', settings: { fofaEmail: '', fofaKey: 'new-fixture', configPath: '' }, expectedSettings: { fofaKey: 'old-fixture' } });
  assert.equal(props.profile.settings.modelProfiles[0].apiBase, '');
  assert.equal(state.form.settings.modelProfiles[0].contextWindow, 12345);
  assert.deepEqual(state.form.settings.unknown, { retained: true });
  assert.equal(state.form.settings.windowsRuntimeDirectory, 'historical');
});

test('FOFA sends explicit account and current business-network route; the key starts hidden', async t => {
  const {state,props,calls}=harness(t,{fofaEmail:' account@example.invalid ',fofaKey:' fixture-key ',proxyUrl:'http://old.invalid:1'});
  props.category={section:'assetApis'};props.proxyUrlContext='http://127.0.0.1:9009';
  assert.equal(state.showFofaKey.value,false);
  await state.testFofaApi();
  assert.deepEqual(calls.find(call=>call[0]==='fofa-test')[1],{key:'fixture-key',email:'account@example.invalid',proxyUrl:'http://127.0.0.1:9009'});
  assert.match(source,/:type="showFofaKey\s*\?\s*'text'\s*:\s*'password'"/);
  props.proxyUrlContext=undefined;await state.testFofaApi();
  assert.equal(calls.filter(call=>call[0]==='fofa-test').length,1);
  assert.match(state.fofaTestMessage.value,/test route is unknown/);
});

test('unchanged scan models do not require a connectivity test for a model-section save', async t => {
  const { state, calls } = harness(t, { modelProfiles: [{ id: 'm', name: 'Fixture', llm: 'fixture', apiBase: '', apiKey: '', localApiKey: '', deployment: 'cloud' }], activeModelProfileId: 'm', _configurationRefs: { models: { policyId: 'owned-models', revision: 1, kind: 'models' } } }, 'models');
  state.form.settings.modelProfiles[0].name = 'Renamed locally';
  await state.save();
  assert.equal(state.error.value, '');
  assert.deepEqual(calls.map(call => call[0]), ['section']);
  assert.equal(calls[0][1].settings.modelName, 'fixture');
});

test('changed scan connection requires its own successful test and never approves a failed response', async t => {
  const { state, calls, setModelResult } = harness(t, { modelProfiles: [{ id: 'm', name: 'Fixture', llm: 'fixture', apiBase: '', apiKey: '', localApiKey: '', deployment: 'cloud' }], activeModelProfileId: 'm', _configurationRefs: { models: { policyId: 'owned-models', revision: 1, kind: 'models' } } }, 'models');
  state.form.settings.modelProfiles[0].apiBase = 'https://changed.example.test/v1';
  await state.save();
  assert.equal(calls.length, 0);
  assert.match(state.error.value, /changed/);
  setModelResult(async () => ({ ok: false, message: 'fixture refused' }));
  await state.testActiveAgentProfile();
  await state.save();
  assert.equal(calls.filter(call => call[0] === 'section').length, 0);
  setModelResult(async () => ({ ok: true, message: 'fixture connected' }));
  await state.testActiveAgentProfile();
  await state.save();
  assert.equal(calls.filter(call => call[0] === 'section').length, 1);
});

test('late model test cannot approve a connection edited while awaiting its response', async t => {
  const { state, setModelResult } = harness(t, { modelProfiles: [{ id: 'm', name: 'Fixture', llm: 'fixture', apiBase: '', apiKey: '', localApiKey: '', deployment: 'cloud' }], activeModelProfileId: 'm' }, 'models');
  let resolve;
  setModelResult(() => new Promise(done => { resolve = done; }));
  const pending = state.testActiveAgentProfile();
  state.form.settings.modelProfiles[0].apiBase = 'https://edited.example.test/v1';
  resolve({ ok: true, message: 'fixture connected' });
  await pending;
  assert.equal(state.profileHasAcceptedTest(state.activeAgentProfile.value), false);
});

test('global rule edits save only through their explicit action and keep enabled choice', async t => {
  const { state, calls } = harness(t, { semgrepRuleRepository: 'https://historical.example.test/rules.git' }, 'securityRules');
  await state.loadRulePacks();
  const rule = state.ruleDefinitions[0];
  state.ruleDrafts[rule.key].repository = 'https://fixture.example.test/rules.git';
  state.ruleDrafts[rule.key].enabled = false;
  await state.save();
  assert.deepEqual(calls.map(call => call[0]), ['adopt-policy']);
  assert.deepEqual(Object.keys(calls[0][1].settings), ['nativeAnalyzers']);
  await state.syncRulePack(rule);
  assert.deepEqual(calls.map(call => call[0]), ['adopt-policy']);
  await state.saveRulePack(rule);
  assert.deepEqual(calls.map(call => call[0]), ['adopt-policy', 'rule-save']);
  assert.equal(calls[1][1].enabled, false);
  assert.equal(state.rulePackChanged(rule.key), false);
  assert.equal(state.form.settings.semgrepRuleRepository, 'https://historical.example.test/rules.git');
});

test('inline section has no full-profile creation fallback and follows its fixed section prop', async t => {
  const { state, props, calls } = harness(t, { unknown: 'retained', modelProfiles: [] }, 'collection', 0);
  assert.equal(state.tab.value, 'collection');
  props.section = 'network'; await vue.nextTick();
  assert.equal(state.tab.value, 'network');
  await state.save();
  assert.deepEqual(calls, []);
  assert.match(state.error.value, /not loaded|compatibility scope/);
  assert.equal(state.form.settings.unknown, 'retained');
});

test('policy adoption previews only the chosen category and requires explicit confirmation', async t => {
  const { state, calls, emitted } = harness(t, { pageSize: 100, fofaKey: 'synthetic-secret', modelProfiles: [] }, 'collection');
  state.form.settings.pageSize = 200;
  assert.equal(state.canAdoptPolicy.value, true);
  await state.adoptPolicy(); assert.equal(calls.length, 0);
  state.previewPolicy(); assert.equal(calls.length, 0);
  assert.equal(state.policyPreview.value.settings.pageSize, 200);
  assert.equal(JSON.stringify(state.policyPreview.value).includes('synthetic-secret'), false);
  await state.adoptPolicy();
  assert.deepEqual(calls.map(call => call[0]), ['adopt-policy']);
  assert.equal(calls[0][1].section, 'collection');
  assert.deepEqual(calls[0][1].expectedSettings, { pageSize: 100 });
  assert.deepEqual(emitted, [['saved', 7]]);
});

test('a policy draft changed after preview cannot be adopted or carried across categories', async t => {
  const { state, calls } = harness(t, { pageSize: 100 }, 'collection');
  state.previewPolicy(); state.form.settings.pageSize = 300;
  await state.adoptPolicy();
  assert.equal(calls.length, 0); assert.match(state.error.value, /changed after preview/);
  state.previewPolicy(); state.tab.value = 'probe'; await vue.nextTick();
  assert.equal(state.policyPreview.value, undefined);
});

test('adopted policy saves include the exact expected owner revision', async t => {
  const reference = { policyId: 'fixture-policy', revision: 3, kind: 'probe' };
  const { state, calls } = harness(t, { workers: 6, _configurationRefs: { probe: reference } }, 'probe');
  assert.equal(state.canAdoptPolicy.value, false);
  state.form.settings.workers = 12; await state.save();
  assert.equal(calls[0][0], 'section');
  assert.deepEqual(calls[0][1].expectedReference, reference);
  assert.equal(Object.hasOwn(calls[0][1].settings, '_configurationRefs'), false);
});

test('scan and asset adoption previews hide secrets while keeping exact scoped write payload', async t => {
  const model = { id: 'm', name: 'Synthetic', llm: 'synthetic', apiBase: 'https://synthetic.invalid/v1', apiKey: 'synthetic-cloud-secret', localApiKey: '', deployment: 'cloud' };
  const { state, calls } = harness(t, { modelProfiles: [model], activeModelProfileId: 'm', fofaKey: 'unrelated-fofa-secret' }, 'models');
  assert.equal(state.canAdoptPolicy.value, true);
  state.previewPolicy();
  const preview = sections.redactedConfigPreview(state.policyPreview.value.settings);
  assert.equal(JSON.stringify(preview).includes('synthetic-cloud-secret'), false);
  assert.equal(JSON.stringify(preview).includes('unrelated-fofa-secret'), false);
  assert.equal(preview.modelProfiles[0].apiKey, '[configured]');
  await state.adoptPolicy();
  assert.equal(calls[0][0], 'adopt-policy');
  assert.equal(calls[0][1].settings.modelProfiles[0].apiKey, 'synthetic-cloud-secret');
  assert.equal(Object.hasOwn(calls[0][1].settings, 'fofaKey'), false);
  const asset = harness(t, { fofaKey: 'synthetic-fofa-secret', configPath: '' }, 'assetApis');
  asset.state.previewPolicy();
  assert.deepEqual(sections.redactedConfigPreview(asset.state.policyPreview.value.settings), { fofaEmail: '', fofaKey: '[configured]', configPath: '' });
  await asset.state.adoptPolicy(); assert.equal(asset.calls[0][1].section, 'assetApis');
});

test('model adoption cannot bypass the changed connection test gate', async t => {
  const { state, calls } = harness(t, { modelProfiles: [{ id: 'm', name: 'Synthetic', llm: 'synthetic', apiBase: 'https://synthetic.invalid/v1', apiKey: 'synthetic', localApiKey: '', deployment: 'cloud' }], activeModelProfileId: 'm' }, 'models');
  state.form.settings.modelProfiles[0].apiBase = 'https://changed.invalid/v1';
  state.previewPolicy(); await state.adoptPolicy();
  assert.equal(calls.length, 0); assert.match(state.error.value, /connectivity test/);
  await state.testActiveAgentProfile(); await state.adoptPolicy();
  assert.deepEqual(calls.map(call => call[0]), ['model-test', 'adopt-policy']);
});

test('legacy environment-derived model connections require explicit addresses before split preview', t => {
  const { state, calls } = harness(t, { modelProfiles: [{ id: 'm', name: 'Legacy', llm: 'synthetic', apiBase: '', apiKey: 'synthetic', localApiKey: '', deployment: 'cloud' }], activeModelProfileId: 'm' }, 'models');
  state.previewPolicy();
  assert.equal(state.policyPreview.value, undefined); assert.equal(calls.length, 0);
  assert.match(state.error.value, /explicit API address/);
});

test('direct model save shares explicit adoption validation and never borrows environment credentials',async t=>{
  const {state,calls}=harness(t,{modelProfiles:[{id:'m',name:'Legacy',llm:'fixture',deployment:'cloud',apiBase:'',apiKey:'fixture',localApiKey:''}],activeModelProfileId:'m'},'models');
  await state.save();
  assert.deepEqual(calls,[]);
  assert.match(state.error.value,/explicit API address/);
});

test('stale live profile updates preserve the draft and prevent a misleading save',async t=>{
  const reference={policyId:'probe',revision:2,kind:'probe'};
  const h=harness(t,{workers:96,_configurationRefs:{probe:reference}},'probe');
  h.state.form.settings.workers=4;
  h.props.profile={...h.props.profile,settings:{workers:8,_configurationRefs:{probe:{...reference,revision:3}}}};
  await vue.nextTick();await h.state.save();
  assert.equal(h.state.form.settings.workers,4);
  assert.equal(h.state.sourceChanged.value,true);
  assert.deepEqual(h.calls,[]);
  assert.match(h.state.error.value,/Reload before saving/);
});

test('each of eight inline categories saves only its independent owner without global rule reads',async t=>{
  for(const section of sections.INDEPENDENT_POLICY_SECTIONS){
    const h=harness(t,{modelProfiles:[],nativeAnalyzers:{semgrep:{image:''},codeql:{image:''}},_configurationRefs:{[section]:{policyId:`owner-${section}`,revision:2,kind:section}}},section);
    await h.state.save();
    assert.equal(h.state.error.value,'',section);
    assert.deepEqual(h.calls.map(call=>call[0]),['section'],section);
    assert.equal(h.calls[0][1].section,section);
    assert.deepEqual(Object.keys(h.calls[0][1].settings).sort(),[...sections.sectionSettingKeys[section]].filter(key=>Object.hasOwn(h.state.form.settings,key)).sort());
    assert.equal(h.getRuleReads(),section==='securityRules'?1:0,section);
  }
});

test('advanced probe policies retain explicit historical security choices and own only probe fields',async t=>{
  const h=harness(t,{allowPrivate:true,strictTls:true,schemeFallback:false,maxBodyBytes:65536,perHostInterval:2.5,collectionCache:false},'probe');
  await h.state.save();
  const input=h.calls[0][1];
  assert.equal(input.settings.allowPrivate,true);assert.equal(input.settings.strictTls,true);assert.equal(input.settings.schemeFallback,false);assert.equal(input.settings.maxBodyBytes,65536);assert.equal(input.settings.perHostInterval,2.5);
  assert.equal(Object.hasOwn(input.settings,'collectionCache'),false);
  assert.equal(defaults.CONFIG_PROFILE_SETTING_DEFAULTS.allowPrivate,false);
  assert.equal(defaults.CONFIG_PROFILE_SETTING_DEFAULTS.strictTls,false);
});

test('a completed save after leaving the editor cannot remount or discard a newer category draft',async t=>{
  const h=harness(t,{workers:96},'probe');
  let resolve;h.api.adoptProfilePolicy=input=>{h.calls.push(['adopt-policy',input]);return new Promise(done=>resolve=done);};
  const pending=h.state.save();h.unmount();resolve(7);await pending;
  assert.deepEqual(h.emitted,[]);
  assert.equal(h.calls[0][1].section,'probe');
});
