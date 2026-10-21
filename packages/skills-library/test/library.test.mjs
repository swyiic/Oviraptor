import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { access, chmod, mkdir, mkdtemp, open, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync, gzipSync } from 'node:zlib';
import { SkillLibrary } from '../src/index.mjs';

const bundlePath = fileURLToPath(new URL('../../../src-tauri/resources/skill-packages/compound-engineering.json.gz', import.meta.url));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const codeOf = error => error?.code ?? String(error?.message ?? error).split(':')[0];
const hasCode = expected => error => expected instanceof RegExp
  ? expected.test(codeOf(error))
  : codeOf(error) === expected;

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-skill-library-'));
  const result = { root, dataDir: join(root, 'library'), source: join(root, 'source') };
  await mkdir(result.source);
  result.library = new SkillLibrary({ dataDir: result.dataDir });
  t.after(async () => {
    try { result.library.close(); }
    finally { await rm(root, { recursive: true, force: true }); }
  });
  return result;
}

async function put(root, path, bytes) {
  const target = join(root, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes);
  return target;
}

function instructions(name, body = 'Read selected project evidence; ask before expanding permissions.') {
  return `---\nname: ${name}\ndescription: Synthetic library regression fixture\n---\n${body}\n`;
}

async function skill(root, path = 'SKILL.md', name = 'synthetic-skill') {
  return put(root, path, instructions(name));
}

async function installDirectory(library, source) {
  const preview = await library.previewDirectory(source);
  return library.importDirectory(source, preview.digest);
}

function compatibleDigest(metadata, files) {
  // Independent contract oracle: the published Rust snapshot hash format.
  const hash = createHash('sha256');
  for (const key of ['packageKey', 'name', 'version', 'source', 'license']) {
    hash.update(metadata[key]); hash.update('\0');
  }
  for (const file of [...files].sort((a, b) => Buffer.compare(Buffer.from(a.path), Buffer.from(b.path)))) {
    hash.update(file.path); hash.update('\0'); hash.update(file.sha256); hash.update('\0');
  }
  return hash.digest('hex');
}

async function manifest() {
  return JSON.parse(gunzipSync(await readFile(bundlePath)));
}

function rowCount(library, table, packageId) {
  return Number(library.db.prepare(`SELECT COUNT(*) AS count FROM ${table} WHERE package_id=?`).get(packageId).count);
}

function assertChangedOrRejected(library, id, previous) {
  let current;
  try { current = library.fingerprint(id); }
  catch (error) {
    assert.match(codeOf(error), /^skill_package_/);
    return;
  }
  assert.notEqual(current, previous, 'a previous grant fingerprint must not survive changed reviewed data');
}

test('new library owns its SQLite catalog without initializing application business tables', async t => {
  const { library } = await fixture(t);
  assert.deepEqual(library.listPackages(), []);
  assert.deepEqual(library.listSkills(), []);
  const tables = library.db.prepare("SELECT name FROM sqlite_schema WHERE type='table'").all().map(row => row.name);
  for (const expected of ['skill_packages', 'skill_entries', 'skill_files']) assert.ok(tables.includes(expected));
  for (const domain of ['projects', 'assets', 'sentinel_scans', 'agent_runs', 'agent_skills', 'conversations', 'conversation_grants']) {
    assert.ok(!tables.includes(domain), `Skills Library must not own ${domain}`);
  }
});

test('real pinned bundle preserves all36 skills,443 files, provenance and Rust-compatible digest', async t => {
  const { library } = await fixture(t);
  const upstream = await manifest();
  const saved = await library.installBundled(bundlePath);
  assert.equal(saved.id, 'every-compound-engineering');
  assert.equal(saved.packageKey, 'every-compound-engineering');
  assert.equal(saved.name, 'Compound Engineering');
  assert.equal(saved.source, 'https://github.com/EveryInc/compound-engineering-plugin');
  assert.equal(saved.version, '50532b00de523980010d97fe950965b35382a3b7');
  assert.equal(saved.license, 'MIT');
  assert.equal(saved.builtin, true);
  assert.equal(saved.skillCount, 36);
  assert.equal(saved.fileCount, 443);
  assert.equal(saved.digest, compatibleDigest(upstream, upstream.files));
  assert.equal(saved.totalBytes, upstream.files.reduce((size, file) => size + Buffer.from(file.contentBase64, 'base64').length, 0));
  assert.equal(library.listPackages().length, 1);
  assert.equal(library.listSkills().length, 36);
  assert.equal(saved.skills.length, 36);
  assert.ok(Number.isSafeInteger(saved.revision) && saved.revision > 0);
  for (const entry of saved.skills) {
    assert.equal(entry.packageId, saved.id);
    assert.ok(Number.isSafeInteger(entry.id) && entry.id > 0);
    assert.ok(Number.isSafeInteger(entry.generation) && entry.generation > 0);
    assert.equal(entry.enabled, true);
  }
  assert.match(JSON.stringify(saved.missingCapabilities), /inert|not installed|not authorized/i);
  const license = library.readResource(saved.skills[0].id, 'LICENSE');
  assert.match(license.content, /MIT License/);
  assert.equal(license.content, Buffer.from(upstream.files.find(file => file.path === 'LICENSE').contentBase64, 'base64').toString('utf8'));
});

test('every one of the443 bundled files retains its exact upstream bytes and SHA256, including shared resources', async t => {
  const { library } = await fixture(t);
  const upstream = await manifest();
  const saved = await library.installBundled(bundlePath);
  const rows = library.db.prepare('SELECT path,content,size_bytes,sha256 FROM skill_files WHERE package_id=? ORDER BY path').all(saved.id);
  assert.equal(rows.length, 443);
  const byPath = new Map(rows.map(row => [row.path, row]));
  assert.ok(byPath.has('skills/CODING_STANDARDS.md'));
  for (const file of upstream.files) {
    const bytes = Buffer.from(file.contentBase64, 'base64');
    const actual = byPath.get(file.path);
    assert.ok(actual, `missing retained resource: ${file.path}`);
    assert.equal(file.sha256, sha256(bytes));
    assert.equal(actual.sha256, file.sha256, file.path);
    assert.equal(Number(actual.size_bytes), bytes.length, file.path);
    assert.deepEqual(Buffer.from(actual.content), bytes, file.path);
  }
  for (const entry of saved.skills) {
    const row = library.db.prepare('SELECT instructions,entry_path FROM skill_entries WHERE id=?').get(entry.id);
    assert.equal(row.entry_path, entry.entryPath);
    assert.equal(row.instructions, Buffer.from(byPath.get(entry.entryPath).content).toString('utf8'));
  }
});

test('local preview/import retains instructions, legal notices and inert scripts without granting executable capabilities', async t => {
  const { library, source, root } = await fixture(t);
  await skill(source, 'SKILL.md', 'local-inert');
  await put(source, 'LICENSE', 'Synthetic permissive license\n');
  await put(source, 'NOTICE', 'Synthetic source provenance\n');
  await put(source, 'references/guide.md', '# Synthetic reference\nUse explicit authorization.\n');
  const marker = join(root, 'script-must-not-run');
  const script = `import { writeFileSync } from 'node:fs';\nwriteFileSync(${JSON.stringify(marker)}, 'executed');\n`;
  const scriptPath = await put(source, 'scripts/never-run.mjs', script);
  await chmod(scriptPath, 0o755);
  await put(source, 'package.json', JSON.stringify({ scripts: { postinstall: 'node scripts/never-run.mjs' } }));
  const preview = await library.previewDirectory(source);
  assert.equal(preview.name, basename(source));
  assert.equal(preview.skillCount, 1);
  assert.equal(preview.fileCount, 6);
  assert.deepEqual(preview.skills.map(entry => entry.name), ['local-inert']);
  assert.equal(preview.skills[0].entryPath, 'SKILL.md');
  assert.ok(preview.warnings.length > 0);
  assert.ok(preview.missingCapabilities.length > 0);
  const saved = await library.importDirectory(source, preview.digest);
  assert.equal(saved.builtin, false);
  assert.equal(saved.digest, preview.digest);
  const rows = library.db.prepare('SELECT path,sha256 FROM skill_files WHERE package_id=?').all(saved.id);
  assert.equal(saved.digest, compatibleDigest(saved, rows));
  const id = saved.skills[0].id;
  const listing = library.listResources(id);
  assert.equal(listing.skillId, id);
  assert.equal(listing.packageId, saved.id);
  assert.equal(listing.resources.find(row => row.path === 'scripts/never-run.mjs').kind, 'script');
  assert.equal(library.readResource(id, 'scripts/never-run.mjs').content, script);
  assert.equal(library.readResource(id, 'NOTICE').content, 'Synthetic source provenance\n');
  assert.equal(typeof library.fingerprint(id), 'string');
  await assert.rejects(access(marker), { code: 'ENOENT' });
});

test('sibling skill folder layout preserves root LICENSE/NOTICE and isolates each entry resources', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'alpha/SKILL.md', 'scope-alpha');
  await skill(source, 'beta/SKILL.md', 'scope-beta');
  await put(source, 'alpha/references/a.txt', 'alpha guidance');
  await put(source, 'beta/references/b.txt', 'beta guidance');
  await put(source, 'LICENSE', 'Synthetic sibling license');
  await put(source, 'NOTICE', 'Synthetic sibling provenance');
  const saved = await installDirectory(library, source);
  assert.equal(saved.skillCount, 2);
  assert.equal(saved.fileCount, 6);
  const alpha = saved.skills.find(entry => entry.name === 'scope-alpha');
  const paths = library.listResources(alpha.id).resources.map(row => row.path);
  assert.ok(paths.includes('alpha/SKILL.md'));
  assert.ok(paths.includes('alpha/references/a.txt'));
  assert.ok(paths.includes('LICENSE'));
  assert.ok(paths.includes('NOTICE'));
  assert.ok(paths.every(path => !path.startsWith('beta/')));
  assert.throws(() => library.readResource(alpha.id, 'beta/SKILL.md'), hasCode('skill_package_resource_denied'));
  assert.equal(library.readResource(alpha.id, 'LICENSE').content, 'Synthetic sibling license');
});

test('preview digest is stable and import rejects changed instructions or resources before writing catalog rows', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'preview-recheck');
  await put(source, 'references/guide.txt', 'original guide');
  const before = await library.previewDirectory(source);
  assert.equal((await library.previewDirectory(source)).digest, before.digest);
  await put(source, 'references/guide.txt', 'changed guide');
  assert.notEqual((await library.previewDirectory(source)).digest, before.digest);
  await assert.rejects(library.importDirectory(source, before.digest), hasCode('skill_package_preview_changed'));
  assert.deepEqual(library.listPackages(), []);
  assert.deepEqual(library.listSkills(), []);
  await assert.rejects(library.importDirectory(source, undefined), hasCode(/^skill_package_/));
  assert.deepEqual(library.listPackages(), []);
});

test('custom package name collision is case-insensitive, atomic and preserves existing content', async t => {
  const { library, source, root } = await fixture(t);
  await skill(source, 'SKILL.md', 'Existing-Custom');
  const first = await installDirectory(library, source);
  const secondSource = join(root, 'second-source');
  await skill(secondSource, 'skills/a/SKILL.md', 'new-unique');
  await skill(secondSource, 'skills/b/SKILL.md', 'existing-custom');
  const preview = await library.previewDirectory(secondSource);
  await assert.rejects(library.importDirectory(secondSource, preview.digest), hasCode('skill_package_name_conflict'));
  assert.equal(library.listPackages().length, 1);
  assert.equal(library.listSkills().length, 1);
  assert.equal(library.listSkills()[0].id, first.skills[0].id);
  assert.equal(library.readResource(first.skills[0].id, 'SKILL.md').content, instructions('Existing-Custom'));
  assert.equal(Number(library.db.prepare('SELECT COUNT(*) AS count FROM skill_files').get().count), 1);
});

test('built-in bundle remains available despite custom names and reserved namespace collisions', async t => {
  const { library, source } = await fixture(t);
  const upstream = await manifest();
  const firstEntry = upstream.files.find(file => file.path.endsWith('/SKILL.md'));
  const originalName = /^name:\s*(.+)$/m.exec(Buffer.from(firstEntry.contentBase64, 'base64').toString('utf8'))[1].trim();
  await skill(source, 'skills/a/SKILL.md', originalName);
  await skill(source, 'skills/b/SKILL.md', `${originalName} (Every Compound)`);
  const custom = await installDirectory(library, source);
  const saved = await library.installBundled(bundlePath);
  assert.equal(saved.skillCount, 36);
  assert.equal(saved.fileCount, 443);
  assert.equal(library.listPackages().length, 2);
  assert.equal(library.listSkills().length, 38);
  assert.equal(new Set(library.listSkills().map(entry => entry.name.toLowerCase())).size, 38);
  const installed = saved.skills.find(entry => entry.entryPath === firstEntry.path);
  assert.notEqual(installed.name.toLowerCase(), originalName.toLowerCase());
  assert.ok([...installed.name].length <= 80);
  const original = custom.skills.find(entry => entry.name === originalName);
  assert.equal(library.readResource(original.id, original.entryPath).content, instructions(originalName));
  assert.equal(library.db.prepare('SELECT instructions FROM skill_entries WHERE id=?').get(installed.id).instructions,
    Buffer.from(firstEntry.contentBase64, 'base64').toString('utf8'));
});

test('installing an already active package is idempotent and does not undo an explicit disabled selection', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'idempotent');
  const first = await installDirectory(library, source);
  const disabled = library.setEnabled(first.skills[0].id, false);
  const currentRevision = library.listPackages().find(entry => entry.id === first.id).revision;
  const again = await installDirectory(library, source);
  assert.equal(again.id, first.id);
  assert.equal(again.revision, currentRevision);
  assert.equal(again.skills[0].id, disabled.id);
  assert.equal(again.skills[0].enabled, false);
  assert.equal(again.skills[0].generation, disabled.generation);
  assert.equal(rowCount(library, 'skill_files', first.id), 1);
});

test('symlink roots, resource files and resource directories are rejected without reading outside the selected folder', { skip: process.platform === 'win32' }, async t => {
  const { library, source, root } = await fixture(t);
  const outside = join(root, 'outside');
  await skill(outside, 'SKILL.md', 'outside-fixture');
  await symlink(outside, join(root, 'root-link'));
  await assert.rejects(library.previewDirectory(join(root, 'root-link')), hasCode(/^skill_package_(symlink_forbidden|safe_open_failed)$/));
  await skill(source, 'SKILL.md', 'inside-fixture');
  await symlink(join(outside, 'SKILL.md'), join(source, 'linked.md'));
  await assert.rejects(library.previewDirectory(source), hasCode(/^skill_package_(symlink_forbidden|safe_open_failed)$/));
  await rm(join(source, 'linked.md'));
  await symlink(outside, join(source, 'references'));
  await assert.rejects(library.previewDirectory(source), hasCode(/^skill_package_(symlink_forbidden|safe_open_failed)$/));
  assert.deepEqual(library.listPackages(), []);
});

test('unsafe literal Unix names are rejected instead of normalized into different resource identities', { skip: process.platform === 'win32' }, async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'unsafe-path');
  for (const path of ['literal\\alternate.txt', 'unsafe:name.txt', 'trailing.', 'trailing ']) {
    await put(source, path, 'synthetic resource');
    await assert.rejects(library.previewDirectory(source), hasCode('skill_package_unsafe_path'));
    await rm(join(source, path));
  }
});

test('duplicate case-insensitive paths and skill names cannot enter a package', async t => {
  const { library, source, root } = await fixture(t);
  await skill(source, 'skills/a/SKILL.md', 'duplicate-name');
  await skill(source, 'skills/b/SKILL.md', 'DUPLICATE-NAME');
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_duplicate_skill_name'));
  const second = join(root, 'case-path');
  await skill(second, 'SKILL.md', 'case-path');
  await put(second, 'skill.md', 'case-sensitive fixture');
  if ((await readdir(second)).length === 2) {
    await assert.rejects(library.previewDirectory(second), hasCode('skill_package_duplicate_path'));
  }
});

test('local import rejects non-UTF8 and oversized instructions rather than silently replacing content', async t => {
  const { library, source } = await fixture(t);
  await put(source, 'SKILL.md', Buffer.from([0xff, 0xfe, 0xfd]));
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_non_utf8_instructions'));
  await put(source, 'SKILL.md', 'x'.repeat(30_001));
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_invalid_instructions'));
  await put(source, 'SKILL.md', '---\nname: unclosed\nMissing frontmatter terminator');
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_invalid_frontmatter'));
});

test('local import enforces1MiB per-file and16MiB total content bounds', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'size-limits');
  const oversized = await open(join(source, 'oversized.txt'), 'w');
  try { await oversized.truncate(1_048_577); } finally { await oversized.close(); }
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_file_too_large'));
  await rm(join(source, 'oversized.txt'));
  const block = Buffer.alloc(1_048_576, 0x61);
  for (let index = 0; index < 16; index++) await put(source, `references/block-${index}.txt`, block);
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_byte_limit'));
  assert.deepEqual(library.listPackages(), []);
});

test('local import enforces512 files and bounded directory depth', async t => {
  const { library, source, root } = await fixture(t);
  await skill(source, 'SKILL.md', 'file-count-limit');
  for (let index = 0; index < 512; index++) await put(source, `references/${index}.txt`, 'fixture');
  await assert.rejects(library.previewDirectory(source), hasCode('skill_package_file_limit'));
  const deep = join(root, 'deep-source');
  await skill(deep, 'SKILL.md', 'depth-limit');
  await put(deep, `${Array.from({ length: 18 }, () => 'nested').join('/')}/resource.txt`, 'fixture');
  await assert.rejects(library.previewDirectory(deep), hasCode('skill_package_depth_limit'));
});

test('bundled importer verifies actual content against recorded hashes before persisting any rows', async t => {
  const { library, root } = await fixture(t);
  const modified = await manifest();
  modified.files[0].contentBase64 = Buffer.from('synthetic replacement with the old recorded hash').toString('base64');
  const path = await put(root, 'tampered-bundle.json.gz', gzipSync(Buffer.from(JSON.stringify(modified))));
  await assert.rejects(library.installBundled(path), hasCode('skill_package_hash_mismatch'));
  assert.deepEqual(library.listPackages(), []);
  assert.deepEqual(library.listSkills(), []);
});

test('resource reader rejects traversal, absolute, drive and ambiguous separator paths', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'resource-path');
  const saved = await installDirectory(library, source);
  for (const path of ['', '/SKILL.md', './SKILL.md', '../SKILL.md', 'a/../SKILL.md', 'a//b', 'a\\b', 'C:SKILL.md', 'a/./b', 'trailing.', 'newline\nname']) {
    assert.throws(() => library.readResource(saved.skills[0].id, path), hasCode('skill_package_unsafe_path'), path);
  }
});

test('UTF8 resource pages use byte offsets without splitting code points and enforce page limits', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'utf8-pages');
  await put(source, 'references/unicode.txt', 'a中😀z');
  const saved = await installDirectory(library, source);
  const id = saved.skills[0].id;
  const first = library.readResource(id, 'references/unicode.txt', 0, 5);
  assert.equal(first.content, 'a中');
  assert.equal(first.offset, 0);
  assert.equal(first.nextOffset, 4);
  assert.equal(first.hasMore, true);
  assert.equal(first.totalBytes, 9);
  const second = library.readResource(id, 'references/unicode.txt', first.nextOffset, 5);
  assert.equal(second.content, '😀z');
  assert.equal(second.hasMore, false);
  assert.equal(second.nextOffset, null);
  assert.equal(library.readResource(id, 'references/unicode.txt', 9, 5).content, '');
  for (const offset of [-1, 2, 10, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => library.readResource(id, 'references/unicode.txt', offset, 5), hasCode('skill_package_offset_invalid'));
  }
  assert.throws(() => library.readResource(id, 'references/unicode.txt', 4, 1), hasCode('skill_package_page_too_small'));
  for (const limit of [0, -1, 8193, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => library.readResource(id, 'references/unicode.txt', 0, limit), hasCode('skill_package_page_limit'));
  }
});

test('binary and private resources are retained without disclosing their contents through reader/listing', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'private-resources');
  await put(source, 'references/binary.bin', Buffer.from([0, 159, 146, 150]));
  const privatePaths = ['.env', '.env.local', '.aws/credentials', '.ssh/id_ed25519', 'credentials.json', 'references/example.pem'];
  for (const path of privatePaths) await put(source, path, 'Synthetic privacy fixture; contains no secret.');
  const saved = await installDirectory(library, source);
  const id = saved.skills[0].id;
  const paths = library.listResources(id).resources.map(row => row.path);
  for (const path of privatePaths) {
    assert.ok(!paths.includes(path), path);
    assert.throws(() => library.readResource(id, path), hasCode('skill_package_resource_denied'), path);
  }
  const binary = library.readResource(id, 'references/binary.bin');
  assert.equal(binary.content, null);
  assert.equal(binary.encoding, 'binary');
  assert.equal(binary.hasMore, false);
  assert.equal(binary.nextOffset, null);
  assert.equal(binary.totalBytes, 4);
  assert.equal(rowCount(library, 'skill_files', saved.id), 8);
});

test('credential detection scans the complete bounded resource before returning its first harmless page', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'credential-pages');
  // Clearly synthetic text verifies denial without using or printing real credentials.
  await put(source, 'references/example.txt', `${'safe synthetic guidance '.repeat(300)}\napi_key = "synthetic-example-only"\n`);
  const saved = await installDirectory(library, source);
  assert.throws(() => library.readResource(saved.skills[0].id, 'references/example.txt', 0, 16), hasCode('skill_package_credential_content_denied'));
});

test('tampered stored resource bytes invalidate both direct reads and grant fingerprints', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'stored-integrity');
  await put(source, 'references/guide.txt', 'reviewed content');
  const saved = await installDirectory(library, source);
  const id = saved.skills[0].id;
  const before = library.fingerprint(id);
  assert.match(before, /^[a-f0-9]{64}$/);
  library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='references/guide.txt'").run(Buffer.from('tampered content'), saved.id);
  assert.throws(() => library.readResource(id, 'references/guide.txt'), hasCode('skill_package_resource_integrity_failed'));
  assert.throws(() => library.fingerprint(id), hasCode('skill_package_resource_integrity_failed'));
});

test('fingerprints bind current provenance, legal notices and validly rehashed reviewed resources', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'fingerprint-content');
  await put(source, 'references/guide.txt', 'original guidance');
  await put(source, 'LICENSE', 'Synthetic original license');
  const saved = await installDirectory(library, source);
  const id = saved.skills[0].id;
  const before = library.fingerprint(id);
  library.db.prepare('UPDATE skill_packages SET source=? WHERE id=?').run('Synthetic amended source', saved.id);
  assertChangedOrRejected(library, id, before);
  library.db.prepare('UPDATE skill_packages SET source=? WHERE id=?').run(saved.source, saved.id);
  assertChangedOrRejected(library, id, before);
  for (const [path, text] of [['references/guide.txt', 'reviewed amended guidance'], ['LICENSE', 'Synthetic amended license']]) {
    const previous = library.fingerprint(id);
    const original = library.db.prepare('SELECT content,size_bytes,sha256 FROM skill_files WHERE package_id=? AND path=?').get(saved.id, path);
    const bytes = Buffer.from(text);
    library.db.prepare('UPDATE skill_files SET content=?,size_bytes=?,sha256=? WHERE package_id=? AND path=?').run(bytes, bytes.length, sha256(bytes), saved.id, path);
    assertChangedOrRejected(library, id, previous);
    library.db.prepare('UPDATE skill_files SET content=?,size_bytes=?,sha256=? WHERE package_id=? AND path=?').run(original.content, original.size_bytes, original.sha256, saved.id, path);
  }
});

test('disable/re-enable advances generation and invalidates prior grant fingerprints without preventing management previews', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'generation');
  const saved = await installDirectory(library, source);
  const first = saved.skills[0];
  const before = library.fingerprint(first.id);
  const disabled = library.setEnabled(first.id, false);
  assert.equal(disabled.enabled, false);
  assert.ok(disabled.generation > first.generation);
  assert.throws(() => library.fingerprint(first.id), hasCode(/^skill_package_.*unavailable$/));
  assert.ok(library.listResources(first.id).resources.some(row => row.path === 'SKILL.md'));
  assert.equal(library.readResource(first.id, 'SKILL.md').content, instructions('generation'));
  const enabled = library.setEnabled(first.id, true);
  assert.equal(enabled.enabled, true);
  assert.ok(enabled.generation > disabled.generation);
  assert.notEqual(library.fingerprint(first.id), before);
  assert.throws(() => library.setEnabled(first.id, 'false'), hasCode(/^skill_package_/));
  assert.throws(() => library.fingerprint(Number.MAX_SAFE_INTEGER), hasCode(/^skill_package_.*unavailable$/));
});

test('uninstall requires a current revision, tombstones data, and reinstall reuses IDs with fresh generation', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'uninstall-generation');
  await put(source, 'references/guide.txt', 'retained synthetic reference');
  const installed = await installDirectory(library, source);
  const first = installed.skills[0];
  const oldFingerprint = library.fingerprint(first.id);
  for (const expectedRevision of [undefined, 0, -1, 1.5, '1', Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => library.uninstallPackage(installed.id, { expectedRevision }), hasCode(/^skill_package_/));
  }
  assert.throws(() => library.uninstallPackage(installed.id, { expectedRevision: installed.revision + 1 }), hasCode('skill_package_revision_conflict'));
  assert.equal(library.listPackages().length, 1);
  const result = library.uninstallPackage(installed.id, { expectedRevision: installed.revision });
  assert.equal(result.id, installed.id);
  assert.equal(result.uninstalled, true);
  assert.equal(result.dataRetained, true);
  assert.ok(result.revision > installed.revision);
  assert.deepEqual(library.listPackages(), []);
  assert.deepEqual(library.listSkills(), []);
  assert.throws(() => library.fingerprint(first.id), hasCode(/^skill_package_.*unavailable$/));
  const tombstone = library.db.prepare('SELECT installed,revision FROM skill_packages WHERE id=?').get(installed.id);
  assert.equal(Number(tombstone.installed), 0);
  assert.equal(Number(tombstone.revision), result.revision);
  assert.equal(rowCount(library, 'skill_files', installed.id), 2);
  const retained = library.db.prepare('SELECT enabled,generation FROM skill_entries WHERE id=?').get(first.id);
  assert.equal(Number(retained.enabled), 0);
  assert.ok(Number(retained.generation) > first.generation);
  const reinstalled = await installDirectory(library, source);
  assert.equal(reinstalled.id, installed.id);
  assert.equal(reinstalled.skills[0].id, first.id);
  assert.equal(reinstalled.skills[0].enabled, true);
  assert.ok(reinstalled.skills[0].generation > Number(retained.generation));
  assert.ok(reinstalled.revision > result.revision);
  assert.notEqual(library.fingerprint(first.id), oldFingerprint);
  assert.throws(() => library.uninstallPackage(installed.id, { expectedRevision: installed.revision }), hasCode('skill_package_revision_conflict'));
  assert.equal(library.listPackages().length, 1);
  assert.equal(rowCount(library, 'skill_files', installed.id), 2);
});

test('package records, disabled generations and tombstones survive reopening the independent library', async t => {
  const f = await fixture(t);
  await skill(f.source, 'SKILL.md', 'persistent-state');
  const saved = await installDirectory(f.library, f.source);
  const disabled = f.library.setEnabled(saved.skills[0].id, false);
  f.library.close();
  f.library = new SkillLibrary({ dataDir: f.dataDir });
  assert.equal(f.library.listPackages()[0].id, saved.id);
  const restored = f.library.listSkills().find(entry => entry.id === disabled.id);
  assert.equal(restored.enabled, false);
  assert.equal(restored.generation, disabled.generation);
  const revision = f.library.listPackages()[0].revision;
  f.library.uninstallPackage(saved.id, { expectedRevision: revision });
  f.library.close();
  f.library = new SkillLibrary({ dataDir: f.dataDir });
  assert.deepEqual(f.library.listPackages(), []);
  assert.equal(rowCount(f.library, 'skill_files', saved.id), 1);
  assert.throws(() => f.library.fingerprint(saved.skills[0].id), hasCode(/^skill_package_.*unavailable$/));
});

test('reinstall refuses corrupted retained resource bytes and leaves the package tombstoned', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'retained-file-integrity');
  await put(source, 'references/guide.txt', 'reviewed retained guidance');
  const saved = await installDirectory(library, source);
  const removed = library.uninstallPackage(saved.id, { expectedRevision: saved.revision });
  library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='references/guide.txt'").run(Buffer.from('corrupted bytes'), saved.id);
  const afterTamper = library.db.prepare('SELECT revision FROM skill_packages WHERE id=?').get(saved.id).revision;
  assert.ok(Number(afterTamper) > removed.revision);
  const preview = await library.previewDirectory(source);
  await assert.rejects(library.importDirectory(source, preview.digest), hasCode('skill_package_resource_integrity_failed'));
  assert.deepEqual(library.listPackages(), []);
  assert.deepEqual(library.listSkills(), []);
  const row = library.db.prepare('SELECT installed,revision FROM skill_packages WHERE id=?').get(saved.id);
  assert.equal(Number(row.installed), 0);
  assert.equal(Number(row.revision), Number(afterTamper));
  assert.equal(rowCount(library, 'skill_files', saved.id), 2);
});

test('reinstall refuses retained entry instructions that no longer match the reviewed SKILL.md', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'retained-entry-integrity');
  const saved = await installDirectory(library, source);
  library.uninstallPackage(saved.id, { expectedRevision: saved.revision });
  library.db.prepare('UPDATE skill_entries SET instructions=? WHERE id=?').run('Synthetic altered instructions outside the reviewed resource snapshot.', saved.skills[0].id);
  const preview = await library.previewDirectory(source);
  await assert.rejects(library.importDirectory(source, preview.digest), hasCode('skill_package_resource_integrity_failed'));
  assert.deepEqual(library.listPackages(), []);
  assert.throws(() => library.fingerprint(saved.skills[0].id), hasCode(/^skill_package_.*unavailable$/));
});

test('reinstall cannot silently adopt altered retained package provenance under the original reviewed digest', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'retained-provenance-integrity');
  const saved = await installDirectory(library, source);
  library.uninstallPackage(saved.id, { expectedRevision: saved.revision });
  library.db.prepare('UPDATE skill_packages SET source=?,version=?,license=? WHERE id=?')
    .run('Synthetic amended retained source', 'altered-version', 'Synthetic amended retained license', saved.id);
  const preview = await library.previewDirectory(source);
  assert.equal(preview.digest, saved.digest);
  let restored;
  try { restored = await library.importDirectory(source, preview.digest); }
  catch (error) {
    assert.match(codeOf(error), /^skill_package_/);
    assert.deepEqual(library.listPackages(), []);
    return;
  }
  // Rejection or restoration of the newly reviewed snapshot are both safe.
  // Relabeling altered metadata with its previous digest is not.
  for (const key of ['packageKey', 'name', 'source', 'version', 'license', 'builtin', 'digest', 'fileCount', 'totalBytes']) {
    assert.equal(restored[key], saved[key], `reviewed ${key} must survive reinstallation`);
  }
});

test('restoring identical resource bytes cannot revive a fingerprint captured before the intervening content change', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'SKILL.md', 'content-generation');
  await put(source, 'references/guide.txt', 'original reviewed guidance');
  const saved = await installDirectory(library, source);
  const id = saved.skills[0].id;
  const before = library.fingerprint(id);
  const original = library.db.prepare("SELECT content,size_bytes,sha256 FROM skill_files WHERE package_id=? AND path='references/guide.txt'").get(saved.id);
  library.db.prepare("UPDATE skill_files SET content=? WHERE package_id=? AND path='references/guide.txt'").run(Buffer.from('synthetic intervening content'), saved.id);
  assert.throws(() => library.fingerprint(id), hasCode('skill_package_resource_integrity_failed'));
  library.db.prepare("UPDATE skill_files SET content=?,size_bytes=?,sha256=? WHERE package_id=? AND path='references/guide.txt'")
    .run(original.content, original.size_bytes, original.sha256, saved.id);
  assert.equal(library.readResource(id, 'references/guide.txt').content, 'original reviewed guidance');
  assert.notEqual(library.fingerprint(id), before);
  assert.ok(library.listSkills().find(entry => entry.id === id).generation > saved.skills[0].generation);
});

test('a sibling-only resource change preserves an unrelated entry fingerprint while shared legal changes revoke both', async t => {
  const { library, source } = await fixture(t);
  await skill(source, 'skills/a/SKILL.md', 'generation-scope-a');
  await skill(source, 'skills/b/SKILL.md', 'generation-scope-b');
  await put(source, 'skills/a/references/guide.txt', 'A guidance');
  await put(source, 'skills/b/references/guide.txt', 'B guidance');
  await put(source, 'LICENSE', 'Synthetic shared license');
  const saved = await installDirectory(library, source);
  const alpha = saved.skills.find(entry => entry.name === 'generation-scope-a');
  const beta = saved.skills.find(entry => entry.name === 'generation-scope-b');
  const beforeAlpha = library.fingerprint(alpha.id);
  const beforeBeta = library.fingerprint(beta.id);
  const next = Buffer.from('C guidance');
  library.db.prepare("UPDATE skill_files SET content=?,size_bytes=?,sha256=? WHERE package_id=? AND path='skills/b/references/guide.txt'")
    .run(next, next.length, sha256(next), saved.id);
  assert.equal(library.fingerprint(alpha.id), beforeAlpha);
  assert.equal(library.listSkills().find(entry => entry.id === alpha.id).generation, alpha.generation);
  assert.notEqual(library.fingerprint(beta.id), beforeBeta);
  const currentBeta = library.fingerprint(beta.id);
  const license = Buffer.from('Synthetic amended shared license');
  library.db.prepare("UPDATE skill_files SET content=?,size_bytes=?,sha256=? WHERE package_id=? AND path='LICENSE'")
    .run(license, license.length, sha256(license), saved.id);
  assert.notEqual(library.fingerprint(alpha.id), beforeAlpha);
  assert.notEqual(library.fingerprint(beta.id), currentBeta);
});
