import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, writeFile, readFile, symlink } from 'node:fs/promises';
import { join } from 'node:path';
import { createDirectoryPackage, validatePackageDirectory } from '../src/index.mjs';
import { manifest, temporary } from './helpers.mjs';
async function source(t) {
  const root = await temporary(t); const directory = join(root, 'source'); await mkdir(directory);
  await writeFile(join(directory, 'worker.mjs'), '// worker\n'); await writeFile(join(directory, 'page.html'), '<!doctype html>');
  const spec = manifest(); delete spec.files;
  return { root, directory, spec, output: join(root, 'output') };
}
test('directory packager vendors runtime and validates every shipped file against SHA-256', async t => {
  const { directory, spec, output } = await source(t);
  const packed = await createDirectoryPackage({ sourceDirectory: directory, outputDirectory: output, manifest: spec, files: ['worker.mjs', 'page.html'], vendorSdk: true });
  assert.equal(packed.manifest.files.length, 8);
  assert.ok(packed.manifest.files.some(file => file.path === 'sdk/worker.mjs'));
  assert.ok(packed.manifest.files.some(file => file.path === 'sdk/sqlite-data.mjs'));
  assert.ok(packed.manifest.files.some(file => file.path === 'sdk/legacy-bridge-stub.mjs'));
  assert.equal(packed.directory, output);
  assert.match(packed.manifestSha256, /^[a-f0-9]{64}$/);
  assert.deepEqual((await validatePackageDirectory(output)).manifest, packed.manifest);
  await assert.rejects(createDirectoryPackage({ sourceDirectory: directory, outputDirectory: output, manifest: spec, files: ['worker.mjs', 'page.html'] }), { code: 'app_package_exists' });
  await writeFile(join(output, 'worker.mjs'), '// tampered');
  await assert.rejects(validatePackageDirectory(output), { code: 'app_package_hash_mismatch' });
});
test('directory validator refuses unlisted assets symlinks bad UTF-8 and missing files', async t => {
  const { directory, spec, output } = await source(t);
  await createDirectoryPackage({ sourceDirectory: directory, outputDirectory: output, manifest: spec, files: ['worker.mjs', 'page.html'] });
  await writeFile(join(output, 'unlisted.js'), '');
  await assert.rejects(validatePackageDirectory(output), { code: 'app_package_invalid' });
  const { rm } = await import('node:fs/promises'); await rm(join(output, 'unlisted.js'));
  await symlink(join(output, 'worker.mjs'), join(output, 'linked'));
  await assert.rejects(validatePackageDirectory(output), { code: 'app_package_invalid' });
  await rm(join(output, 'linked'));
  const original = await readFile(join(output, 'manifest.json'));
  await writeFile(join(output, 'manifest.json'), Buffer.from([0xff]));
  await assert.rejects(validatePackageDirectory(output), { code: 'app_manifest_invalid' });
  await writeFile(join(output, 'manifest.json'), original); await rm(join(output, 'page.html'));
  await assert.rejects(validatePackageDirectory(output), { code: 'app_package_invalid' });
});
test('packager rejects source symlinks traversal duplicate and self-contained output violations', async t => {
  const { directory, spec, output } = await source(t);
  await symlink(join(directory, 'worker.mjs'), join(directory, 'linked.mjs'));
  const base = { sourceDirectory: directory, outputDirectory: output, manifest: spec };
  for (const files of [['worker.mjs', 'page.html', 'linked.mjs'], ['../outside'], ['worker.mjs', 'page.html', 'worker.mjs'], ['worker.mjs', 'page.html', 'manifest.json']]) await assert.rejects(createDirectoryPackage({ ...base, files }));
  await assert.rejects(createDirectoryPackage({ ...base, outputDirectory: join(directory, 'out'), files: ['worker.mjs', 'page.html'] }));
});
test('packager leaves an existing empty output directory and source untouched', async t => {
  const { directory, spec, output } = await source(t); await mkdir(output);
  await assert.rejects(createDirectoryPackage({ sourceDirectory: directory, outputDirectory: output, manifest: spec, files: ['worker.mjs', 'page.html'] }), { code: 'app_package_exists' });
  const { readdir } = await import('node:fs/promises');
  assert.deepEqual(await readdir(output), []);
  assert.equal(await readFile(join(directory, 'worker.mjs'), 'utf8'), '// worker\n');
});
test('validator rejects oversized file and excessive empty directory tree', async t => {
  const { directory, spec, output } = await source(t);
  await createDirectoryPackage({ sourceDirectory: directory, outputDirectory: output, manifest: spec, files: ['worker.mjs', 'page.html'] });
  const { open, truncate } = await import('node:fs/promises');
  const handle = await open(join(output, 'worker.mjs'), 'r+'); await handle.truncate(8 * 1024 * 1024 + 1); await handle.close();
  await assert.rejects(validatePackageDirectory(output), { code: 'app_package_invalid' });
  await truncate(join(output, 'worker.mjs'), 0); await writeFile(join(output, 'worker.mjs'), '// worker\n');
  for (let index = 0; index < 1025; index++) await mkdir(join(output, `empty-${index}`));
  await assert.rejects(validatePackageDirectory(output), { code: 'app_package_invalid' });
});
