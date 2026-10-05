const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');

const source = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/fuse/useFuseDisposition.ts'), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const disposition = {};
new Function('require', 'exports', compiled)(require, disposition);

test('saved disposition submits exactly the edited entry and refreshes the queue', async () => {
  const calls = [], notifications = [];
  const state = disposition.useFuseDisposition({
    save: async (input) => calls.push(['save', input]),
    remove: async () => { throw Error('unexpected remove'); },
    reload: async () => calls.push(['reload']),
    notify: (...args) => notifications.push(args),
  });
  state.editFuse({ id: 4, verdict: 'needs_followup', note: 'approval', evidence: 'receipt', archived: false });
  await state.saveFuse(true);
  assert.deepEqual(calls, [['save', { id: 4, verdict: 'needs_followup', note: 'approval',
    evidence: 'receipt', archived: true }], ['reload']]);
  assert.equal(state.fuseEditor.value, undefined);
  assert.equal(state.fuseBusy.value, false);
  assert.match(notifications[0][1], /归档/);
});

test('failed removal remains pending; successful removal creates the returned retry task', async () => {
  const notifications = [], calls = [];
  let offline = true;
  const state = disposition.useFuseDisposition({
    save: async () => {},
    remove: async (id) => { calls.push(id); if (offline) throw Error('offline'); return { id: 'retry-7' }; },
    reload: async () => calls.push('reload'),
    notify: (...args) => notifications.push(args),
  });
  state.pendingFuseRemoval.value = { id: 7 };
  await state.removeFuse();
  assert.equal(state.pendingFuseRemoval.value.id, 7);
  assert.equal(state.fuseBusy.value, false);
  assert.match(notifications[0][1], /offline/);
  offline = false;
  await state.removeFuse();
  assert.equal(state.pendingFuseRemoval.value, undefined);
  assert.deepEqual(calls, [7, 7, 'reload']);
  assert.match(notifications[1][1], /retry-7/);
});
