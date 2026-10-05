const test = require('node:test');
const assert = require('node:assert/strict');
const { mount, flush, deferred, draft } = require('./agent_workbench_harness.cjs');

test('source modes do not create a task with an unenforceable USD ceiling', async (t) => {
  for (const mode of ['code', 'greybox', 'cicd']) {
    const submitted = [];
    const app = await mount({ startWorkbenchScan: async (input) => { submitted.push(input); return draft; } }, { initialMode: mode });
    t.after(app.unmount);
    await app.b.start();
    assert.equal(submitted.length, 0, `${mode} must not create a permanently incomplete source task`);
    assert.ok(app.events.some(([name, kind, text]) => name === 'notify' && kind === 'error' && text.includes('USD')));
    app.b.form.maxBudgetUsd = undefined;
    await app.b.start();
    assert.equal(submitted.length, 1, `${mode} proceeds only after the operator clears the cap`);
    assert.equal(submitted[0].maxBudgetUsd, undefined);
  }
});

test('late source task response cannot navigate or clear a new workspace form', async (t) => {
  const pending = deferred();
  const app = await mount({ startWorkbenchScan: () => pending.promise }, { initialMode: 'code' });
  t.after(app.unmount);
  app.b.form.taskName = 'Source A';
  app.b.form.sourcePath = '/safe/source-a';
  app.b.form.maxBudgetUsd = undefined;
  const start = app.b.start();
  await flush();
  app.b.form.projectId = 2;
  app.b.form.projectId = 1;
  app.b.form.taskName = 'Source B';
  pending.resolve({ ...draft, id: 'old-source', scanType: 'code' });
  await start;
  assert.equal(app.b.form.taskName, 'Source B');
  assert.equal(app.events.some(([event]) => event === 'open' || event === 'notify'), false);
});

test('late CI task response after unmount cannot publish a UI action', async () => {
  const pending = deferred();
  const app = await mount({ startWorkbenchScan: () => pending.promise }, { initialMode: 'cicd' });
  app.b.form.maxBudgetUsd = undefined;
  const start = app.b.start();
  await flush();
  app.unmount();
  pending.resolve({ ...draft, id: 'old-ci', scanType: 'cicd' });
  await start;
  assert.equal(app.events.some(([event]) => event === 'open' || event === 'notify'), false);
});

test('editing source task fields during a pending save preserves the new text', async (t) => {
  const pending = deferred();
  const app = await mount({ startWorkbenchScan: () => pending.promise }, { initialMode: 'greybox' });
  t.after(app.unmount);
  app.b.form.maxBudgetUsd = undefined;
  app.b.form.taskName = 'Original';
  app.b.form.instruction = 'Original instructions';
  const start = app.b.start();
  await flush();
  app.b.form.taskName = 'Next';
  app.b.form.instruction = 'Next instructions';
  pending.resolve({ ...draft, id: 'greybox-1', scanType: 'greybox' });
  await start;
  assert.equal(app.b.form.taskName, 'Next');
  assert.equal(app.b.form.instruction, 'Next instructions');
});
