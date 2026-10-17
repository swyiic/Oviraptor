import test from 'node:test';
import assert from 'node:assert/strict';
import { manifest } from './helpers.mjs';
import { validateManifest, validatePackagePath, qualifiedToolName, validateToolArguments } from '../src/index.mjs';

test('version 1 manifest preserves public declarations and namespaces local tools', () => {
  const source = manifest();
  assert.deepEqual(validateManifest(source), source);
  assert.notEqual(validateManifest(source), source);
  assert.equal(qualifiedToolName(source.id, 'echo'), 'org.oviraptor.test::echo');
});
test('manifest rejects unknown versions fields permissions names and unlisted entry/page', () => {
  const mutations = [v => { v.schemaVersion = 2; }, v => { v.hostApi = '*'; }, v => { v.name = 'display'; }, v => { v.version = '01.0.0'; },
    v => { v.id = 'single'; }, v => { v.tools[0].name = 'org.other::echo'; }, v => { v.tools[0].permission = 'missing'; },
    v => { v.permissions.push(v.permissions[0]); }, v => { v.tools.push(v.tools[0]); }, v => { v.pages[0].path = 'other.html'; },
    v => { v.files[0].sha256 = 'A'.repeat(64); }, v => { v.files.push({ path: 'WORKER.MJS', sha256: 'a'.repeat(64) }); },
    v => { v.files.push({ path: 'worker.mjs/child', sha256: 'a'.repeat(64) }); }, v => { v.files.push({ path: 'manifest.json', sha256: 'a'.repeat(64) }); },
    v => { v.lifecycle = { activate: true }; }, v => { v.dataSchemaVersion = 0; }];
  for (const mutate of mutations) { const value = manifest(); mutate(value); assert.throws(() => validateManifest(value), { name: 'AppSdkError' }); }
});
test('portable package paths reject traversal absolute platform aliases and symlink-like separators', () => {
  for (const path of ['/worker.mjs', '../worker.mjs', 'a/../b', 'a/./b', 'a\\b', 'a//b', 'a/', 'C:/x', 'con.txt', 'aux/a', 'a.', 'a\0b', 'a b']) {
    assert.throws(() => validatePackagePath(path), { code: 'app_manifest_invalid' }, path);
  }
  assert.equal(validatePackagePath('assets/page-v1.css'), 'assets/page-v1.css');
});
test('finite closed JSON schema enforces nested bounds and denies unsupported schema mechanisms', () => {
  const schema = { type: 'object', properties: { entries: { type: 'array', maxItems: 2, items: { type: 'object', properties: { value: { type: 'integer', minimum: 1, maximum: 3 } }, required: ['value'], additionalProperties: false } } }, required: ['entries'], additionalProperties: false };
  assert.deepEqual(validateToolArguments(schema, { entries: [{ value: 2 }] }), { entries: [{ value: 2 }] });
  for (const args of [{}, { entries: [{ value: 4 }] }, { entries: [{ value: 2, extra: true }] }, { entries: [1] }, { entries: [], extra: true }, { entries: [{ value: NaN }] }]) assert.throws(() => validateToolArguments(schema, args));
  for (const inputSchema of [{ type: 'object', properties: {}, additionalProperties: true }, { type: 'object', properties: {}, additionalProperties: false, $ref: 'x' }, { type: 'object', properties: { constructor: { type: 'string' } }, additionalProperties: false }, { type: 'array', items: { type: 'string' } }]) {
    const value = manifest(); value.tools[0].inputSchema = inputSchema; assert.throws(() => validateManifest(value));
  }
});
test('manifest refuses descriptors exceeding the method result budget', () => {
  const value = manifest();
  value.tools = Array.from({ length: 40 }, (_, i) => ({ ...value.tools[0], name: `tool_${i}`, description: 'x'.repeat(2048) }));
  assert.throws(() => validateManifest(value), { code: 'app_manifest_invalid' });
});

test('manifest accepts optional paired_skills reference-only entries', () => {
  const value = manifest({ paired_skills: [{ name: 'guide-one', source: 'bundled', required: false }, { name: 'guide-two', source: 'docs/how-to.md' }] });
  const validated = validateManifest(value);
  assert.equal(validated.paired_skills.length, 2);
  assert.equal(validated.paired_skills[1].source, 'docs/how-to.md');
});

test('manifest rejects executable paired_skills sources', () => {
  for (const source of ['bin/tool.sh', 'worker.mjs', 'scripts/run.js', 'lib/native.so']) {
    const value = manifest({ paired_skills: [{ name: 'bad', source }] });
    assert.throws(() => validateManifest(value), { code: 'app_manifest_invalid' }, source);
  }
});
