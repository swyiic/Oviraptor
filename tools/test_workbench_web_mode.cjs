const test = require('node:test');
const assert = require('node:assert/strict');
const { mount, draft } = require('./agent_workbench_harness.cjs');

test('ordinary new Web passes explicit Single and Multi through real creation controller', async (t) => {
  for (const mode of ['single', 'multi']) {
    const sent = [];
    const app = await mount({ createSentinelUrlScan: async (...args) => { sent.push(args); return draft; } });
    t.after(app.unmount);
    app.b.form.urls = 'https://mode.example.test/app';
    app.b.form.orchestrationMode = mode;
    await app.b.start(true);
    assert.equal(sent.length, 1);
    assert.equal(sent[0][11], mode);
  }
});

test('new Web UI default is explicit Multi while Source payload has no mode authority', async (t) => {
  const app = await mount();
  t.after(app.unmount);
  assert.equal(app.b.form.orchestrationMode, 'multi');
  for (const mode of ['code', 'greybox', 'cicd']) {
    const sent = [];
    const source = await mount({ startWorkbenchScan: async (input) => { sent.push(input); return draft; } }, { initialMode: mode });
    t.after(source.unmount);
    source.b.form.maxBudgetUsd = undefined;
    source.b.form.orchestrationMode = 'single';
    await source.b.start();
    assert.equal(sent.length, 1);
    assert.equal(Object.hasOwn(sent[0], 'orchestrationMode'), false);
    assert.equal(Object.hasOwn(sent[0], 'orchestration'), false);
  }
});
