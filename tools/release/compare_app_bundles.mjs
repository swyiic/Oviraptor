// Read-only comparison of two quiescent macOS application bundle trees.
// Equality is not build provenance, signature validation or a runtime test.
import { lstat, realpath } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { digest, inventoryFiles } from './file_inventory.mjs';

async function checkedRoot(value) {
  const root = path.resolve(value);
  const info = await lstat(root);
  if (info.isSymbolicLink()) throw new Error('bundle_root_symlink');
  if (!info.isDirectory() || !root.endsWith('.app')) throw new Error('bundle_root_not_app_directory');
  return realpath(root);
}

async function inventory(root) {
  const entries = await inventoryFiles(root, [''], 'bundle');
  if (!entries.some((item) => item.path === 'Contents/Info.plist')
    || !entries.some((item) => item.path.startsWith('Contents/MacOS/')))
    throw new Error('bundle_required_layout_missing');
  return entries;
}

export async function snapshotAppBundle(bundlePath) {
  const root = await checkedRoot(bundlePath);
  const entries = await inventory(root);
  return { root, entries, treeSha256: digest(JSON.stringify(entries)) };
}

export async function compareAppBundles(expectedPath, installedPath) {
  const expectedRoot = await checkedRoot(expectedPath);
  const installedRoot = await checkedRoot(installedPath);
  if (expectedRoot === installedRoot) throw new Error('bundle_roots_identical');
  const expected = await inventory(expectedRoot);
  const installed = await inventory(installedRoot);
  const left = new Map(expected.map((item) => [item.path, item]));
  const right = new Map(installed.map((item) => [item.path, item]));
  const differences = [];
  for (const name of [...new Set([...left.keys(), ...right.keys()])].sort()) {
    const a = left.get(name), b = right.get(name);
    if (!a) differences.push({ path: name, kind: 'unexpected_installed' });
    else if (!b) differences.push({ path: name, kind: 'missing_installed' });
    else {
      if (a.sha256 !== b.sha256 || a.size !== b.size) differences.push({ path: name, kind: 'content_changed' });
      if (a.executionMode !== b.executionMode) differences.push({ path: name, kind: 'execution_mode_changed' });
    }
  }
  return { schemaVersion: 1, bundleTreesMatch: differences.length === 0,
    sourceBinding: 'not_verified', runtimeValidation: 'not_performed',
    expectedRoot, installedRoot, expectedFiles: expected.length, installedFiles: installed.length,
    expectedTreeSha256: digest(JSON.stringify(expected)), installedTreeSha256: digest(JSON.stringify(installed)),
    differences };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    if (process.argv.length !== 4) throw new Error('usage: compare_app_bundles.mjs EXPECTED.app INSTALLED.app');
    const report = await compareAppBundles(process.argv[2], process.argv[3]);
    console.log(JSON.stringify(report, null, 2));
    process.exitCode = report.bundleTreesMatch ? 0 : 2;
  } catch (error) {
    console.error(String(error));
    process.exitCode = 1;
  }
}
