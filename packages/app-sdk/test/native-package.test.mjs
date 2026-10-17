import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, writeFile, readFile, stat, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { validateManifest, MANIFEST_LIMITS, NATIVE_MANIFEST_LIMITS, ARTIFACT_READ_SCHEMA } from '../src/manifest.mjs';
import { validateNativeExecutable, validatePackageDirectory } from '../src/package-directory.mjs';
import { createDirectoryPackage } from '../src/packager.mjs';
import { validateActionArguments, validateToolArguments } from '../src/schema.mjs';

const schema = { type: 'object', properties: {}, required: [], additionalProperties: false };
function author() {
  return { schemaVersion: 2, id: 'example.native', version: '2.0.0', hostApi: '^2.0.0', entry: 'bin/worker', runtime: { kind: 'native', platform: process.platform, arch: process.arch }, dataSchemaVersion: 1,
    permissions: [{ id: 'read', description: 'Read the current project' }, { id: 'configure', description: 'Manage the whole application' }],
    tools: [{ name: 'read', description: 'Read', permission: 'read', inputSchema: schema }], pages: [{ id: 'main', title: 'Native app', path: 'page.html' }],
    actions: [{ name: 'configure', description: 'Configure', permission: 'configure', scope: 'application', inputSchema: schema }] };
}
const hashed = value => ({ ...value, files: [{ path: value.entry, sha256: 'a'.repeat(64) }, { path: 'page.html', sha256: 'b'.repeat(64) }] });
test('artifact delivery is an explicit project operator read contract', () => {
  const value = hashed(author());
  value.actions.push({ name: 'read_artifact', description: 'Read one owned artifact chunk', permission: 'read', scope: 'project', delivery: 'artifact-read', inputSchema: structuredClone(ARTIFACT_READ_SCHEMA) });
  assert.equal(validateManifest(value).actions[1].delivery, 'artifact-read');
  for (const mutate of [m => m.actions[1].scope = 'application', m => m.actions[1].delivery = null, m => m.actions[1].delivery = 'execute', m => m.actions[1].inputSchema.properties.path = { type: 'string' }, m => m.tools.push({ ...m.actions[1], name: 'smuggled' })]) {
    const invalid = structuredClone(value); mutate(invalid);
    assert.throws(() => validateManifest(invalid), { code: 'app_manifest_invalid' });
  }
});
test('native descriptor is separate and closed; version1 limits and Node contract stay unchanged', () => {
  assert.equal(MANIFEST_LIMITS.fileBytes, 8 * 1024 * 1024);
  assert.equal(MANIFEST_LIMITS.packageBytes, 32 * 1024 * 1024);
  assert.equal(validateManifest(hashed(author())).runtime.kind, 'native');
  for (const mutate of [m => m.runtime.args = ['--arbitrary'], m => m.runtime.kind = 'node', m => m.hostApi = '^1.0.0', m => m.actions[0].scope = 'global', m => m.actions[0].permission = 'undeclared', m => m.entry = '../worker']) {
    const value = hashed(author()); mutate(value);
    assert.throws(() => validateManifest(value), { code: 'app_manifest_invalid' });
  }
  const value = hashed(author()); value.schemaVersion = 1; value.hostApi = '^1.0.0'; value.entry = 'worker.mjs';
  assert.throws(() => validateManifest(value), { code: 'app_manifest_invalid' });
});
test('native executable validation rejects scripts and a different architecture', () => {
  assert.throws(() => validateNativeExecutable(Buffer.from('#!/bin/sh\nexit 0'), { platform: 'linux', arch: 'x64' }), { code: 'app_package_executable_invalid' });
  const elf = Buffer.alloc(64); Buffer.from([0x7f, 0x45, 0x4c, 0x46, 2, 1]).copy(elf); elf.writeUInt16LE(183, 18);
  assert.throws(() => validateNativeExecutable(elf, { platform: 'linux', arch: 'x64' }), { code: 'app_package_executable_invalid' });
  assert.doesNotThrow(() => validateNativeExecutable(elf, { platform: 'linux', arch: 'arm64' }));
});
test('operator batches have an independent finite budget while model tool limits remain unchanged', () => {
  const batch = { type: 'object', properties: { ids: { type: 'array', items: { type: 'integer', minimum: 1 }, maxItems: 1000 } }, additionalProperties: false };
  assert.doesNotThrow(() => validateActionArguments(batch, { ids: Array.from({ length: 200 }, (_, i) => i + 1) }));
  assert.throws(() => validateToolArguments(batch, { ids: [1] }), { code: 'app_schema_invalid' });
  assert.throws(() => validateActionArguments(batch, { ids: Array.from({ length: 1001 }, (_, i) => i + 1) }), { code: 'app_arguments_invalid' });
});
test('native authoring copies a real ELF, hashes it, publishes an executable and detects later tampering', { skip: process.platform !== 'linux' }, async () => {
  const root = await mkdtemp(join(tmpdir(), 'native-sdk-package-'));
  try {
    const source = join(root, 'source'); await mkdir(join(source, 'bin'), { recursive: true });
    await copyFile('/usr/bin/true', join(source, 'bin/worker')); await writeFile(join(source, 'page.html'), '<!doctype html><title>Native</title>');
    const output = join(root, 'package');
    const built = await createDirectoryPackage({ sourceDirectory: source, outputDirectory: output, manifest: author(), files: ['bin/worker', 'page.html'] });
    assert.equal(built.manifest.schemaVersion, 2); assert.equal((await stat(join(output, 'bin/worker'))).mode & 0o777, 0o700);
    assert.deepEqual(await readFile(join(output, 'bin/worker')), await readFile('/usr/bin/true'));
    await writeFile(join(output, 'page.html'), 'changed');
    await assert.rejects(validatePackageDirectory(output), { code: 'app_package_hash_mismatch' });
  } finally { await rm(root, { recursive: true, force: true }); }
});
test('native finite per-file limits reject sparse oversized inputs before allocating their declared size', async () => {
  const root = await mkdtemp(join(tmpdir(), 'native-sdk-limit-'));
  try {
    const source = join(root, 'source'); await mkdir(join(source, 'bin'), { recursive: true });
    const handle = await open(join(source, 'bin/worker'), 'wx');
    try { await handle.truncate(NATIVE_MANIFEST_LIMITS.fileBytes + 1); } finally { await handle.close(); }
    await writeFile(join(source, 'page.html'), 'page');
    await assert.rejects(createDirectoryPackage({ sourceDirectory: source, outputDirectory: join(root, 'package'), manifest: author(), files: ['bin/worker', 'page.html'] }), { code: 'app_package_invalid' });
    await assert.rejects(stat(join(root, 'package')), { code: 'ENOENT' });
  } finally { await rm(root, { recursive: true, force: true }); }
});
