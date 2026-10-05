// Read-only reconciliation of unsigned local evidence, not release attestation.
import { constants } from 'node:fs';
import { lstat, open, realpath } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { digest } from './file_inventory.mjs';
import { snapshotBuildInputs } from './build_receipt.mjs';
import { compareAppBundles } from './compare_app_bundles.mjs';

async function directory(value) {
  const resolved = path.resolve(value);
  const info = await lstat(resolved);
  if (info.isSymbolicLink() || !info.isDirectory()) throw new Error('observation_directory_invalid');
  return realpath(resolved);
}

async function readJson(file, limit = 32 * 1024 * 1024) {
  const info = await lstat(file);
  if (!info.isFile() || info.isSymbolicLink()) throw new Error('observation_json_not_regular');
  const handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile()) throw new Error('observation_json_not_regular');
    if (before.size > BigInt(limit)) throw new Error('observation_json_size');
    const buffer = Buffer.alloc(Number(before.size) + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await handle.read(buffer, length, buffer.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    const after = await handle.stat({ bigint: true });
    if (BigInt(length) !== before.size || before.size !== after.size
      || before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs)
      throw new Error('observation_json_changed_during_read');
    try { return JSON.parse(buffer.subarray(0, length).toString('utf8')); }
    catch { throw new Error('observation_json_malformed'); }
  } finally { await handle.close(); }
}

function checkManifest(value, expectedDigest) {
  if (!value || !Array.isArray(value.entries) || !value.entries.length || value.entries.length > 100000
    || !/^[a-f0-9]{64}$/.test(expectedDigest) || value.treeSha256 !== expectedDigest
    || digest(JSON.stringify(value.entries)) !== expectedDigest) throw new Error('observation_manifest_digest');
  let previous = '';
  for (const entry of value.entries) {
    if (!entry || typeof entry.path !== 'string' || entry.path <= previous
      || entry.path.includes('\\') || entry.path.includes('\0')
      || entry.path.split('/').some(part => !part || part === '.' || part === '..')
      || !/^[a-f0-9]{64}$/.test(entry.sha256) || typeof entry.size !== 'string'
      || !/^(0|[1-9][0-9]*)$/.test(entry.size) || !Number.isInteger(entry.executionMode)
      || entry.executionMode < 0 || entry.executionMode > 0o111 || (entry.executionMode & ~0o111) !== 0)
      throw new Error('observation_manifest_entry');
    previous = entry.path;
  }
}

export async function verifyBuildObservation({ root, receiptPath, installedPath }) {
  root = await directory(root);
  receiptPath = path.resolve(receiptPath);
  if (path.basename(receiptPath) !== 'receipt.json') throw new Error('observation_receipt_filename');
  const evidenceDir = await directory(path.dirname(receiptPath));
  const receipt = await readJson(path.join(evidenceDir, 'receipt.json'), 65536);
  const bundleRoot = path.join(evidenceDir, 'target/release/bundle/macos/Oviraptor.app');
  if (!receipt || receipt.schemaVersion !== 1 || receipt.status !== 'build_observed'
    || receipt.build?.exitCode !== 0 || receipt.build.signal !== null || receipt.inputsUnchanged !== true
    || receipt.inputsBeforeSha256 !== receipt.inputsAfterSha256 || receipt.platform !== 'darwin'
    || receipt.assurance !== 'unsigned_local_observation') throw new Error('observation_receipt_inconsistent');
  if (receipt.root !== root || receipt.evidenceDir !== evidenceDir || receipt.bundleRoot !== bundleRoot)
    throw new Error('observation_receipt_path_mismatch');
  const before = await readJson(path.join(evidenceDir, 'inputs-before.json'));
  const after = await readJson(path.join(evidenceDir, 'inputs-after.json'));
  const bundle = await readJson(path.join(evidenceDir, 'bundle.json'));
  checkManifest(before, receipt.inputsBeforeSha256);
  checkManifest(after, receipt.inputsAfterSha256);
  checkManifest(bundle, receipt.bundleTreeSha256);
  if (before.schemaVersion !== 1 || after.schemaVersion !== 1
    || before.scope !== 'reviewed_project_inputs' || after.scope !== before.scope
    || !Array.isArray(before.roots) || !before.roots.length
    || before.roots.some(item => typeof item !== 'string')
    || JSON.stringify(before.roots) !== JSON.stringify(after.roots) || bundle.root !== bundleRoot)
    throw new Error('observation_manifest_metadata');
  // Never follow a redirected output directory in the saved evidence tree.
  let current = evidenceDir;
  for (const component of ['target', 'release', 'bundle', 'macos', 'Oviraptor.app']) {
    current = path.join(current, component);
    await directory(current);
  }
  const inputs = await snapshotBuildInputs(root);
  const bundleComparison = await compareAppBundles(bundleRoot, installedPath);
  const checks = {
    currentInputsMatchRecorded: inputs.treeSha256 === after.treeSha256
      && JSON.stringify(inputs.roots) === JSON.stringify(after.roots),
    builtBundleMatchesRecorded: bundleComparison.expectedTreeSha256 === bundle.treeSha256,
    installedMatchesBuiltBundle: bundleComparison.bundleTreesMatch,
  };
  return { schemaVersion: 1, checkedAt: new Date().toISOString(), root,
    receiptPath: path.join(evidenceDir, 'receipt.json'), checks,
    localObservationsMatch: Object.values(checks).every(Boolean),
    currentInputsSha256: inputs.treeSha256, recordedInputsSha256: after.treeSha256,
    recordedBundleSha256: bundle.treeSha256, bundleComparison,
    assurance: 'unsigned_local_observation', sourceBinding: 'not_verified',
    signatureValidation: 'not_performed', runtimeValidation: 'not_performed', releaseAcceptance: 'not_verified',
    limits: ['not_tamper_resistant', 'not_hermetic', 'not_concurrent_writer_safe',
      'input_scope_is_explicit_not_dependency_closure', 'dependencies_and_user_toolchain_config_not_snapshotted',
      'not_signature_validation', 'not_release_acceptance'] };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    if (process.argv.length !== 5)
      throw new Error('usage: verify_build_observation.mjs PROJECT_ROOT RECEIPT.json INSTALLED.app');
    const report = await verifyBuildObservation({ root: process.argv[2], receiptPath: process.argv[3], installedPath: process.argv[4] });
    console.log(JSON.stringify(report, null, 2));
    process.exitCode = report.localObservationsMatch ? 0 : 2;
  } catch (error) {
    console.error(String(error));
    process.exitCode = 1;
  }
}
