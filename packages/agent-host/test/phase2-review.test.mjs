import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { SkillLibrary } from '../../skills-library/src/index.mjs';

const changed = error => error.code === 'host_skill_access_changed';
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-phase2-review-'));
  const prompting = deferred(), response = deferred(), events = [];
  const runtime = { status: { pid: 191 }, async start() {}, async newSession() { return { sessionId: 'synthetic-phase2-session' }; },
    prompt() { prompting.resolve(); return response.promise; }, async cancel() { return true; },
    async close() { response.resolve({ stopReason: 'cancelled', metrics: {} }); } };
  const host = new AgentHost({ dataDir: directory, runtime, onEvent: event => events.push(event) });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Synthetic phase2 review');
  const conversation = host.store.createConversation(project.id);
  const access = await host.ensureSkills();
  async function install(name) {
    const source = join(directory, `source-${name}`);
    const base = `skills/${name}`;
    await mkdir(join(source, base, 'references'), { recursive: true });
    await writeFile(join(source, base, 'SKILL.md'), `---\nname: synthetic-${name}\ndescription: Synthetic inert guidance\n---\nUse bounded project evidence.\n`);
    await writeFile(join(source, base, 'references/alternate.txt'), 'Synthetic alternate guidance bytes');
    if (name === 'alpha') {
      await mkdir(join(source, 'skills/sibling'), { recursive: true });
      await writeFile(join(source, 'skills/sibling/SKILL.md'), '---\nname: synthetic-sibling\ndescription: Synthetic independent sibling\n---\nInert sibling guidance.\n');
    }
    const preview = await host.library.previewDirectory(source);
    return host.library.importDirectory(source, preview.digest);
  }
  const first = await install('alpha'), second = await install('beta');
  const skill = first.skills.find(value => value.name === 'synthetic-alpha');
  const shown = access.get(conversation.id);
  access.set(conversation.id, { selectedSkillIds: [skill.id], expectedRevision: shown.revision, disclosureAccepted: true, expectedRuntimeIdentity: shown.runtimeIdentity, expectedSkillRevision: shown.skillRevision });
  return { host, access, conversation, skill, first, second, prompting, response, events };
}

test('quick Skills streaming guard rejects a changed persisted entry path before emitting further text', async t => {
  const f = await fixture(t);
  const lease = f.access.capture(f.conversation.id);
  f.host.library.db.prepare('UPDATE skill_entries SET entry_path=? WHERE id=?').run('skills/alpha/references/alternate.txt', f.skill.id);
  assert.throws(() => f.access.validate(f.conversation.id, lease), changed);
  assert.throws(() => f.access.validateQuick(f.conversation.id, lease), changed);
});

for (const [field, replacement] of [
  ['entry_path', f => 'skills/alpha/references/alternate.txt'],
  ['package_id', f => f.second.id],
]) {
  test(`changed then restored Skill ${field} permanently stales the old streaming consent`, async t => {
    const f = await fixture(t);
    const lease = f.access.capture(f.conversation.id);
    const before = f.host.library.db.prepare(`SELECT ${field} AS value,generation FROM skill_entries WHERE id=?`).get(f.skill.id);
    const update = f.host.library.db.prepare(`UPDATE skill_entries SET ${field}=? WHERE id=?`);
    update.run(replacement(f), f.skill.id);
    update.run(before.value, f.skill.id);
    assert(f.host.library.db.prepare('SELECT generation FROM skill_entries WHERE id=?').get(f.skill.id).generation > before.generation);
    assert.throws(() => f.access.validate(f.conversation.id, lease), changed);
    assert.throws(() => f.access.validateQuick(f.conversation.id, lease), changed);
  });
}

test('actual AgentHost streaming withholds a delta after the selected Skill entry path changes', async t => {
  const f = await fixture(t);
  await f.host.send(f.conversation.id, { text: 'Synthetic phase2 local-only request' });
  const run = f.host.runs.get(f.conversation.id);
  await f.prompting.promise;
  f.host.library.db.prepare('UPDATE skill_entries SET entry_path=? WHERE id=?').run('skills/alpha/references/alternate.txt', f.skill.id);
  f.host.runtimeEvent({ type: 'text_delta', sessionId: run.sessionId, text: 'Synthetic text after invalidation' });
  assert.equal(run.text, '');
  assert.equal(f.events.filter(event => event.type === 'message.delta').length, 0);
  assert.equal(run.cancelled, true);
  f.response.resolve({ stopReason: 'cancelled', metrics: {} });
  await run.completion;
  assert.equal(f.host.store.messages(f.conversation.id).find(row => row.role === 'assistant').content, '');
});


test('an existing installed Skills database gains the scope trigger on reopen without revoking unchanged consent', async t => {
  const f = await fixture(t);
  const lease = f.access.capture(f.conversation.id);
  const directory = f.host.library.dataDir;
  f.host.library.db.exec('DROP TRIGGER skill_generation_entry_scope_change');
  f.host.library.close();
  f.host.library = new SkillLibrary({ dataDir: directory });
  f.access.library = f.host.library;
  assert(f.host.library.db.prepare("SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='skill_generation_entry_scope_change'").get());
  assert.equal(f.access.validateQuick(f.conversation.id, lease), true);
  const original = f.host.library.db.prepare('SELECT entry_path,generation FROM skill_entries WHERE id=?').get(f.skill.id);
  const update = f.host.library.db.prepare('UPDATE skill_entries SET entry_path=? WHERE id=?');
  update.run('skills/alpha/references/alternate.txt', f.skill.id);
  update.run(original.entry_path, f.skill.id);
  assert(f.host.library.db.prepare('SELECT generation FROM skill_entries WHERE id=?').get(f.skill.id).generation > original.generation);
  assert.throws(() => f.access.validate(f.conversation.id, lease), changed);
  assert.throws(() => f.access.validateQuick(f.conversation.id, lease), changed);
});

test('no-op and unrelated sibling entry-path changes preserve the selected Skill lease', async t => {
  const f = await fixture(t);
  const lease = f.access.capture(f.conversation.id);
  const before = f.host.library.db.prepare('SELECT entry_path,generation FROM skill_entries WHERE id=?').get(f.skill.id);
  f.host.library.db.prepare('UPDATE skill_entries SET entry_path=entry_path,package_id=package_id WHERE id=?').run(f.skill.id);
  assert.equal(f.host.library.db.prepare('SELECT generation FROM skill_entries WHERE id=?').get(f.skill.id).generation, before.generation);
  const sibling = f.first.skills.find(value => value.name === 'synthetic-sibling');
  const packageRevision = f.host.library.listPackages().find(value => value.id === f.first.id).revision;
  f.host.library.db.prepare('UPDATE skill_entries SET entry_path=? WHERE id=?').run('skills/sibling/alternate-SKILL.md', sibling.id);
  assert(f.host.library.listPackages().find(value => value.id === f.first.id).revision > packageRevision);
  assert.equal(f.access.validate(f.conversation.id, lease), true);
  assert.equal(f.access.validateQuick(f.conversation.id, lease), true);
});
