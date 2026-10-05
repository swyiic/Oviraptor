// Local release verification only: no application startup, install or network.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { spawnSync } = require('node:child_process');

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'oviraptor-bundle-compare-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const expected = path.join(root, 'expected.app');
  const installed = path.join(root, 'installed.app');
  for (const directory of [expected, installed]) {
    await fs.mkdir(path.join(directory, 'Contents', 'MacOS'), { recursive: true });
    await fs.mkdir(path.join(directory, 'Contents', 'Resources'));
    await fs.writeFile(path.join(directory, 'Contents', 'Info.plist'), 'same version');
    await fs.writeFile(path.join(directory, 'Contents', 'MacOS', 'oviraptor'), 'synthetic binary', { mode: 0o755 });
  }
  const { compareAppBundles } = await import('./release/compare_app_bundles.mjs');
  return { root, expected, installed, compareAppBundles };
}

test('equal bundle trees do not claim source or runtime verification', async (t) => {
  const f = await fixture(t);
  const report = await f.compareAppBundles(f.expected, f.installed);
  assert.equal(report.bundleTreesMatch, true);
  assert.equal(report.sourceBinding, 'not_verified');
  assert.equal(report.runtimeValidation, 'not_performed');
  assert.equal(report.expectedFiles, 2);
  assert.deepEqual(report.differences, []);
});

test('same version cannot hide changed binary, missing resource or extra installed file', async (t) => {
  const f = await fixture(t);
  await fs.writeFile(path.join(f.installed, 'Contents', 'MacOS', 'oviraptor'), 'outdated binary');
  await fs.writeFile(path.join(f.expected, 'Contents', 'Resources', 'current.json'), '{}');
  await fs.writeFile(path.join(f.installed, 'Contents', 'Resources', 'obsolete.json'), '{}');
  const before = await fs.readFile(path.join(f.installed, 'Contents', 'MacOS', 'oviraptor'));
  const report = await f.compareAppBundles(f.expected, f.installed);
  assert.equal(report.bundleTreesMatch, false);
  assert.deepEqual(report.differences, [
    { path: 'Contents/MacOS/oviraptor', kind: 'content_changed' },
    { path: 'Contents/Resources/current.json', kind: 'missing_installed' },
    { path: 'Contents/Resources/obsolete.json', kind: 'unexpected_installed' },
  ]);
  assert.deepEqual(await fs.readFile(path.join(f.installed, 'Contents', 'MacOS', 'oviraptor')), before);
});

test('bundle comparison detects executable permission drift', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t);
  await fs.chmod(path.join(f.installed, 'Contents', 'MacOS', 'oviraptor'), 0o644);
  const report = await f.compareAppBundles(f.expected, f.installed);
  assert.deepEqual(report.differences, [{ path: 'Contents/MacOS/oviraptor', kind: 'execution_mode_changed' }]);
});

test('bundle comparison refuses root and nested symlinks rather than reading their targets', async (t) => {
  const f = await fixture(t);
  const alias = path.join(f.root, 'alias.app');
  await fs.symlink(f.expected, alias, 'dir');
  await assert.rejects(f.compareAppBundles(alias, f.installed), /bundle_root_symlink/);
  await fs.symlink(path.join(f.root, 'nonexistent-secret'), path.join(f.expected, 'Contents', 'Resources', 'link'));
  await assert.rejects(f.compareAppBundles(f.expected, f.installed), /bundle_entry_symlink/);
});

test('bundle comparison refuses identical roots and missing bundles', async (t) => {
  const f = await fixture(t);
  await assert.rejects(f.compareAppBundles(f.expected, f.expected), /bundle_roots_identical/);
  await assert.rejects(f.compareAppBundles(f.expected, path.join(f.root, 'absent.app')), /ENOENT/);
});

test('empty app-shaped directories cannot be accepted as matching applications', async (t) => {
  const f = await fixture(t);
  const empty = path.join(f.root, 'empty.app');
  await fs.mkdir(empty);
  await assert.rejects(f.compareAppBundles(f.expected, empty), /bundle_required_layout_missing/);
  await assert.rejects(f.compareAppBundles(path.join(f.expected, 'Contents'), f.installed), /bundle_root_not_app_directory/);
});

test('CLI distinguishes match, mismatch and inspection failure without installing anything', async (t) => {
  const f = await fixture(t);
  const script = path.join(__dirname, 'release', 'compare_app_bundles.mjs');
  const run = (...args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' });
  assert.equal(run(f.expected, f.installed).status, 0);
  await fs.writeFile(path.join(f.installed, 'Contents', 'Info.plist'), 'different');
  const mismatch = run(f.expected, f.installed);
  assert.equal(mismatch.status, 2);
  assert.equal(JSON.parse(mismatch.stdout).sourceBinding, 'not_verified');
  assert.equal(run(f.expected).status, 1);
  assert.equal(run(f.expected, f.expected).status, 1);
});
