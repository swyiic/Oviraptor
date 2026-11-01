import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';

// Execute the production TypeScript helpers with synthetic credentials and a
// mocked HTTP boundary. No model service, application state or database is used.
const require = createRequire(new URL('../package.json', import.meta.url));
const ts = require('typescript');
const root = fileURLToPath(new URL('../', import.meta.url));
const modules = new Map();
function load(relative) {
  const filename = path.resolve(root, relative);
  if (modules.has(filename)) return modules.get(filename);
  const exports = {};
  modules.set(filename, exports);
  const compiled = ts.transpileModule(readFileSync(filename, 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  vm.runInNewContext(compiled, { exports, URL, require(name) {
    if (name.startsWith('.')) return load(path.resolve(path.dirname(filename), `${name}.ts`));
    throw new Error(`Unexpected module: ${name}`);
  } }, { filename });
  return exports;
}
const { modelSettingsForConnection } = load('src/features/configuration/modelConnections.ts');
const { configPolicyPreviewSettings, modelConnectionSignature } = load('src/features/configuration/configProfileSections.ts');
const { createHostSettingsClient } = load('src/features/agentHost/settingsClient.ts');
const profile = (id, extra = {}) => ({ id, name: id, llm: 'openai/example-model', apiBase: 'https://models.example.test/v1', deployment: 'cloud', apiKey: 'synthetic-test-key', localApiKey: '', ...extra });
const plain = value => JSON.parse(JSON.stringify(value));
const ref = { policyId: 'synthetic-policy', revision: 7, kind: 'models', modelProfileId: 'model-a' };
function settingsClient() {
  const calls = [];
  const client = createHostSettingsClient({ origin: 'http://127.0.0.1:12345', session: 'synthetic-session', fetchImpl: async (url, init) => {
    calls.push({ path: new URL(url).pathname, body: init.body ? JSON.parse(init.body) : null, headers: init.headers });
    return { ok: true, text: async () => JSON.stringify({ runtime: { id: 'xai-api' } }) };
  } });
  return { client, calls };
}

test('adding another model preserves the selected default and all prior connections', () => {
  const original = { modelProfiles: [profile('first')], activeModelProfileId: 'first' };
  const settings = modelSettingsForConnection(original, profile('second'), false);
  assert.deepEqual(Array.from(settings.modelProfiles, item => item.id), ['first', 'second']);
  assert.equal(settings.activeModelProfileId, 'first');
  assert.equal(original.modelProfiles.length, 1);
});

test('first model becomes the default and editing by ID does not create duplicates', () => {
  const first = modelSettingsForConnection({}, profile('first'), false);
  const edited = modelSettingsForConnection(first, profile('first', { name: 'Renamed' }), false);
  assert.equal(edited.activeModelProfileId, 'first');
  assert.equal(edited.modelProfiles.length, 1);
  assert.equal(edited.modelProfiles[0].name, 'Renamed');
});

test('an OAuth default retains its authentication type without copying API credentials', () => {
  const oauth = profile('grok', { llm: 'grok-4.7', authType: 'oauth', apiBase: 'https://api.x.ai/v1', apiKey: '', localApiKey: '' });
  const settings = modelSettingsForConnection({ modelProfiles: [profile('first')], activeModelProfileId: 'first' }, oauth, true);
  assert.equal(settings.activeModelProfileId, 'grok');
  assert.equal(settings.modelAuthType, 'oauth');
  assert.equal(settings.modelApiKey, '');
  assert.equal(settings.localApiKey, '');
  assert.equal(settings.modelProfiles.length, 2);
});

test('model payload projection retains OAuth identity and changing authentication invalidates a prior test', () => {
  const api = profile('grok');
  const oauth = { ...api, authType: 'oauth' };
  const settings = configPolicyPreviewSettings({ modelAuthType: 'oauth', modelProfiles: [oauth], unrelated: true }, 'models');
  assert.equal(settings.modelAuthType, 'oauth');
  assert.equal(settings.modelProfiles[0].authType, 'oauth');
  assert.equal(settings.unrelated, undefined);
  assert.notEqual(modelConnectionSignature(api), modelConnectionSignature(oauth));
  assert.equal(modelConnectionSignature(api), modelConnectionSignature({ ...api, authType: 'api-key' }));
});

test('bound API connections keep their explicit endpoint even when the model name is Grok', async () => {
  const { client, calls } = settingsClient();
  await client.setProvider({ provider: 'openai-compatible', model: ' openai/grok-private-model ', baseUrl: 'https://gateway.example.test/v1', apiKey: 'synthetic-test-key', connectionRef: ref });
  const request = calls.find(call => call.path === '/api/settings/provider');
  assert.equal(request.body.model, 'openai/grok-private-model');
  assert.equal(request.body.baseUrl, 'https://gateway.example.test/v1');
  assert.equal(request.body.provider, 'openai-compatible');
  assert.deepEqual(plain(request.body.connectionRef), ref);
});

test('bound model IDs preserve custom namespaces and case', async () => {
  const { client, calls } = settingsClient();
  await client.setProvider({ provider: 'openai-compatible', model: 'vendor/DeepSeek-private', baseUrl: 'http://127.0.0.1:9000/v1', authType: 'none', clearApiKey: true, connectionRef: ref });
  assert.equal(calls[0].body.model, 'vendor/DeepSeek-private');
  assert.equal(calls[0].body.baseUrl, 'http://127.0.0.1:9000/v1');
  assert.equal(calls[0].body.authType, 'none');
  assert.equal(calls[0].body.apiKey, undefined);
});

test('bound model IDs reach Host intact so nested openai prefixes are normalized only once', async () => {
  const { client, calls } = settingsClient();
  await client.setProvider({ provider: 'openai-compatible', model: ' openai/openai/custom ', baseUrl: 'https://gateway.example.test/v1', connectionRef: ref });
  assert.equal(calls[0].body.model, 'openai/openai/custom');
});

test('OAuth sign-in can save credentials without activating a different default', async () => {
  const { client, calls } = settingsClient();
  await client.startOAuth({ background: true, activate: false });
  assert.deepEqual(calls[0].body, { background: true, activate: false });
  assert.equal(calls[0].path, '/api/settings/provider/oauth/start');
  assert.equal(calls.length, 1);
});

test('OAuth selection sends the selected model and catalog binding, with no credential material', async () => {
  const { client, calls } = settingsClient();
  await client.selectOAuth({ model: 'grok-4.7', connectionRef: ref });
  assert.equal(calls[0].path, '/api/settings/provider/oauth/select');
  assert.deepEqual(calls[0].body, { model: 'grok-4.7', connectionRef: ref });
  assert.equal(calls[0].headers['X-Oviraptor-Session'], 'synthetic-session');
});
