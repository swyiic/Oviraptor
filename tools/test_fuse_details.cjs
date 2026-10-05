const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');

const source = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/fuse/useFuseDetails.ts'), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const details = {};
new Function('require', 'exports', compiled)(require, details);

const item = { id: 1, url: 'https://example.test/path', sourceScanId: 'scan-1' };

test('URL association stays within the source scan and preserves query identity', () => {
  assert.equal(details.sameTargetUrl('https://example.test/path/', item.url), true);
  assert.equal(details.sameTargetUrl('https://example.test/path?a=1', item.url), false);
  const targets = vue.ref([
    { scanId: 'other', url: item.url },
    { scanId: 'scan-1', url: `${item.url}/` },
  ]);
  const state = details.useFuseDetails({ targets, loadFindings: async () => [],
    loadValidations: async () => [], notifyError: () => {} });
  assert.equal(state.fuseTarget(item), targets.value[1]);
});

test('detail toggle loads once, filters evidence, then reopens without a new request', async () => {
  let findingsCalls = 0, validationsCalls = 0;
  const state = details.useFuseDetails({ targets: vue.ref([]),
    loadFindings: async () => { findingsCalls++; return [
      { targetUrl: `${item.url}/`, kind: 'endpoint' },
      { targetUrl: 'https://other.test', kind: 'endpoint' },
    ]; },
    loadValidations: async () => { validationsCalls++; return [
      { url: item.url }, { url: 'https://other.test' },
    ]; }, notifyError: () => {} });
  assert.equal(state.fuseState(item).loaded, false);
  await state.toggleFuseDetail(item);
  assert.equal(state.fuseState(item).loaded, true);
  assert.equal(state.fuseRows(item, 'endpoint').length, 1);
  assert.equal(state.fuseValidationRows(item).length, 1);
  await state.toggleFuseDetail(item);
  await state.toggleFuseDetail(item);
  assert.equal(state.fuseState(item).open, true);
  assert.deepEqual([findingsCalls, validationsCalls], [1, 1]);
});

test('failed detail load closes the panel and permits retry', async () => {
  const errors = [];
  let attempts = 0;
  const state = details.useFuseDetails({ targets: vue.ref([]),
    loadFindings: async () => { if (++attempts === 1) throw Error('offline'); return []; },
    loadValidations: async () => [], notifyError: (error) => errors.push(error) });
  await state.toggleFuseDetail(item);
  assert.equal(state.fuseState(item).open, false);
  assert.equal(state.fuseState(item).loaded, false);
  assert.match(errors[0], /offline/);
  await state.toggleFuseDetail(item);
  assert.equal(state.fuseState(item).loaded, true);
});
