import { mkdir, readFile, writeFile, copyFile, rename, rm, lstat, readdir, open, chmod } from 'node:fs/promises';
import { constants, lstatSync } from 'node:fs';
import { join, dirname, resolve, relative, isAbsolute } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { validatePackageDirectory } from '../../app-sdk/src/index.mjs';
import { hostError } from './store.mjs';
import { issueNodeUpgradeProof } from './native-owners.mjs';

const COPY_LIMITS = Object.freeze({ entries: 4096, bytes: 256 * 1024 * 1024, fileBytes: 64 * 1024 * 1024 });
const sameManifest = (left, right) => JSON.stringify(left) === JSON.stringify(right);
const digest = manifest => createHash('sha256').update(JSON.stringify(manifest)).digest('hex');
const exists = async path => { try { return await lstat(path); } catch (error) { if (error.code === 'ENOENT') return null; throw error; } };

function compareVersion(left, right) {
  const parse = version => {
    const [core, prerelease] = version.split('+')[0].split('-');
    return { core: core.split('.').map(BigInt), prerelease: prerelease === undefined ? null : version.split('+')[0].slice(core.length + 1).split('.') };
  };
  const a = parse(left), b = parse(right);
  for (let i = 0; i < 3; i++) if (a.core[i] !== b.core[i]) return a.core[i] > b.core[i] ? 1 : -1;
  if (a.prerelease === null || b.prerelease === null) return a.prerelease === b.prerelease ? 0 : a.prerelease === null ? 1 : -1;
  for (let i = 0; i < Math.max(a.prerelease.length, b.prerelease.length); i++) {
    const x = a.prerelease[i], y = b.prerelease[i];
    if (x === undefined || y === undefined) return x === undefined ? -1 : 1;
    if (x === y) continue;
    const xn = /^\d+$/u.test(x), yn = /^\d+$/u.test(y);
    if (xn && yn) return BigInt(x) > BigInt(y) ? 1 : -1;
    if (xn !== yn) return xn ? -1 : 1;
    return x > y ? 1 : -1;
  }
  return 0;
}
function originalData(manager, app) {
  const legacy = resolve(manager.root, 'data', app.id);
  const versions = resolve(manager.root, 'data-versions', app.id);
  const path = resolve(app.data_path ?? legacy);
  const generation = app.data_version ?? 'legacy';
  if (generation === 'legacy' && path === legacy) return path;
  if (typeof generation !== 'string' || !/^[0-9a-f-]{36}$/u.test(generation) || path !== join(versions, generation)) throw hostError('host_app_data_pointer_invalid');
  return path;
}
async function requireManagedPath(root, path) {
  const rel = relative(resolve(root), resolve(path));
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) throw hostError('host_app_data_pointer_invalid');
  let current = resolve(root);
  for (const part of rel.split('/')) {
    current = join(current, part);
    const info = await lstat(current);
    if (!info.isDirectory() || info.isSymbolicLink()) throw hostError('host_app_data_copy_invalid');
  }
}
async function copyData(source, target) {
  let entries = 0, bytes = 0;
  async function visit(from, to) {
    const children = await readdir(from, { withFileTypes: true });
    for (const child of children) {
      if (++entries > COPY_LIMITS.entries) throw hostError('host_app_data_copy_limited');
      const path = join(from, child.name), destination = join(to, child.name);
      const info = await lstat(path);
      if (info.isSymbolicLink() || (!info.isDirectory() && !info.isFile()) || info.nlink > (info.isDirectory() ? Number.MAX_SAFE_INTEGER : 1)) throw hostError('host_app_data_copy_invalid');
      if (info.isDirectory()) { await mkdir(destination, { mode: 0o700 }); await visit(path, destination); continue; }
      bytes += info.size;
      if (info.size > COPY_LIMITS.fileBytes || bytes > COPY_LIMITS.bytes) throw hostError('host_app_data_copy_limited');
      const input = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
      let output;
      try {
        const before = await input.stat();
        if (!before.isFile() || before.dev !== info.dev || before.ino !== info.ino || before.size !== info.size || before.nlink !== 1) throw hostError('host_app_data_copy_changed');
        output = await open(destination, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o600);
        const buffer = Buffer.alloc(64 * 1024); let copied = 0;
        while (true) {
          const result = await input.read(buffer, 0, buffer.length, null);
          if (!result.bytesRead) break;
          copied += result.bytesRead;
          if (copied > before.size) throw hostError('host_app_data_copy_changed');
          let offset = 0;
          while (offset < result.bytesRead) offset += (await output.write(buffer, offset, result.bytesRead - offset, null)).bytesWritten;
        }
        const after = await input.stat();
        if (copied !== before.size || after.size !== before.size || after.mtimeMs !== before.mtimeMs || after.ctimeMs !== before.ctimeMs) throw hostError('host_app_data_copy_changed');
        await output.sync();
      } finally { await input.close(); await output?.close(); }
    }
  }
  await visit(source, target);
}
async function stagePackage(manager, source, checked) {
  const manifest = checked.manifest;
  const identity = digest(manifest);
  const root = join(manager.root, 'packages');
  const staging = join(root, `.upgrade-${randomUUID()}`);
  const target = join(root, `${manifest.id}-${manifest.version}-${identity}`);
  await mkdir(root, { recursive: true, mode: 0o700 });
  const retained = await exists(target);
  if (retained) {
    if (!sameManifest((await validatePackageDirectory(target)).manifest, manifest)) throw hostError('host_package_identity_conflict');
    return { path: target, digest: identity };
  }
  await mkdir(staging, { mode: 0o700 });
  try {
    for (const file of manifest.files) {
      const destination = join(staging, file.path);
      await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
      await copyFile(join(source, file.path), destination, constants.COPYFILE_EXCL);
      if (manifest.schemaVersion === 2 && file.path === manifest.entry) await chmod(destination, 0o700);
    }
    await writeFile(join(staging, 'manifest.json'), JSON.stringify(manifest), { flag: 'wx', mode: 0o600 });
    const copied = await validatePackageDirectory(staging);
    if (!sameManifest(copied.manifest, manifest)) throw hostError('host_package_identity_conflict');
    await rename(staging, target);
    return { path: target, digest: identity };
  } finally { await rm(staging, { recursive: true, force: true }); }
}
async function stopConfirmed(worker) {
  await worker.stop();
  // AppProcess.stop waits for the exit event. Process factories used by embedders
  // must provide the same guarantee; a real child with unknown exit is refused.
  if (worker.child && worker.exitObserved !== true) throw hostError('host_app_stop_unconfirmed', 409);
}
function assertUnchanged(manager, original, expectedRevision) {
  const current = manager.store.application(original.id);
  const legacy = resolve(manager.root, 'data', original.id);
  const learnedLegacy = original.data_path == null && original.data_version === 'legacy'
    && current.data_version === 'legacy' && current.data_path === legacy;
  if (learnedLegacy) {
    let directory;
    try { directory = lstatSync(legacy); }
    catch (error) { if (['ENOENT', 'ENOTDIR'].includes(error.code)) throw hostError('host_app_data_missing'); throw error; }
    if (!directory.isDirectory() || directory.isSymbolicLink()) throw hostError('host_app_data_invalid');
  }
  if (current.enabled || current.revision !== expectedRevision || current.version !== original.version || current.digest !== original.digest
    || current.path !== original.path || current.data_path !== original.data_path && !learnedLegacy || current.data_version !== original.data_version) throw hostError('host_app_revision_conflict', 409);
}

/** Prepare on isolated app data, then switch only host-owned registry pointers. */
export async function upgradeApplication(manager, id, source, options = {}) {
  if (manager.closing) throw hostError('host_app_manager_closing', 409);
  if (!options || Object.keys(options).some(key => key !== 'expectedRevision') || !Number.isSafeInteger(options.expectedRevision) || options.expectedRevision < 1) throw hostError('host_app_revision_required', 409);
  if (typeof source !== 'string') throw hostError('host_package_path_invalid');
  const app = manager.store.application(id), expectedRevision = options.expectedRevision;
  if (app.revision !== expectedRevision) throw hostError('host_app_revision_conflict', 409);
  if (app.enabled) throw hostError('host_app_upgrade_requires_disabled', 409);
  if (manager.changing.has(id)) throw hostError('host_app_changing', 409);
  if (manager.startingWorkers.has(id) || manager.workers.get(id)?.active) throw hostError('host_app_busy', 409);
  manager.upgradeWorkers ??= new Map();
  manager.changing.add(id);
  let candidate;
  try {
    const leftover = manager.upgradeWorkers.get(id);
    if (leftover) { await stopConfirmed(leftover); manager.upgradeWorkers.delete(id); }
    const checked = await validatePackageDirectory(source);
    const manifest = checked.manifest;
    manager.checkPlatform(manifest);
    if (manifest.id !== app.id) throw hostError('host_app_upgrade_identity_mismatch');
    if (compareVersion(manifest.version, app.version) <= 0) throw hostError('host_app_upgrade_version_not_newer');
    if (manifest.dataSchemaVersion < app.manifest.dataSchemaVersion) throw hostError('host_app_upgrade_schema_downgrade');
    const data = originalData(manager, app);
    const existing = manager.workers.get(id);
    if (existing?.stopped) { await stopConfirmed(existing); manager.workers.delete(id); }
    // Cold-start the current owner too: persistent paused obligations must be
    // allowed to veto stop even when the host has not yet created this worker.
    const old = await manager.worker(app);
    if (old.active) throw hostError('host_app_busy', 409);
    await stopConfirmed(old);
    if (manager.workers.get(id) === old) manager.workers.delete(id);
    assertUnchanged(manager, app, expectedRevision);
    await requireManagedPath(manager.root, data);
    const installed = await stagePackage(manager, checked.directory, checked);
    const dataVersion = randomUUID();
    const parent = join(manager.root, 'data-versions', app.id);
    await mkdir(parent, { recursive: true, mode: 0o700 });
    const candidateData = join(parent, dataVersion);
    await mkdir(candidateData, { mode: 0o700 });
    await copyData(data, candidateData);
    const candidateApp = { ...app, version: manifest.version, path: installed.path, digest: installed.digest, manifest,
      enabled: false, data_path: candidateData, data_version: dataVersion };
    const nodeUpgradeProof = app.manifest.schemaVersion === 1 && manifest.schemaVersion === 2 ? issueNodeUpgradeProof(app, old, candidateApp) : undefined;
    candidate = manager.createWorker(candidateApp, candidateData, nodeUpgradeProof);
    manager.upgradeWorkers.set(id, candidate);
    await manager.startWorker(candidate, candidateApp);
    await stopConfirmed(candidate);
    manager.upgradeWorkers.delete(id);
    const db = manager.store.db;
    db.exec('BEGIN IMMEDIATE');
    try {
      manager.nativeOwners.assertSettled(id);
      assertUnchanged(manager, app, expectedRevision);
      db.prepare('INSERT INTO application_versions(app_id,version,digest,path,manifest,data_path,data_version,replaced_at,replaced_by_version) VALUES(?,?,?,?,?,?,?,?,?)')
        .run(app.id, app.version, app.digest, app.path, JSON.stringify(app.manifest), data, app.data_version ?? 'legacy', new Date().toISOString(), manifest.version);
      const changed = db.prepare('UPDATE applications SET version=?,digest=?,path=?,manifest=?,data_path=?,data_version=?,enabled=0,revision=revision+1,status=? WHERE id=? AND revision=? AND enabled=0')
        .run(manifest.version, installed.digest, installed.path, JSON.stringify(manifest), candidateData, dataVersion, 'installed', app.id, expectedRevision);
      if (changed.changes !== 1) throw hostError('host_app_revision_conflict', 409);
      db.prepare("UPDATE application_grants SET permissions='[]',revision=revision+1 WHERE app_id=?").run(id);
      db.exec('COMMIT');
    } catch (error) { db.exec('ROLLBACK'); throw error; }
    return manager.view(manager.store.application(id));
  } catch (error) {
    if (candidate && manager.upgradeWorkers.get(id) === candidate) {
      // Never discard a still-busy migration writer or its data. The manager's
      // close/retry path supervises this map; it is not a tool-capability map.
      await stopConfirmed(candidate);
      manager.upgradeWorkers.delete(id);
    }
    throw error;
  } finally { manager.changing.delete(id); }
}
