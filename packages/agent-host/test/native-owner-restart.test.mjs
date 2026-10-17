import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, copyFile, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { AppProcess } from '../src/app-process.mjs';
import { NativeOwners } from '../src/native-owners.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

// Actual child transport and persistent Host SQLite, deliberately synthetic:
// this does not claim Rust SDK or either business engine integration.
const peer = fileURLToPath(new URL('./fixtures/native-protocol-peer.mjs', import.meta.url));
const ID = 'example.native';
const empty = { type: 'object', properties: {}, additionalProperties: false };
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'native-owner-restart-'));
  const storeFile = join(root, 'host.sqlite'), appsRoot = join(root, 'applications');
  let store = new HostStore(storeFile), apps;
  const workers = [], launches = [], kills = [], modes = new Map();
  const factory = (app, data) => {
    const worker = new AppProcess(app, data, { timeoutMs: modes.get(app.version) === 'timeout' ? 300 : 3000,
      spawn(executable, args, options) {
        const owner = new NativeOwners(store.db).get(app.id);
        assert.equal(owner.state, 'starting'); assert.equal(owner.version, app.version);
        assert.equal(owner.digest, app.digest); assert.equal(owner.data_path, data);
        launches.push({ executable, args, owner: { ...owner } });
        const child = spawn(process.execPath, [peer, app.path, modes.get(app.version) ?? 'normal'], options);
        const kill = child.kill.bind(child); child.kill = (...args) => { kills.push(args); return kill(...args); };
        return child;
      },
    });
    workers.push(worker); return worker;
  };
  apps = new AppManager(store, appsRoot, { processFactory: factory });
  const project = store.createProject('Synthetic retained owner');
  async function packageFor(version = '2.0.0') {
    const source = join(root, `source-${version}`); await mkdir(join(source, 'bin'), { recursive: true });
    await copyFile('/usr/bin/true', join(source, 'bin/worker')); await writeFile(join(source, 'page.html'), '<!doctype html>');
    const manifest = { schemaVersion: 2, id: ID, version, hostApi: '^2.0.0', entry: 'bin/worker', dataSchemaVersion: 1,
      runtime: { kind: 'native', platform: process.platform, arch: process.arch }, tools: [],
      actions: [{ name: 'read', description: 'Read inert fixture', inputSchema: empty, permission: 'read', scope: 'project' }],
      pages: [{ id: 'main', title: 'Fixture', path: 'page.html' }], permissions: [{ id: 'read', description: 'Read fixture project' }] };
    return createDirectoryPackage({ sourceDirectory: source, outputDirectory: join(root, `package-${version}`), manifest, files: ['bin/worker', 'page.html'] });
  }
  const built = await packageFor(); await apps.install(built.directory);
  t.after(async () => {
    // Only our inert Node peers are explicitly reaped after all ownership checks.
    // Stored PIDs are never used to find or signal a process.
    for (const worker of workers) if (worker.child && !worker.exitObserved) worker.child.kill('SIGTERM');
    await Promise.all(workers.map(w => w.exitPromise).filter(Boolean));
    try { store.close(); } catch { /* The restart fixture may have closed it. */ }
    await rm(root, { recursive: true, force: true });
  });
  return { root, appsRoot, storeFile, built, project, workers, launches, kills, modes, packageFor,
    get store() { return store; }, get apps() { return apps; },
    restart() { store.close(); store = new HostStore(storeFile); apps = new AppManager(store, appsRoot, { processFactory: factory }); },
    async enable() { await apps.setEnabled(ID, true); apps.setGrants(ID, project.id, ['read'], store.grant(ID, project.id).revision); },
    action() { const app = store.application(ID), grant = store.grant(ID, project.id); return apps.callAction(ID, 'read', project.id, {}, { expectedAppRevision: app.revision, expectedGrantRevision: grant.revision, pageId: 'main', operationId: 'same-operation' }); },
  };
}

test('native owner is durably reserved before real child spawn and bound to exact data generation', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t); await f.enable();
  const row = f.apps.nativeOwners.get(ID), launched = f.launches[0].owner;
  assert.equal(launched.state, 'starting'); assert.equal(row.state, 'running');
  assert.equal(row.generation_nonce, launched.generation_nonce); assert.equal(row.pid, f.workers[0].child.pid);
  assert.equal(row.data_path, f.store.application(ID).data_path);
  await f.apps.close(); assert.equal(f.apps.nativeOwners.get(ID).state, 'stopped');
});

test('Host restart with an existing live native owner blocks dispatch enable upgrade and uninstall without successor', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t); await f.enable(); const before = f.apps.nativeOwners.get(ID), original = f.store.application(ID);
  const next = await f.packageFor('2.1.0'); f.restart();
  assert.equal(f.apps.list()[0].busy, true); assert.equal(f.apps.list()[0].status, 'owner_unresolved');
  assert.deepEqual(f.apps.capabilities(f.project.id), []);
  await assert.rejects(f.action(), { code: 'host_app_owner_unresolved' });
  await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_unresolved' });
  await f.apps.setEnabled(ID, false);
  await assert.rejects(f.apps.upgrade(ID, next.directory, { expectedRevision: f.store.application(ID).revision }), { code: 'host_app_owner_unresolved' });
  await assert.rejects(f.apps.uninstall(ID), { code: 'host_app_owner_unresolved' });
  assert.equal(f.launches.length, 1); assert.equal(f.kills.length, 0);
  assert.equal(f.apps.nativeOwners.get(ID).generation_nonce, before.generation_nonce);
  assert.equal(f.store.application(ID).digest, original.digest);
});

test('only actual idle-stop acknowledgement and observed child exit permits a fresh owner after restart', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t); await f.enable(); const first = f.apps.nativeOwners.get(ID);
  const marker = join(first.data_path, 'retained-data'); await writeFile(marker, 'same original generation');
  await f.apps.close(); assert.equal(f.workers[0].stopConfirmed, true); assert.equal(f.workers[0].exitObserved, true);
  assert.equal(f.apps.nativeOwners.get(ID).state, 'stopped'); f.restart(); await f.apps.setEnabled(ID, true);
  const second = f.apps.nativeOwners.get(ID); assert.equal(second.state, 'running'); assert.notEqual(second.generation_nonce, first.generation_nonce);
  assert.equal(second.data_path, first.data_path); assert.equal(await readFile(marker, 'utf8'), 'same original generation');
  assert.equal(f.launches.length, 2); await f.apps.close();
});

test('native timeout remains durable across Host restart and disabling never clears unresolved ownership', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t); f.modes.set('2.0.0', 'timeout'); await f.enable();
  await assert.rejects(f.action(), { code: 'host_app_timeout' }); const before = f.apps.nativeOwners.get(ID);
  assert.equal(before.state, 'unresolved'); assert.equal(f.kills.length, 0); f.restart();
  await f.apps.setEnabled(ID, false); const after = f.apps.nativeOwners.get(ID);
  assert.equal(after.state, 'unresolved'); assert.equal(after.generation_nonce, before.generation_nonce);
  await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_unresolved' });
  await assert.rejects(f.apps.uninstall(ID), { code: 'host_app_owner_unresolved' });
  assert.equal(f.launches.length, 1); assert.equal(f.kills.length, 0);
});

test('an unresolved owner marker survives removed app metadata and rejects reinstall without touching data', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t); await f.enable(); const before = f.apps.nativeOwners.get(ID);
  const marker = join(before.data_path, 'retained-data'); await writeFile(marker, 'must remain');
  // Emulate a pre-ledger/foreign Host deleting registry metadata, not the normal
  // API, which refuses uninstall above. Owner rows intentionally have no FK.
  f.store.db.prepare('DELETE FROM applications WHERE id=?').run(ID);
  await assert.rejects(f.apps.install(f.built.directory), { code: 'host_app_owner_unresolved' });
  assert.equal(f.apps.nativeOwners.get(ID).generation_nonce, before.generation_nonce);
  assert.equal(await readFile(marker, 'utf8'), 'must remain'); assert.equal(f.launches.length, 1);
});

test('failed native upgrade candidate retains its durable owner and original application pointer across restart', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t); await f.enable(); await f.apps.setEnabled(ID, false);
  const original = f.store.application(ID), next = await f.packageFor('2.1.0'); f.modes.set('2.1.0', 'bad-description');
  await assert.rejects(f.apps.upgrade(ID, next.directory, { expectedRevision: original.revision }));
  const candidate = f.apps.nativeOwners.get(ID); assert.equal(candidate.state, 'unresolved'); assert.equal(candidate.version, '2.1.0');
  assert.notEqual(candidate.data_path, original.data_path); assert.equal(f.store.application(ID).digest, original.digest);
  f.restart(); await assert.rejects(f.apps.upgrade(ID, next.directory, { expectedRevision: original.revision }), { code: 'host_app_owner_unresolved' });
  await assert.rejects(f.apps.uninstall(ID), { code: 'host_app_owner_unresolved' });
  assert.equal(f.launches.length, 2); assert.equal(f.apps.nativeOwners.get(ID).generation_nonce, candidate.generation_nonce);
  assert.equal(f.store.application(ID).digest, original.digest);
});

test('old native installations without an owner ledger are conservatively migrated as unresolved', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t);
  f.store.db.exec("DROP TABLE native_application_owners; DELETE FROM host_settings WHERE key='native_owner_ledger_version'");
  const original = f.store.application(ID); f.restart();
  const owner = f.apps.nativeOwners.get(ID); assert.equal(owner.state, 'unresolved'); assert.equal(owner.pid, null);
  assert.equal(owner.data_path, null); assert.equal(owner.digest, original.digest);
  await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_unresolved' });
  assert.equal(f.launches.length, 0);
});

test('owner nonce CAS rejects stale events and never uses a PID as permission to stop or replace', t => {
  const store = new HostStore(':memory:'); t.after(() => store.close()); const owners = new NativeOwners(store.db);
  const app = { id: ID, version: '2.0.0', digest: 'a'.repeat(64), enabled: false, revision: 1, data_path: null, data_version: 'legacy' };
  const first = owners.reserve(app, '/synthetic/data-one'); owners.transition(first, { state: 'running', pid: 12345 });
  assert.throws(() => owners.reserve(app, '/synthetic/data-two'), { code: 'host_app_owner_unresolved' });
  assert.throws(() => owners.transition(first, { state: 'stopped', pid: 12345 }), { code: 'host_app_stop_unconfirmed' });
  owners.transition(first, { state: 'stopped', idleStopConfirmed: true, exitObserved: true });
  const second = owners.reserve(app, '/synthetic/data-two');
  assert.throws(() => owners.transition(first, { state: 'unresolved', pid: 12345 }), { code: 'host_app_owner_generation_conflict' });
  assert.equal(owners.get(ID).generation_nonce, second.nonce); assert.equal(owners.get(ID).state, 'starting');
});

test('a failed durable reservation prevents child spawn and is never retried as a fresh data generation', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t);
  f.store.db.exec("CREATE TRIGGER reject_owner_insert BEFORE INSERT ON native_application_owners BEGIN SELECT RAISE(ABORT,'synthetic');END");
  await assert.rejects(f.apps.setEnabled(ID, true)); assert.equal(f.launches.length, 0);
  const pointer = f.store.application(ID).data_path; assert.equal(typeof pointer, 'string');
  f.store.db.exec('DROP TRIGGER reject_owner_insert');
  await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_unresolved' });
  assert.equal(f.store.application(ID).data_path, pointer); assert.equal(f.launches.length, 0);
});

test('running owner persistence failure keeps durable starting custody and refuses startup success', { skip: process.platform !== 'linux' }, async t => {
  const f = await fixture(t);
  f.store.db.exec("CREATE TRIGGER reject_owner_update BEFORE UPDATE ON native_application_owners BEGIN SELECT RAISE(ABORT,'synthetic');END");
  await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_persistence_failed' });
  const owner = f.apps.nativeOwners.get(ID); assert.equal(owner.state, 'starting'); assert.equal(f.launches.length, 1); assert.equal(f.kills.length, 0);
  f.restart(); await assert.rejects(f.apps.setEnabled(ID, true), { code: 'host_app_owner_unresolved' });
  assert.equal(f.apps.nativeOwners.get(ID).generation_nonce, owner.generation_nonce); assert.equal(f.launches.length, 1);
});
