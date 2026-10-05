const test = require('node:test');
const assert = require('node:assert/strict');
const { mount } = require('./agent_workbench_harness.cjs');

const builtin = { id: 1, name: 'Builtin', description: 'Base', instructions: '## Scope\nBase', enabled: true, builtin: true };
const custom = { id: 2, name: 'Custom', description: 'Local', instructions: '## Scope\nLocal', enabled: true, builtin: false };

test('skills remain shared with task selection; saving refreshes and clears previous selection', async (t) => {
  const saved = [];
  const app = await mount({
    listAgentSkills: async () => [builtin, custom],
    saveAgentSkill: async (input) => { saved.push(input); },
  });
  t.after(app.unmount);
  const { b } = app;
  assert.deepEqual(b.skills.value.map((skill) => skill.id), [1, 2]);
  b.form.skillIds = [2];
  b.skillCatalog.editSkill(custom);
  b.skillCatalog.skillForm.instructions = '## Scope\nNew\n## Output\nEvidence';
  assert.deepEqual(b.skillCatalog.skillPreview.value.map((section) => section.heading), ['Scope', 'Output']);
  await b.skillCatalog.saveSkill();
  assert.equal(saved.length, 1);
  assert.equal(saved[0].id, 2);
  assert.deepEqual(b.form.skillIds, []);
  assert.equal(b.skillCatalog.showSkillEditor.value, false);
});

test('built-in skill cloning never overwrites its original id', async (t) => {
  let saved;
  const app = await mount({ saveAgentSkill: async (input) => { saved = input; } });
  t.after(app.unmount);
  app.b.skillCatalog.cloneBuiltinSkill(builtin);
  await app.b.skillCatalog.saveSkill();
  assert.equal(saved.id, undefined);
  assert.match(saved.name, /自定义增强版/);
  assert.equal(saved.instructions, builtin.instructions);
});

test('importing JSON and internal knowledge uses the selected path and refreshes one catalog', async (t) => {
  const paths = [];
  const app = await mount({
    importAgentSkills: async (path) => { paths.push(path); return 2; },
    importSecSkillKnowledge: async (path) => { paths.push(path); return { filesScanned: 3 }; },
    listAgentSkills: async () => [custom],
  }, {}, { open: async (options) => options.directory ? '/safe/sec_skills' : '/safe/skills.json' });
  t.after(app.unmount);
  await app.b.skillCatalog.importSkills();
  await app.b.skillCatalog.importInternalSecSkills();
  assert.deepEqual(paths, ['/safe/skills.json', '/safe/sec_skills']);
  assert.deepEqual(app.b.skills.value.map((skill) => skill.id), [2]);
});
