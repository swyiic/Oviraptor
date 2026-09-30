const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');

const modulePath = path.join(__dirname, '../src/features/sentinel/execution/presentation.ts');
const boardPath = path.join(__dirname, '../src/components/SentinelBoard.vue');

test('historical backend is a display-only label, never a selectable native backend', () => {
  const compiled = ts.transpileModule(fs.readFileSync(modulePath, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const presentation = {};
  new Function('exports', compiled)(presentation);
  assert.equal(presentation.agentBackendLabel('native'), '原生 Agent');
  // REM-009 Loop1: neutral historical display, no brand-specific label.
  assert.equal(presentation.agentBackendLabel('legacy_backend_removed'), '历史封存（只读）');
  assert.equal(presentation.agentBackendLabel('strix'), '历史封存（只读）');
  for (const value of [undefined, '', 'other', 'Native', 'legacy_backend_removed_active']) {
    assert.equal(presentation.agentBackendLabel(value), '未记录');
  }
  assert.equal(presentation.agentModeLabel('deep'), '深度');
  assert.equal(presentation.agentModeLabel(undefined), '未记录');
  // Display strings must not contain the retired brand; data-compat conditions
  // may still match the old value, but never render it.
  const source = fs.readFileSync(modulePath, 'utf8');
  assert.doesNotMatch(source, /Strix（历史只读）/);
  assert.doesNotMatch(source, /Strix.*历史|历史.*Strix/);
});

test('page container only consumes isolated presentation, without historical backend literals', () => {
  const board = fs.readFileSync(boardPath, 'utf8');
  const details = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/components/SentinelExecutionDetails.vue'), 'utf8');
  assert.match(board, /from "\.\.\/features\/sentinel\/components\/SentinelExecutionDetails\.vue"/);
  assert.match(board, /:scope="agentExecutionScope"/);
  assert.match(details, /from "\.\.\/execution\/presentation"/);
  assert.match(details, /agentBackendLabel\(String\(execution\.backend\)\)/);
  assert.doesNotMatch(board, /strix/i);
});
