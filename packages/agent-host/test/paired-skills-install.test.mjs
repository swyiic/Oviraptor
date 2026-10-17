import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { buildPairedSkillsPreview, importPairedSkills } from '../src/paired-skills.mjs';


async function hostFixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-paired-'));
  const host = new AgentHost({ dataDir: join(directory, 'host') });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Paired skills project');
  return { host, directory, project };
}

async function buildPairedPackage(directory) {
  const sourceDirectory = join(directory, 'paired-app-src');
  await mkdir(join(sourceDirectory, 'docs'), { recursive: true });
  await writeFile(join(sourceDirectory, 'docs', 'operator-guide.md'), '# Operator guide\n\nRead this before calling ping. Reference only.\n');
  await writeFile(join(sourceDirectory, 'worker.mjs'), `import { readFile } from 'node:fs/promises';
import { startStdioWorker } from './sdk/worker.mjs';
const manifest = JSON.parse(await readFile(new URL('./manifest.json', import.meta.url), 'utf8'));
await startStdioWorker({
  manifest,
  async onStart() { return { dataSchemaVersion: 1 }; },
  async onStop() { return { busy: false }; },
  tools: {
    async ping(args) { return { content: [{ type: 'text', text: args.note }], structuredContent: { ok: true, note: args.note } }; },
  },
}).closed;
`);
  await writeFile(join(sourceDirectory, 'page.html'), '<!doctype html><html><body><h1>Paired</h1></body></html>\n');
  const manifest = {
    schemaVersion: 1,
    id: 'org.oviraptor.fixture.paired.install',
    version: '1.0.0',
    hostApi: '^1.0.0',
    entry: 'worker.mjs',
    dataSchemaVersion: 1,
    pages: [{ id: 'home', title: 'Paired', path: 'page.html' }],
    permissions: [{ id: 'pair.write', description: 'write' }],
    tools: [{
      name: 'ping',
      description: 'Synthetic paired ping.',
      inputSchema: { type: 'object', properties: { note: { type: 'string', maxLength: 64 } }, required: ['note'], additionalProperties: false },
      permission: 'pair.write',
    }],
    paired_skills: [
      { name: 'paired-operator-guide', source: 'docs/operator-guide.md', required: false },
      { name: 'fixture-guide', source: 'bundled', required: false },
    ],
    lifecycle: { start: true, stop: true },
  };
  const packaged = await createDirectoryPackage({
    sourceDirectory,
    outputDirectory: join(directory, 'paired-app-package'),
    manifest,
    files: ['worker.mjs', 'page.html', 'docs/operator-guide.md'],
    vendorSdk: true,
  });
  return packaged.directory;
}

test('install returns pairedSkillsPreview with package-relative excerpt and bundled reference', async t => {
  const { host, directory } = await hostFixture(t);
  const path = await buildPairedPackage(directory);
  const installed = await host.withPairedSkillsPreview(await host.apps.install(path));
  assert.equal(installed.pairedSkills.length, 2);
  assert.ok(installed.pairedSkillsPreview);
  assert.equal(installed.pairedSkillsPreview.appId, installed.id);
  const packageSkill = installed.pairedSkillsPreview.skills.find(skill => skill.name === 'paired-operator-guide');
  const bundledSkill = installed.pairedSkillsPreview.skills.find(skill => skill.name === 'fixture-guide');
  assert.equal(packageSkill.status, 'package_reference');
  assert.equal(packageSkill.importable, true);
  assert.match(packageSkill.excerpt, /Operator guide/);
  assert.equal(typeof packageSkill.digest, 'string');
  assert.equal(packageSkill.digest.length, 64);
  assert.equal(bundledSkill.status, 'bundled_reference');
  assert.equal(bundledSkill.importable, false);
  assert.equal(bundledSkill.inSkillsReadScope, false);
});

test('one-click import brings package skill into library and skills_read; uninstall keeps the skill', async t => {
  const { host, directory, project } = await hostFixture(t);
  const path = await buildPairedPackage(directory);
  const installed = await host.apps.install(path);
  await host.apps.setEnabled(installed.id, true);
  await host.ensureSkills();
  const conversation = host.store.createConversation(project.id);

  const imported = await host.importPairedSkills(installed.id, {
    names: ['paired-operator-guide'],
    conversationId: conversation.id,
    includeInSkillsRead: true,
  });
  assert.equal(imported.imported.length, 1);
  assert.equal(imported.imported[0].name, 'paired-operator-guide');
  assert.ok(imported.imported[0].skillId >= 1);
  assert.ok(imported.conversationSelection);
  assert.deepEqual(imported.conversationSelection.selectedSkillIds, [imported.imported[0].skillId]);
  assert.equal(imported.conversationSelection.disclosureAccepted, true);

  const preview = await host.pairedSkillsPreview(installed.id);
  const packageSkill = preview.skills.find(skill => skill.name === 'paired-operator-guide');
  assert.equal(packageSkill.status, 'in_library');
  assert.equal(packageSkill.inSkillsReadScope, true);
  assert.equal(packageSkill.importable, false);

  const skillId = packageSkill.skillId;
  assert.ok(host.library.listSkills().some(skill => skill.id === skillId));

  await host.apps.uninstall(installed.id);
  assert.ok(host.library.listSkills().some(skill => skill.id === skillId), 'uninstall must not delete Skills');
  assert.equal(host.apps.list().length, 0);
});

test('bundled paired skill already in library is alreadyPresent and readable via paired scope', async t => {
  const { host, directory, project } = await hostFixture(t);
  const path = await buildPairedPackage(directory);
  const installed = await host.apps.install(path);
  await host.apps.setEnabled(installed.id, true);
  await host.ensureSkills();

  const skillDir = join(directory, 'bundled-like');
  await mkdir(skillDir, { recursive: true });
  await writeFile(join(skillDir, 'SKILL.md'), '---\nname: fixture-guide\ndescription: Synthetic fixture guidance\n---\n# Fixture guide\n');
  const skillPreview = await host.library.previewDirectory(skillDir);
  await host.library.importDirectory(skillDir, skillPreview.digest);

  const result = await host.importPairedSkills(installed.id, { names: ['fixture-guide'] });
  assert.equal(result.imported.length, 0);
  assert.equal(result.alreadyPresent.length, 1);
  assert.equal(result.alreadyPresent[0].name, 'fixture-guide');

  const preview = await host.pairedSkillsPreview(installed.id);
  assert.equal(preview.skills.find(skill => skill.name === 'fixture-guide').inSkillsReadScope, true);

  const conversation = host.store.createConversation(project.id);
  const access = await host.ensureSkills();
  const guide = host.library.listSkills().find(skill => skill.name === 'fixture-guide');
  const current = access.get(conversation.id);
  access.set(conversation.id, {
    selectedSkillIds: [guide.id],
    expectedRevision: current.revision,
    disclosureAccepted: true,
    expectedRuntimeIdentity: host.runtimeIdentity(),
    expectedSkillRevision: current.skillRevision,
  });
  const lease = access.capture(conversation.id);
  const read = access.execute(conversation.id, lease, 'skills_read', { skillId: guide.id, offset: 0, limit: 256 }, {
    pairedSkillIds: [guide.id],
  });
  assert.match(JSON.stringify(read), /Fixture guide|fixture/i);
});

test('HTTP install and enable attach pairedSkillsPreview; import route is reference-only', async t => {
  const { host, directory } = await hostFixture(t);
  const path = await buildPairedPackage(directory);
  const web = await serveHost(host, { port: 0 });
  t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };

  const installResponse = await fetch(`${web.origin}/api/apps`, { method: 'POST', headers, body: JSON.stringify({ path }) });
  assert.equal(installResponse.status, 200);
  const installed = await installResponse.json();
  assert.ok(installed.pairedSkillsPreview?.skills?.some(skill => skill.name === 'paired-operator-guide'));

  const enableResponse = await fetch(`${web.origin}/api/apps/${installed.id}/state`, {
    method: 'POST', headers, body: JSON.stringify({ enabled: true }),
  });
  assert.equal(enableResponse.status, 200);
  const enabled = await enableResponse.json();
  assert.ok(enabled.pairedSkillsPreview);

  const previewResponse = await fetch(`${web.origin}/api/apps/${installed.id}/paired-skills`, { headers });
  assert.equal(previewResponse.status, 200);
  const preview = await previewResponse.json();
  assert.equal(preview.appId, installed.id);

  const importResponse = await fetch(`${web.origin}/api/apps/${installed.id}/paired-skills/import`, {
    method: 'POST', headers, body: JSON.stringify({ names: ['paired-operator-guide'] }),
  });
  assert.equal(importResponse.status, 200);
  const imported = await importResponse.json();
  assert.equal(imported.imported.length, 1);
  assert.match(imported.note, /no tools|no execution|Uninstalling/i);
});

test('buildPairedSkillsPreview rejects missing app', async () => {
  await assert.rejects(() => buildPairedSkillsPreview({ app: null, library: null }), /host_app_unavailable/);
});

test('importPairedSkills refuses unknown names', async t => {
  const { host, directory } = await hostFixture(t);
  const path = await buildPairedPackage(directory);
  const installed = await host.apps.install(path);
  await host.ensureSkills();
  await assert.rejects(
    () => importPairedSkills({
      app: host.store.application(installed.id),
      library: host.library,
      names: ['not-declared'],
    }),
    error => error.code === 'host_paired_skill_unknown',
  );
});
