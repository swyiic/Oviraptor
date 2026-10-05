const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');

function load(relativePath, dependencies = {}) {
  const source = fs.readFileSync(path.join(__dirname, '..', relativePath), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText;
  const exports = {};
  new Function('require', 'exports', compiled)((id) => dependencies[id] || require(id), exports);
  return exports;
}

const presentation = load('src/features/sentinel/presentation.ts');
const { useApiResults } = load('src/features/sentinel/results/useApiResults.ts', {
  '../presentation': presentation,
});

const finding = (id, record) => ({ id, kind: 'api', recordJson: JSON.stringify(record) });

test('API display follows the selected target and preserves source evidence', () => {
  const selectedUrl = vue.ref('https://example.test/app/');
  const current = vue.ref([finding(1, { path: '/v1/users?limit=2', method: 'post',
    parameters: [{ name: 'limit' }], responseKeys: ['id'], source: 'browser',
    observedCount: 2, identityKeys: ['owner'], requestBody: { limit: 2 },
    responseHeaders: { 'x-trace': 'yes' } })]);
  const state = useApiResults(selectedUrl, (...kinds) => current.value.filter((row) => kinds.includes(row.kind)));
  const item = state.apiRows.value[0];
  assert.equal(state.apiUrl(item), 'https://example.test/v1/users?limit=2');
  assert.equal(state.apiPath(item), '/v1/users');
  assert.deepEqual(state.apiQuery(item), ['limit']);
  assert.equal(state.apiMethod(item), 'POST');
  assert.match(state.apiSourceSummary(item), /2 次观察/);
  assert.equal(state.apiIdentitySummary(item), 'owner');
  assert.deepEqual(state.apiRequestPayload(item), { limit: 2 });
  assert.deepEqual(state.apiResponseHeaders(item), { 'x-trace': 'yes' });
  state.toggleApiRow(1);
  assert.deepEqual(state.expandedApiRows.value, [1]);
  state.toggleApiRow(1);
  assert.deepEqual(state.expandedApiRows.value, []);
  selectedUrl.value = 'https://other.test/';
  current.value = [finding(2, { path: 'status', queryKeys: { verbose: true } })];
  assert.equal(state.apiRows.value[0].id, 2);
  assert.equal(state.apiUrl(state.apiRows.value[0]), 'https://other.test/status');
  assert.deepEqual(state.apiQuery(state.apiRows.value[0]), ['verbose']);
});

test('malformed historical JSON remains display-only and does not throw', () => {
  const item = { id: 3, kind: 'api', recordJson: '{broken' };
  const state = useApiResults(vue.ref('https://example.test/'), () => [item]);
  assert.equal(state.apiMethod(item), 'GET');
  assert.deepEqual(state.apiQuery(item), []);
  assert.deepEqual(state.apiRequestPayload(item), {});
  assert.deepEqual(state.apiResponseHeaders(item), {});
  assert.equal(state.apiPath(item), '/');
});
