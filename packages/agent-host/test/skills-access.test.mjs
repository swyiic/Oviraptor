import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { ConversationSkills } from '../src/skills.mjs';
import { SkillLibrary } from '../../skills-library/src/index.mjs';
import { validateInputSchema, validateToolArguments } from '../../app-sdk/src/index.mjs';

const bundlePath = fileURLToPath(new URL('../../../src-tauri/resources/skill-packages/compound-engineering.json.gz', import.meta.url));
const codeOf = error => error?.code ?? String(error?.message ?? error).split(':')[0];
const accessError = error => /^host_skill_access_/u.test(codeOf(error));
const changed = error => codeOf(error) === 'host_skill_access_changed';
const inputError = error => accessError(error) || codeOf(error) === 'app_arguments_invalid';
const sorted = values => [...values].sort((a, b) => String(a).localeCompare(String(b)));

async function put(root, path, value) {
  const target = join(root, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, value);
  return target;
}
function instructions(name, body = 'Use scoped project evidence. This reference never authorizes additional tools.') {
  return `---\nname: ${name}\ndescription: Synthetic scoped guidance\n---\n${body}\n`;
}
async function fixture(t, { disk = false, install = true } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-conversation-skills-'));
  const result = { root, source: join(root, 'source'), storePath: disk ? join(root, 'host.sqlite') : ':memory:', identity: 'synthetic-runtime:1' };
  result.store = new HostStore(result.storePath);
  result.library = new SkillLibrary({ dataDir: join(root, 'library') });
  result.access = new ConversationSkills(result.store, result.library, { runtimeIdentity: () => result.identity });
  result.p1 = result.store.createProject('Synthetic project alpha');
  result.p2 = result.store.createProject('Synthetic project beta');
  result.c1 = result.store.createConversation(result.p1.id);
  result.c2 = result.store.createConversation(result.p1.id);
  result.c3 = result.store.createConversation(result.p2.id);
  t.after(async () => {
    try { result.store.close(); }
    finally { try { result.library.close(); } finally { await rm(root, { recursive: true, force: true }); } }
  });
  if (install) {
    await put(result.source, 'skills/a/SKILL.md', instructions('scope-alpha'));
    await put(result.source, 'skills/b/SKILL.md', instructions('scope-beta'));
    await put(result.source, 'skills/a/references/a.txt', 'Synthetic alpha-only reference');
    await put(result.source, 'skills/b/references/b.txt', 'Synthetic beta-only reference');
    await put(result.source, 'LICENSE', 'Synthetic shared license');
    const preview = await result.library.previewDirectory(result.source);
    result.package = await result.library.importDirectory(result.source, preview.digest);
    result.alpha = result.package.skills.find(entry => entry.name === 'scope-alpha');
    result.beta = result.package.skills.find(entry => entry.name === 'scope-beta');
  }
  return result;
}
function grant(f, conversationId = f.c1.id, selectedSkillIds = [f.alpha.id]) {
  return f.access.set(conversationId, strictConfirmation(f.access.get(conversationId), selectedSkillIds));
}
function unpack(result) {
  assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 32_768, 'complete MCP envelope exceeds32KiB');
  assert.equal(result.content.length, 1);
  assert.equal(result.content[0].type, 'text');
  const parsed = JSON.parse(result.content[0].text);
  assert.deepEqual(parsed, result.structuredContent);
  assert.equal(parsed.untrusted, true);
  assert.equal(parsed.authority, 'reference_only_no_execution');
  return parsed;
}
function assertDeepFrozen(value) {
  if (value && typeof value === 'object') {
    assert.equal(Object.isFrozen(value), true);
    for (const child of Object.values(value)) assertDeepFrozen(child);
  }
}
function rejectedExecution(f, conversationId, lease, name, args, predicate = inputError) {
  return assert.rejects(async () => f.access.execute(conversationId, lease, name, args), predicate);
}

test('new conversations expose no tools and support a validated plain-chat capture', async t => {
  const f = await fixture(t);
  const dto = f.access.get(f.c1.id);
  assert.equal(dto.conversationId, f.c1.id);
  assert.equal(dto.projectId, f.p1.id);
  assert.equal(dto.revision, 1);
  assert.deepEqual(dto.selectedSkillIds, []);
  assert.equal(dto.disclosureAccepted, false);
  assert.deepEqual(dto.unavailableSkillIds, []);
  const lease = f.access.capture(f.c1.id);
  assert.equal(lease.disclosureAccepted, false);
  assertDeepFrozen(lease);
  assert.equal(f.access.validate(f.c1.id, lease), true);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, lease), []);
  await rejectedExecution(f, f.c1.id, lease, 'skills_list', {});
  assert.deepEqual(f.access.affectedConversations(), []);
});

test('even explicit empty disclosure grants no tools or access to installed skills', async t => {
  const f = await fixture(t);
  grant(f, f.c1.id, []);
  const lease = f.access.capture(f.c1.id);
  assert.equal(f.access.validate(f.c1.id, lease), true);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, lease), []);
  await rejectedExecution(f, f.c1.id, lease, 'skills_read', { skillId: f.alpha.id });
});

test('accepted scope is conversation-specific across both same-project and different-project chats', async t => {
  const f = await fixture(t);
  const accepted = grant(f);
  assert.equal(accepted.projectId, f.p1.id);
  assert.equal(accepted.disclosureAccepted, true);
  assert.equal(accepted.skillsRequireReview, false);
  assert.deepEqual(accepted.selectedSkillIds, [f.alpha.id]);
  const lease = f.access.capture(f.c1.id);
  assert.equal(f.access.validate(f.c1.id, lease), true);
  for (const other of [f.c2, f.c3]) {
    assert.deepEqual(f.access.get(other.id).selectedSkillIds, []);
    assert.deepEqual(f.access.toolSpecs(other.id, f.access.capture(other.id)), []);
    assert.throws(() => f.access.validate(other.id, lease), changed);
    await rejectedExecution(f, other.id, lease, 'skills_read', { skillId: f.alpha.id }, changed);
  }
  const result = unpack(f.access.execute(f.c1.id, lease, 'skills_read', { skillId: f.alpha.id }));
  assert.equal(result.name, 'scope-alpha');
  assert.equal(result.content, instructions('scope-alpha'));
});

test('set rejects project overrides, unknown authority fields and missing or stale revisions without changing access', async t => {
  const f = await fixture(t);
  for (const extra of [{ projectId: f.p2.id }, { fingerprints: {} }, { runtimeIdentity: 'spoofed-runtime' }, { unknown: true }]) {
    assert.throws(() => f.access.set(f.c1.id, { ...strictConfirmation(f.access.get(f.c1.id), [f.alpha.id]), ...extra }), inputError);
  }
  for (const expectedRevision of [undefined, 0, -1, 1.5, '1', Number.NaN, Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => f.access.set(f.c1.id, { ...strictConfirmation(f.access.get(f.c1.id), [f.alpha.id]), expectedRevision }), inputError);
  }
  assert.equal(f.access.get(f.c1.id).revision, 1);
  const first = grant(f);
  assert.ok(first.revision > 1);
  assert.throws(() => f.access.set(f.c1.id, { ...strictConfirmation(f.access.get(f.c1.id), [f.beta.id]), expectedRevision: 1 }), accessError);
  assert.deepEqual(f.access.get(f.c1.id).selectedSkillIds, [f.alpha.id]);
});

test('selection rejects duplicate, nonpositive, noninteger, unavailable and more-than128 IDs atomically', async t => {
  const f = await fixture(t);
  for (const selectedSkillIds of [[f.alpha.id, f.alpha.id], [0], [-1], [1.5], ['1'], [true], [null], [Number.MAX_SAFE_INTEGER], Array.from({ length: 129 }, (_, index) => index + 1)]) {
    assert.throws(() => f.access.set(f.c1.id, strictConfirmation(f.access.get(f.c1.id), selectedSkillIds)), inputError);
    assert.deepEqual(f.access.get(f.c1.id).selectedSkillIds, []);
    assert.equal(f.access.get(f.c1.id).revision, 1);
  }
});

test('permission expansion requires explicit disclosure while disclosure=false can only retain or reduce existing selection', async t => {
  const f = await fixture(t);
  assert.throws(() => f.access.set(f.c1.id, { selectedSkillIds: [f.alpha.id], disclosureAccepted: false, expectedRevision: 1 }), accessError);
  for (const disclosureAccepted of [undefined, 'true', 1]) {
    assert.throws(() => f.access.set(f.c1.id, { ...strictConfirmation(f.access.get(f.c1.id), [f.alpha.id]), disclosureAccepted }), inputError);
  }
  grant(f);
  assert.throws(() => f.access.set(f.c1.id, { selectedSkillIds: [f.alpha.id, f.beta.id], disclosureAccepted: false, expectedRevision: f.access.get(f.c1.id).revision }), accessError);
  grant(f, f.c1.id, [f.alpha.id, f.beta.id]);
  const reduced = f.access.set(f.c1.id, { selectedSkillIds: [f.alpha.id], disclosureAccepted: false, expectedRevision: f.access.get(f.c1.id).revision });
  assert.deepEqual(reduced.selectedSkillIds, [f.alpha.id]);
  assert.equal(reduced.disclosureAccepted, false);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, f.access.capture(f.c1.id)), []);
});

test('revoke preserves selection, advances access revision and invalidates a captured accepted lease', async t => {
  const f = await fixture(t);
  grant(f, f.c1.id, [f.alpha.id, f.beta.id]);
  const lease = f.access.capture(f.c1.id);
  assert.throws(() => f.access.revoke(f.c1.id, { expectedRevision: lease.revision - 1 }), accessError);
  const revoked = f.access.revoke(f.c1.id, { expectedRevision: lease.revision });
  assert.deepEqual(sorted(revoked.selectedSkillIds), sorted([f.alpha.id, f.beta.id]));
  assert.equal(revoked.disclosureAccepted, false);
  assert.ok(revoked.revision > lease.revision);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  const plain = f.access.capture(f.c1.id);
  assert.equal(f.access.validate(f.c1.id, plain), true);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, plain), []);
});

test('revocation remains available when a previously selected skill is unavailable and never drops that selection silently', async t => {
  const f = await fixture(t);
  grant(f, f.c1.id, [f.alpha.id, f.beta.id]);
  f.library.setEnabled(f.alpha.id, false);
  const current = f.access.get(f.c1.id);
  assert.deepEqual(sorted(current.selectedSkillIds), sorted([f.alpha.id, f.beta.id]));
  assert.ok(current.unavailableSkillIds.includes(f.alpha.id));
  assert.equal(current.skillsRequireReview, true);
  const revoked = f.access.set(f.c1.id, { selectedSkillIds: current.selectedSkillIds, disclosureAccepted: false, expectedRevision: current.revision });
  assert.deepEqual(sorted(revoked.selectedSkillIds), sorted(current.selectedSkillIds));
  assert.equal(revoked.disclosureAccepted, false);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, f.access.capture(f.c1.id)), []);
});

test('captured leases are deeply immutable and only their originating access instance can authenticate them', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  assertDeepFrozen(lease);
  assert.equal(typeof lease.fingerprints, 'object');
  assert.throws(() => lease.selectedSkillIds.push(f.beta.id), TypeError);
  assert.throws(() => { lease.revision = 999; }, TypeError);
  assert.throws(() => { lease.fingerprints.synthetic = 'spoofed'; }, TypeError);
  assert.throws(() => f.access.validate(f.c1.id, structuredClone(lease)), changed);
  assert.throws(() => f.access.validate(f.c1.id, { ...lease }), changed);
  const other = new ConversationSkills(f.store, f.library, { runtimeIdentity: () => f.identity });
  assert.throws(() => other.validate(f.c1.id, lease), changed);
  assert.equal(other.validate(f.c1.id, other.capture(f.c1.id)), true);
});

test('changing the host conversation project invalidates its earlier host-bound skill lease', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  f.store.db.prepare('UPDATE conversations SET project_id=? WHERE id=?').run(f.p2.id, f.c1.id);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  await rejectedExecution(f, f.c1.id, lease, 'skills_read', { skillId: f.alpha.id }, changed);
});

test('runtime identity changes require renewed disclosure and invalidate existing leases without changing selected IDs', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  f.identity = 'synthetic-runtime:2';
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  assert.equal(f.access.get(f.c1.id).disclosureAccepted, false);
  assert.equal(f.access.get(f.c1.id).skillsRequireReview, false, 'runtime recipient changes do not mislabel the unchanged Skill content');
  assert.deepEqual(f.access.get(f.c1.id).selectedSkillIds, [f.alpha.id]);
  assert.ok(f.access.affectedConversations().includes(f.c1.id));
  const plain = f.access.capture(f.c1.id);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, plain), []);
  assert.equal(f.access.validate(f.c1.id, plain), true);
  grant(f);
  const renewed = f.access.capture(f.c1.id);
  assert.equal(renewed.runtimeIdentity, f.identity);
  assert.equal(f.access.validate(f.c1.id, renewed), true);
});

test('disabled and then re-enabled content cannot revive a previous conversation acknowledgement', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  f.library.setEnabled(f.alpha.id, false);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  assert.ok(f.access.get(f.c1.id).unavailableSkillIds.includes(f.alpha.id));
  f.library.setEnabled(f.alpha.id, true);
  assert.deepEqual(f.access.get(f.c1.id).unavailableSkillIds, []);
  assert.equal(f.access.get(f.c1.id).skillsRequireReview, true);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, f.access.capture(f.c1.id)), []);
  grant(f);
  assert.equal(f.access.validate(f.c1.id, f.access.capture(f.c1.id)), true);
});

test('uninstall/reinstall retains selected IDs but requires renewed acknowledgement for the new content generation', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  f.library.uninstallPackage(f.package.id, { expectedRevision: f.library.listPackages()[0].revision });
  const unavailable = f.access.get(f.c1.id);
  assert.deepEqual(unavailable.selectedSkillIds, [f.alpha.id]);
  assert.ok(unavailable.unavailableSkillIds.includes(f.alpha.id));
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  const preview = await f.library.previewDirectory(f.source);
  const installed = await f.library.importDirectory(f.source, preview.digest);
  assert.ok(installed.skills.some(entry => entry.id === f.alpha.id));
  assert.equal(f.access.get(f.c1.id).skillsRequireReview, true);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, f.access.capture(f.c1.id)), []);
});

test('changed and subsequently restored reference bytes permanently stale the previous disclosure snapshot', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  const original = f.library.db.prepare("SELECT content FROM skill_files WHERE package_id=? AND path='skills/a/references/a.txt'").get(f.package.id).content;
  f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'").run(Buffer.from('Synthetic intervening modification'), f.package.id);
  f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'").run(original, f.package.id);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  assert.equal(f.access.get(f.c1.id).skillsRequireReview, true);
  grant(f);
  assert.equal(f.access.validate(f.c1.id, f.access.capture(f.c1.id)), true);
});

test('affectedConversations identifies only persistently accepted stale grants and honors the optional skill filter', async t => {
  const f = await fixture(t);
  grant(f, f.c1.id, [f.alpha.id]);
  grant(f, f.c2.id, [f.beta.id]);
  grant(f, f.c3.id, [f.alpha.id]);
  assert.deepEqual(f.access.affectedConversations(), []);
  f.library.setEnabled(f.alpha.id, false);
  assert.deepEqual(sorted(f.access.affectedConversations()), sorted([f.c1.id, f.c3.id]));
  assert.deepEqual(sorted(f.access.affectedConversations(f.alpha.id)), sorted([f.c1.id, f.c3.id]));
  assert.deepEqual(f.access.affectedConversations(f.beta.id), []);
  f.access.get(f.c1.id);
  assert.deepEqual(sorted(f.access.affectedConversations()), sorted([f.c1.id, f.c3.id]), 'a DTO read must not silently discard persisted acknowledgement');
  f.access.revoke(f.c1.id, { expectedRevision: f.access.get(f.c1.id).revision });
  assert.deepEqual(f.access.affectedConversations(), [f.c3.id]);
});

test('all36 real bundled skills can be explicitly selected and enumerated in pages of at most12', async t => {
  const f = await fixture(t, { install: false });
  const bundle = await f.library.installBundled(bundlePath);
  const ids = bundle.skills.map(entry => entry.id);
  assert.equal(ids.length, 36);
  grant(f, f.c1.id, ids);
  const lease = f.access.capture(f.c1.id);
  assert.equal(lease.selectedSkillIds.length, 36);
  const seen = [];
  let offset = 0;
  for (let page = 0; page < 4; page++) {
    const data = unpack(f.access.execute(f.c1.id, lease, 'skills_list', { offset, limit: 12 }));
    assert.ok(data.skills.length <= 12);
    seen.push(...data.skills.map(entry => entry.id));
    if (!data.hasMore) break;
    assert.ok(data.nextOffset > offset);
    offset = data.nextOffset;
  }
  assert.deepEqual(sorted(seen), sorted(ids));
});

test('tool schemas are fixed, closed and bounded and never accept a model-supplied project ID', async t => {
  const f = await fixture(t);
  grant(f);
  const specs = f.access.toolSpecs(f.c1.id, f.access.capture(f.c1.id));
  assert.deepEqual(sorted(specs.map(spec => spec.name)), sorted(['skills_list', 'skills_read', 'skills_list_resources', 'skills_read_resource']));
  for (const spec of specs) {
    assert.equal(spec.inputSchema.additionalProperties, false);
    assert.ok(!Object.hasOwn(spec.inputSchema.properties, 'projectId'));
    assert.doesNotThrow(() => validateInputSchema(spec.inputSchema));
  }
  const listing = specs.find(spec => spec.name === 'skills_list');
  assert.throws(() => validateToolArguments(listing.inputSchema, { limit: 13 }), error => codeOf(error) === 'app_arguments_invalid');
  const reading = specs.find(spec => spec.name === 'skills_read_resource');
  assert.throws(() => validateToolArguments(reading.inputSchema, { skillId: f.alpha.id, path: 'SKILL.md', limit: 4097 }), error => codeOf(error) === 'app_arguments_invalid');
});

test('tool execution rejects malformed arguments, undeclared tools and unselected skills before reading content', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  let reads = 0;
  const original = f.library.readResource.bind(f.library);
  f.library.readResource = (...args) => { reads++; return original(...args); };
  for (const [name, args] of [
    ['unknown_tool', {}], ['skills_list', { projectId: f.p2.id }], ['skills_list', { limit: 13 }],
    ['skills_read', { skillId: f.beta.id }], ['skills_read', { skillId: String(f.alpha.id) }],
    ['skills_read', { skillId: f.alpha.id, projectId: f.p2.id }],
    ['skills_read', { skillId: f.alpha.id, limit: 4097 }],
    ['skills_read_resource', { skillId: f.alpha.id, path: 'skills/a/SKILL.md', offset: -1 }],
  ]) await rejectedExecution(f, f.c1.id, lease, name, args);
  assert.equal(reads, 0);
});

test('resource tools preserve the selected skill subtree and reject sibling and traversal reads', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  const resources = unpack(f.access.execute(f.c1.id, lease, 'skills_list_resources', { skillId: f.alpha.id }));
  assert.ok(resources.resources.some(row => row.path === 'skills/a/SKILL.md'));
  assert.ok(resources.resources.some(row => row.path === 'LICENSE'));
  assert.ok(resources.resources.every(row => !row.path.startsWith('skills/b/')));
  const reference = unpack(f.access.execute(f.c1.id, lease, 'skills_read_resource', { skillId: f.alpha.id, path: 'skills/a/references/a.txt' }));
  assert.equal(reference.content, 'Synthetic alpha-only reference');
  for (const path of ['skills/b/SKILL.md', '../SKILL.md', '/SKILL.md', 'skills/a/../b/SKILL.md']) {
    await rejectedExecution(f, f.c1.id, lease, 'skills_read_resource', { skillId: f.alpha.id, path }, error => inputError(error) || /^skill_package_/u.test(codeOf(error)));
  }
});

test('resource metadata is paged without silently omitting entries beyond the12-item tool limit', async t => {
  const f = await fixture(t, { install: false });
  await put(f.source, 'SKILL.md', instructions('resource-pagination'));
  for (let index = 0; index < 29; index++) await put(f.source, `references/${String(index).padStart(2, '0')}.txt`, `Synthetic reference ${index}`);
  const preview = await f.library.previewDirectory(f.source);
  const saved = await f.library.importDirectory(f.source, preview.digest);
  grant(f, f.c1.id, [saved.skills[0].id]);
  const lease = f.access.capture(f.c1.id);
  const seen = [];
  let offset = 0;
  for (let page = 0; page < 5; page++) {
    const data = unpack(f.access.execute(f.c1.id, lease, 'skills_list_resources', { skillId: saved.skills[0].id, offset, limit: 12 }));
    assert.ok(data.resources.length <= 12);
    seen.push(...data.resources.map(row => row.path));
    if (!data.hasMore) break;
    assert.ok(data.nextOffset > offset);
    offset = data.nextOffset;
  }
  assert.equal(seen.length, 30);
  assert.equal(new Set(seen).size, 30);
});

for (const [label, mutation] of [
  ['revoke', f => f.access.revoke(f.c1.id, { expectedRevision: f.access.get(f.c1.id).revision })],
  ['runtime identity change', f => { f.identity = 'synthetic-runtime:2'; }],
  ['skill disable', f => f.library.setEnabled(f.alpha.id, false)],
  ['skill disable and re-enable', f => { f.library.setEnabled(f.alpha.id, false); f.library.setEnabled(f.alpha.id, true); }],
  ['package uninstall', f => f.library.uninstallPackage(f.package.id, { expectedRevision: f.library.listPackages()[0].revision })],
]) {
  test(`a completed resource read is withheld after an in-read ${label}`, async t => {
    const f = await fixture(t);
    grant(f);
    const lease = f.access.capture(f.c1.id);
    const original = f.library.readResource.bind(f.library);
    let invoked = false;
    f.library.readResource = (...args) => {
      const result = original(...args);
      invoked = true;
      mutation(f);
      return result;
    };
    await rejectedExecution(f, f.c1.id, lease, 'skills_read', { skillId: f.alpha.id }, changed);
    assert.equal(invoked, true);
  });
}

test('a completed directory listing is withheld if its selected generation changes during the library read', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  const original = f.library.listResources.bind(f.library);
  f.library.listResources = (...args) => {
    const result = original(...args);
    f.library.setEnabled(f.alpha.id, false);
    return result;
  };
  await rejectedExecution(f, f.c1.id, lease, 'skills_list_resources', { skillId: f.alpha.id }, changed);
});

for (const [label, value] of [
  ['UTF8 multibyte', '界😀'.repeat(3000)],
  ['JSON control characters', '\u0001'.repeat(7000)],
]) {
  test(`${label} pages preserve every byte while bounding content4096 and the complete MCP envelope32768`, async t => {
    const f = await fixture(t, { install: false });
    await put(f.source, 'SKILL.md', instructions('bounded-content'));
    await put(f.source, 'references/large.txt', value);
    const preview = await f.library.previewDirectory(f.source);
    const saved = await f.library.importDirectory(f.source, preview.digest);
    const id = saved.skills[0].id;
    grant(f, f.c1.id, [id]);
    const lease = f.access.capture(f.c1.id);
    let offset = 0, rebuilt = '';
    for (let page = 0; page < 32; page++) {
      const data = unpack(f.access.execute(f.c1.id, lease, 'skills_read_resource', { skillId: id, path: 'references/large.txt', offset, limit: 4096 }));
      assert.equal(data.offset, offset);
      const bytes = Buffer.byteLength(data.content);
      assert.ok(bytes <= 4096 && bytes > 0);
      assert.ok(!data.content.includes('\uFFFD'));
      rebuilt += data.content;
      if (!data.hasMore) { assert.equal(data.nextOffset, null); break; }
      assert.equal(data.nextOffset, offset + bytes, 'adaptive envelope truncation must not skip undisclosed bytes');
      offset = data.nextOffset;
    }
    assert.equal(rebuilt, value);
  });
}

test('long catalog metadata is bounded in a complete skill-list envelope without expanding tool authority', async t => {
  const f = await fixture(t, { install: false });
  for (let index = 0; index < 12; index++) {
    await put(f.source, `skills/${index}/SKILL.md`, `---\nname: skill-${index}\ndescription: ${'界'.repeat(2600)}\n---\nSynthetic bounded metadata.\n`);
  }
  const preview = await f.library.previewDirectory(f.source);
  const saved = await f.library.importDirectory(f.source, preview.digest);
  grant(f, f.c1.id, saved.skills.map(entry => entry.id));
  const lease = f.access.capture(f.c1.id);
  const data = unpack(f.access.execute(f.c1.id, lease, 'skills_list', { limit: 12 }));
  assert.equal(data.skills.length, 12);
  for (const entry of data.skills) {
    assert.equal(typeof entry.name, 'string');
    assert.ok(!JSON.stringify(entry).includes('\uFFFD'));
  }
});

test('persisted selected IDs and disclosure survive HostStore reopening but a lease from another instance does not', async t => {
  const f = await fixture(t, { disk: true });
  const accepted = grant(f);
  const lease = f.access.capture(f.c1.id);
  f.store.close();
  f.store = new HostStore(f.storePath);
  f.access = new ConversationSkills(f.store, f.library, { runtimeIdentity: () => f.identity });
  const restored = f.access.get(f.c1.id);
  assert.equal(restored.revision, accepted.revision);
  assert.deepEqual(restored.selectedSkillIds, [f.alpha.id]);
  assert.equal(restored.disclosureAccepted, true);
  assert.equal(restored.skillsRequireReview, false);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  const fresh = f.access.capture(f.c1.id);
  assert.equal(f.access.validate(f.c1.id, fresh), true);
  assert.equal(f.access.toolSpecs(f.c1.id, fresh).length, 4);
});

for (const owner of ['store', 'library']) {
  test(`access reads and captures refuse an outer ${owner} transaction instead of validating a stale snapshot`, async t => {
    const f = await fixture(t);
    grant(f);
    const lease = f.access.capture(f.c1.id);
    f[owner].db.exec('BEGIN');
    try {
      assert.throws(() => f.access.get(f.c1.id), accessError);
      assert.throws(() => f.access.capture(f.c1.id), accessError);
      assert.throws(() => f.access.validate(f.c1.id, lease), accessError);
      await rejectedExecution(f, f.c1.id, lease, 'skills_read', { skillId: f.alpha.id }, accessError);
    } finally { f[owner].db.exec('ROLLBACK'); }
    assert.equal(f.access.validate(f.c1.id, lease), true);
  });
}

test('revoked in-flight captures remain discoverable for cancellation until the matching lease is released', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  f.access.revoke(f.c1.id, { expectedRevision: lease.revision });
  assert.deepEqual(f.access.affectedConversations(f.alpha.id), [f.c1.id]);
  f.access.release(f.c1.id, structuredClone(lease));
  assert.deepEqual(f.access.affectedConversations(f.alpha.id), [f.c1.id]);
  f.access.release(f.c1.id, lease);
  assert.deepEqual(f.access.affectedConversations(f.alpha.id), []);
});

test('listing Skills cannot leak frontmatter metadata when the full instructions contain a synthetic credential assignment', async t => {
  const f = await fixture(t, { install: false });
  await put(f.source, 'SKILL.md', '---\nname: credential-metadata\ndescription: Synthetic api_key = "synthetic-example-only"\n---\nRead scoped references.\n');
  const preview = await f.library.previewDirectory(f.source);
  const saved = await f.library.importDirectory(f.source, preview.digest);
  grant(f, f.c1.id, [saved.skills[0].id]);
  const lease = f.access.capture(f.c1.id);
  await rejectedExecution(f, f.c1.id, lease, 'skills_list', {}, error => codeOf(error) === 'skill_package_credential_content_denied');
});

function strictConfirmation(dto, selectedSkillIds) {
  return { selectedSkillIds, expectedRevision: dto.revision, disclosureAccepted: true,
    expectedRuntimeIdentity: dto.runtimeIdentity, expectedSkillRevision: dto.skillRevision };
}

// These scenarios first require a fresh, correctly scoped handshake to succeed.
// A pre-handshake implementation must fail this positive control, rather than
// accidentally pass the stale-token checks merely by rejecting unknown fields.
test('strict confirmation refuses the displayed runtime identity after the recipient changes without persisting access', async t => {
  const f = await fixture(t);
  f.access.set(f.c2.id, strictConfirmation(f.access.get(f.c2.id), [f.alpha.id]));
  const shown = f.access.get(f.c1.id);
  f.identity = 'synthetic-runtime:2';
  assert.throws(() => f.access.set(f.c1.id, strictConfirmation(shown, [f.alpha.id])), accessError);
  const current = f.access.get(f.c1.id);
  assert.equal(current.revision, shown.revision);
  assert.deepEqual(current.selectedSkillIds, []);
  assert.equal(current.disclosureAccepted, false);
});

test('strict confirmation refuses an old catalog token after resource content changes and its exact bytes are restored', async t => {
  const f = await fixture(t);
  f.access.set(f.c2.id, strictConfirmation(f.access.get(f.c2.id), [f.alpha.id]));
  const shown = f.access.get(f.c1.id);
  const original = f.library.db.prepare("SELECT content FROM skill_files WHERE package_id=? AND path='skills/a/references/a.txt'").get(f.package.id).content;
  f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'").run(Buffer.from('Synthetic intervening handshake change'), f.package.id);
  f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'").run(original, f.package.id);
  assert.throws(() => f.access.set(f.c1.id, strictConfirmation(shown, [f.alpha.id])), accessError);
  const current = f.access.get(f.c1.id);
  assert.notEqual(current.skillRevision, shown.skillRevision);
  assert.equal(current.revision, shown.revision);
  assert.deepEqual(current.selectedSkillIds, []);
});

test('strict confirmation requires both runtime and catalog handshake fields even when the access revision is current', async t => {
  const f = await fixture(t);
  f.access.set(f.c2.id, strictConfirmation(f.access.get(f.c2.id), [f.alpha.id]));
  const shown = f.access.get(f.c1.id);
  for (const missing of [['expectedRuntimeIdentity'], ['expectedSkillRevision'], ['expectedRuntimeIdentity', 'expectedSkillRevision']]) {
    const input = strictConfirmation(shown, [f.alpha.id]);
    for (const field of missing) delete input[field];
    assert.throws(() => f.access.set(f.c1.id, input), accessError);
    assert.equal(f.access.get(f.c1.id).revision, shown.revision);
    assert.deepEqual(f.access.get(f.c1.id).selectedSkillIds, []);
  }
});

test('catalog handshake is a stable64hex metadata snapshot without reading whole bundled resource bytes', async t => {
  const f = await fixture(t, { install: false });
  await f.library.installBundled(bundlePath);
  f.library.fingerprint = () => { throw new Error('an unselected catalog snapshot must not fingerprint every Skill'); };
  f.library.readResource = () => { throw new Error('catalog handshake must not disclose resource content'); };
  const originalPrepare = f.library.db.prepare.bind(f.library.db);
  f.library.db.prepare = sql => {
    if (/^\s*SELECT\b/iu.test(sql) && /\bskill_files\b/iu.test(sql)
      && (/\bcontent\b/iu.test(sql) || /SELECT\s+\*/iu.test(sql))) {
      throw new Error('catalog snapshot must not materialize resource BLOBs');
    }
    return originalPrepare(sql);
  };
  const first = f.access.get(f.c1.id);
  assert.match(first.skillRevision, /^[a-f0-9]{64}$/u);
  assert.equal(f.access.get(f.c1.id).skillRevision, first.skillRevision);
  assert.equal(f.access.get(f.c2.id).skillRevision, first.skillRevision);
});

test('unrelated selectable catalog changes require refreshed confirmation tokens without revoking unchanged selected content', async t => {
  const f = await fixture(t);
  grant(f);
  const shown = f.access.get(f.c1.id);
  const lease = f.access.capture(f.c1.id);
  f.library.db.prepare('UPDATE skill_entries SET description=? WHERE id=?').run('Synthetic amended unrelated description', f.beta.id);
  assert.throws(() => f.access.set(f.c1.id, strictConfirmation(shown, [f.alpha.id])), accessError);
  const current = f.access.get(f.c1.id);
  assert.notEqual(current.skillRevision, shown.skillRevision);
  assert.equal(current.revision, shown.revision);
  assert.equal(current.disclosureAccepted, true);
  assert.equal(f.access.validate(f.c1.id, lease), true);
  grant(f);
  assert.equal(f.access.get(f.c1.id).disclosureAccepted, true);
});

test('false disclosure can revoke with stale displayed runtime and catalog tokens using only the current access revision', async t => {
  const f = await fixture(t);
  grant(f);
  const shown = f.access.get(f.c1.id);
  f.identity = 'synthetic-runtime:2';
  f.library.setEnabled(f.beta.id, false);
  const revoked = f.access.set(f.c1.id, { ...strictConfirmation(shown, [f.alpha.id]), disclosureAccepted: false });
  assert.equal(revoked.disclosureAccepted, false);
  assert.deepEqual(revoked.selectedSkillIds, [f.alpha.id]);
  assert.ok(revoked.revision > shown.revision);
  assert.deepEqual(f.access.toolSpecs(f.c1.id, f.access.capture(f.c1.id)), []);
});

test('catalog tokens bind package provenance and persistent generations even after source and license are restored', async t => {
  const f = await fixture(t);
  let previous = f.access.get(f.c1.id).skillRevision;
  assert.match(previous, /^[a-f0-9]{64}$/u);
  for (const [column, original, replacement] of [
    ['source', f.package.source, 'Synthetic changed catalog source'],
    ['license', f.package.license, 'Synthetic changed catalog license'],
    ['version', f.package.version, 'synthetic-version-2'],
  ]) {
    f.library.db.prepare(`UPDATE skill_packages SET ${column}=? WHERE id=?`).run(replacement, f.package.id);
    const changedToken = f.access.get(f.c1.id).skillRevision;
    assert.notEqual(changedToken, previous);
    f.library.db.prepare(`UPDATE skill_packages SET ${column}=? WHERE id=?`).run(original, f.package.id);
    const restoredToken = f.access.get(f.c1.id).skillRevision;
    assert.notEqual(restoredToken, changedToken);
    assert.notEqual(restoredToken, previous, 'restoring metadata does not roll back persistent generations');
    previous = restoredToken;
  }
});

test('confirmation rechecks the catalog after selected fingerprints are read and rolls back an intervening catalog mutation', async t => {
  const f = await fixture(t);
  const shown = f.access.get(f.c1.id);
  const originalFingerprint = f.library.fingerprint.bind(f.library);
  let changedDuringCheck = false;
  f.library.fingerprint = id => {
    const fingerprint = originalFingerprint(id);
    if (!changedDuringCheck) {
      changedDuringCheck = true;
      f.library.db.prepare('UPDATE skill_entries SET description=? WHERE id=?').run('Synthetic concurrent catalog modification', f.beta.id);
    }
    return fingerprint;
  };
  assert.throws(() => f.access.set(f.c1.id, strictConfirmation(shown, [f.alpha.id])), accessError);
  assert.equal(changedDuringCheck, true);
  assert.equal(f.access.get(f.c1.id).revision, shown.revision);
  assert.deepEqual(f.access.get(f.c1.id).selectedSkillIds, []);
});

for (const [label, mutate] of [
  ['resource bytes change', f => {
    f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'").run(Buffer.from('Synthetic streaming resource amendment'), f.package.id);
  }],
  ['resource bytes change and exact restoration', f => {
    const sql = "SELECT content FROM skill_files WHERE package_id=? AND path='skills/a/references/a.txt'";
    const original = f.library.db.prepare(sql).get(f.package.id).content;
    const update = f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'");
    update.run(Buffer.from('Synthetic intervening streaming resource'), f.package.id);
    update.run(original, f.package.id);
  }],
  ['instruction text change and exact restoration', f => {
    const original = f.library.db.prepare('SELECT instructions FROM skill_entries WHERE id=?').get(f.alpha.id).instructions;
    const update = f.library.db.prepare('UPDATE skill_entries SET instructions=? WHERE id=?');
    update.run('Synthetic intervening streaming guidance', f.alpha.id);
    update.run(original, f.alpha.id);
  }],
  ['disable and re-enable', f => { f.library.setEnabled(f.alpha.id, false); f.library.setEnabled(f.alpha.id, true); }],
  ['package source change and restoration', f => {
    const update = f.library.db.prepare('UPDATE skill_packages SET source=? WHERE id=?');
    update.run('Synthetic intervening streaming source', f.package.id);
    update.run(f.package.source, f.package.id);
  }],
  ['package license change and restoration', f => {
    const update = f.library.db.prepare('UPDATE skill_packages SET license=? WHERE id=?');
    update.run('Synthetic intervening streaming license', f.package.id);
    update.run(f.package.license, f.package.id);
  }],
]) {
  test(`quick streaming validation rejects ${label} even when its previous visible content returns`, async t => {
    const f = await fixture(t);
    grant(f);
    const lease = f.access.capture(f.c1.id);
    assert.equal(f.access.validateQuick(f.c1.id, lease), true);
    mutate(f);
    assert.throws(() => f.access.validateQuick(f.c1.id, lease), changed);
  });
}

for (const [label, mutate] of [
  ['access revision and selection', f => { grant(f, f.c1.id, [f.beta.id]); }],
  ['explicit revoke', f => { f.access.revoke(f.c1.id, { expectedRevision: f.access.get(f.c1.id).revision }); }],
  ['runtime identity', f => { f.identity = 'synthetic-runtime:2'; }],
  ['host project scope', f => { f.store.db.prepare('UPDATE conversations SET project_id=? WHERE id=?').run(f.p2.id, f.c1.id); }],
  ['persisted acknowledgement without a revision change', f => {
    f.store.db.prepare('UPDATE conversation_skill_access SET disclosure_accepted=0 WHERE conversation_id=?').run(f.c1.id);
  }],
  ['persisted bound recipient without a revision change', f => {
    f.store.db.prepare('UPDATE conversation_skill_access SET runtime_identity=? WHERE conversation_id=?').run('synthetic-different-recipient', f.c1.id);
  }],
]) {
  test(`quick streaming validation independently fences changes to ${label}`, async t => {
    const f = await fixture(t);
    grant(f);
    const lease = f.access.capture(f.c1.id);
    mutate(f);
    assert.throws(() => f.access.validateQuick(f.c1.id, lease), changed);
  });
}

test('quick validation preserves an unrelated sibling resource change even when the shared package revision advances', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  const revision = f.library.listPackages()[0].revision;
  const original = f.library.db.prepare("SELECT content FROM skill_files WHERE package_id=? AND path='skills/b/references/b.txt'").get(f.package.id).content;
  const update = f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/b/references/b.txt'");
  update.run(Buffer.from('Synthetic unrelated sibling modification'), f.package.id);
  update.run(original, f.package.id);
  assert.ok(f.library.listPackages()[0].revision > revision);
  assert.equal(f.access.validateQuick(f.c1.id, lease), true);
  assert.equal(f.access.validate(f.c1.id, lease), true);
});

function forbidLibraryPayloadReads(f) {
  for (const name of ['fingerprint', 'listPackages', 'listSkills', 'readResource']) {
    f.library[name] = () => { throw new Error(`quick validation must not call library.${name}`); };
  }
  const originalPrepare = f.library.db.prepare.bind(f.library.db);
  let metadataQueries = 0;
  f.library.db.prepare = sql => {
    if (/^\s*SELECT\b/iu.test(sql)) {
      assert.ok(!/\b(?:content|instructions|skill_files)\b/iu.test(sql), 'quick validation must not select resources or instruction payloads');
      assert.ok(!(/\bskill_entries\b/iu.test(sql) && /\*/u.test(sql)), 'SELECT * on entries would also read instructions');
      metadataQueries++;
    }
    return originalPrepare(sql);
  };
  return () => metadataQueries;
}

test('captured quick stamps support64 streaming checks using exactly one selected-metadata query each and no library payload API', async t => {
  const f = await fixture(t);
  grant(f, f.c1.id, [f.alpha.id, f.beta.id]);
  const lease = f.access.capture(f.c1.id);
  assert.ok(Object.hasOwn(lease, 'quickStamp'));
  assertDeepFrozen(lease);
  const queryCount = forbidLibraryPayloadReads(f);
  for (let index = 0; index < 64; index++) assert.equal(f.access.validateQuick(f.c1.id, lease), true);
  assert.equal(queryCount(), 64);
  assert.throws(() => f.access.validateQuick(f.c1.id, structuredClone(lease)), changed);
  assert.throws(() => f.access.validateQuick(f.c2.id, lease), changed);
});

test('plain unconsented streaming checks require no Skill payload access even with a retained selection', async t => {
  const f = await fixture(t);
  grant(f);
  f.access.revoke(f.c1.id, { expectedRevision: f.access.get(f.c1.id).revision });
  const lease = f.access.capture(f.c1.id);
  assert.equal(lease.disclosureAccepted, false);
  assert.deepEqual(lease.selectedSkillIds, [f.alpha.id]);
  const queryCount = forbidLibraryPayloadReads(f);
  assert.equal(f.access.validateQuick(f.c1.id, lease), true);
  assert.equal(queryCount(), 0);
});

test('full validation still fingerprints actual resources and rejects direct SQLite byte corruption', async t => {
  const f = await fixture(t);
  grant(f);
  const lease = f.access.capture(f.c1.id);
  const originalFingerprint = f.library.fingerprint.bind(f.library);
  let fingerprintReads = 0;
  f.library.fingerprint = id => { fingerprintReads++; return originalFingerprint(id); };
  assert.equal(f.access.validate(f.c1.id, lease), true);
  assert.ok(fingerprintReads > 0, 'the full path must independently verify the actual reviewed resource snapshot');
  fingerprintReads = 0;
  f.library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='skills/a/references/a.txt'").run(Buffer.from('Synthetic corrupt SQLite bytes'), f.package.id);
  assert.throws(() => f.access.validate(f.c1.id, lease), changed);
  assert.ok(fingerprintReads > 0, 'corruption must still be checked by the full resource fingerprint path');
});
