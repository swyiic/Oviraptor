import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtemp, mkdir, copyFile, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { AppProcess } from '../src/app-process.mjs';
import { issueNodeUpgradeProof } from '../src/native-owners.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const ID = 'example.native';
const peer = fileURLToPath(new URL('./fixtures/native-protocol-peer.mjs', import.meta.url));
const empty = { type: 'object', properties: {}, additionalProperties: false };

// This validates real Node-child stop/exit and Host registry/copy custody only.
// The synthetic native peer performs no business-data schema migration.
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'native-runtime-upgrade-'));
  const storeFile = join(root, 'host.sqlite'), appsRoot = join(root, 'apps');
  let store = new HostStore(storeFile), apps, mode = 'normal';
  const workers = [], launches = [], kills = [];
  const factory = (app, data) => {
    const worker = new AppProcess(app, data, { timeoutMs: 3000, ...(app.manifest.schemaVersion === 2 ? {
      spawn(executable, args, options) {
        assert.equal(apps.nativeOwners.get(ID).state, 'starting');
        launches.push({ executable, args });
        const child = spawn(process.execPath, [peer, app.path, mode], options);
        const kill = child.kill.bind(child); child.kill = (...args) => { kills.push(args); return kill(...args); };
        return child;
      },
    } : {}) });
    workers.push(worker); return worker;
  };
  apps = new AppManager(store, appsRoot, { processFactory: factory });
  const oldSource = join(root, 'old-source'); await mkdir(oldSource);
  await writeFile(join(oldSource, 'page.html'), '<!doctype html>');
  await writeFile(join(oldSource, 'worker.mjs'), `import {writeFile} from 'node:fs/promises';
import {join} from 'node:path';import {readFile} from 'node:fs/promises';import {startStdioWorker} from './sdk/worker.mjs';
const manifest=JSON.parse(await readFile(new URL('./manifest.json',import.meta.url),'utf8'));
await startStdioWorker({manifest,onStart:async({dataDir})=>{await writeFile(join(dataDir,'original-data.txt'),'retained original Node bytes');return{dataSchemaVersion:1}},onStop:async()=>({busy:false}),tools:{}}).closed;
`);
  const old = await createDirectoryPackage({ sourceDirectory: oldSource, outputDirectory: join(root, 'old-package'), vendorSdk: true,
    manifest: { schemaVersion: 1, id: ID, version: '1.0.0', hostApi: '^1.0.0', entry: 'worker.mjs', dataSchemaVersion: 1,
      tools: [], pages: [{ id: 'main', title: 'Old fixture', path: 'page.html' }], permissions: [], lifecycle: { start: true, stop: true } }, files: ['worker.mjs', 'page.html'] });
  const nextSource = join(root, 'new-source'); await mkdir(join(nextSource, 'bin'), { recursive: true });
  await copyFile('/usr/bin/true', join(nextSource, 'bin/worker')); await writeFile(join(nextSource, 'page.html'), '<!doctype html>');
  const next = await createDirectoryPackage({ sourceDirectory: nextSource, outputDirectory: join(root, 'new-package'),
    manifest: { schemaVersion: 2, id: ID, version: '2.0.0', hostApi: '^2.0.0', entry: 'bin/worker', dataSchemaVersion: 1,
      runtime: { kind: 'native', platform: process.platform, arch: process.arch }, tools: [],
      actions: [{ name: 'read', description: 'Synthetic read', inputSchema: empty, permission: 'read', scope: 'project' }],
      pages: [{ id: 'main', title: 'New fixture', path: 'page.html' }], permissions: [{ id: 'read', description: 'Synthetic permission' }] }, files: ['bin/worker', 'page.html'] });
  await apps.install(old.directory); await apps.setEnabled(ID, true); await apps.setEnabled(ID, false);
  t.after(async () => {
    for (const worker of workers) if (worker.child && !worker.exitObserved) worker.child.kill('SIGTERM');
    await Promise.all(workers.map(worker => worker.exitPromise).filter(Boolean));
    try { store.close(); } catch { /* Restart fixture closes the preceding handle. */ }
    await rm(root, { recursive: true, force: true });
  });
  return { root, next, workers, launches, kills, get apps() { return apps; }, get store() { return store; }, set mode(value) { mode = value; },
    restart() { store.close(); store = new HostStore(storeFile); apps = new AppManager(store, appsRoot, { processFactory: factory }); },
    candidate() { return { ...store.application(ID), version: next.manifest.version, path: next.directory,
      manifest: next.manifest, digest: createHash('sha256').update(JSON.stringify(next.manifest)).digest('hex'),
      data_path: join(root, 'candidate-data'), data_version: randomUUID() }; },
  };
}

test('a real stopped Node child permits first native candidate ownership without claiming business migration', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t), original = f.store.application(ID), old = f.workers[0];
  const upgraded = await f.apps.upgrade(ID, f.next.directory, { expectedRevision: original.revision });
  const current = f.store.application(ID), candidateOwner = f.apps.nativeOwners.get(ID);
  assert.equal(old.idleStopConfirmed, true); assert.equal(old.exitObserved, true); assert.equal(old.active, 0);
  assert.equal(upgraded.version, '2.0.0'); assert.equal(upgraded.enabled, false);
  assert.equal(candidateOwner.state, 'stopped'); assert.equal(candidateOwner.data_path, current.data_path);
  assert.notEqual(current.data_path, original.data_path);
  assert.equal(await readFile(join(current.data_path, 'original-data.txt'), 'utf8'), 'retained original Node bytes');
  assert.equal(f.launches.length, 1);
  await f.apps.setEnabled(ID, true);
  assert.notEqual(f.apps.nativeOwners.get(ID).generation_nonce, candidateOwner.generation_nonce);
  assert.equal(f.launches.length, 2); await f.apps.close();
});

test('first-native upgrade proof rejects a live child forged token changed candidate and changed registry', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t), original = f.store.application(ID), old = f.workers[0], candidate = f.candidate();
  assert.throws(() => issueNodeUpgradeProof(original, old, candidate), { code: 'host_app_upgrade_owner_proof_invalid' });
  await old.stop();
  const proof = issueNodeUpgradeProof(original, old, candidate), owners = f.apps.nativeOwners;
  assert.throws(() => owners.reserve(candidate, candidate.data_path, {}), { code: 'host_app_owner_unresolved' });
  assert.throws(() => owners.reserve({ ...candidate, digest: 'a'.repeat(64) }, candidate.data_path, proof), { code: 'host_app_owner_unresolved' });
  f.store.db.prepare('UPDATE applications SET revision=revision+1 WHERE id=?').run(ID);
  assert.throws(() => owners.reserve(candidate, candidate.data_path, proof), { code: 'host_app_owner_unresolved' });
  assert.equal(owners.get(ID), null); assert.equal(f.launches.length, 0);
  const current = f.store.application(ID), currentProof = issueNodeUpgradeProof(current, old, candidate);
  const ticket = owners.reserve(candidate, candidate.data_path, currentProof);
  owners.transition(ticket, { state: 'unresolved' });
  assert.throws(() => owners.reserve(candidate, candidate.data_path, proof), { code: 'host_app_owner_unresolved' });
  assert.equal(owners.get(ID).generation_nonce, ticket.nonce); assert.equal(f.launches.length, 0);
});

test('failed first native candidate leaves old Node pointer but blocks either runtime successor after Host restart', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t), original = f.store.application(ID); f.mode = 'bad-description';
  await assert.rejects(f.apps.upgrade(ID, f.next.directory, { expectedRevision: original.revision }));
  const owner = f.apps.nativeOwners.get(ID); assert.equal(owner.state, 'unresolved'); assert.equal(owner.version, '2.0.0');
  assert.equal(f.store.application(ID).version, '1.0.0'); assert.equal(f.store.application(ID).data_path, original.data_path);
  assert.equal(f.workers[0].exitObserved, true); assert.equal(f.launches.length, 1); assert.equal(f.kills.length, 0);
  f.restart();
  await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_unresolved' });
  await assert.rejects(f.apps.upgrade(ID, f.next.directory, { expectedRevision: original.revision }), { code: 'host_app_owner_unresolved' });
  await assert.rejects(f.apps.uninstall(ID), { code: 'host_app_owner_unresolved' });
  assert.equal(f.launches.length, 1); assert.equal(f.workers.length, 2);
  assert.equal(f.apps.nativeOwners.get(ID).generation_nonce, owner.generation_nonce);
});
