const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');

const source = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/fuse/presentation.ts'), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const presentation = {};
new Function('exports', compiled)(presentation);

test('saved routing evidence takes precedence when describing why a target stopped', () => {
  const item = { reason: 'HTTP 超时' };
  const target = { routingReason: 'SourceMap；API；Token 熔断' };
  assert.deepEqual(presentation.fuseReasonParts(item, target), [
    { text: 'SourceMap', tone: 'sourcemap', label: 'SourceMap' },
    { text: 'API', tone: 'api', label: 'API' },
    { text: 'Token 熔断', tone: 'fuse', label: '熔断' },
  ]);
  assert.equal(presentation.fuseReasonCategory(item, target), 'budget');
  assert.match(presentation.fuseRecommendedAction(item, target), /Token/);
});

test('a missing target still describes the saved stop reason without granting a resume', () => {
  const item = { reason: '403 鉴权失败' };
  assert.equal(presentation.fuseReasonCategory(item), 'access');
  assert.equal(presentation.fuseReasonParts(item)[0].text, item.reason);
  assert.match(presentation.fuseRecommendedAction(item), /登录态/);
  assert.equal(presentation.fuseCategoryLabel('unknown'), 'unknown');
});
