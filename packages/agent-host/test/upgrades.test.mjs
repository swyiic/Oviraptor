import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm, readdir, symlink } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { AppProcess } from '../src/app-process.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const ID = 'org.oviraptor.upgrade-fixture';
const code = expected => error => error?.code === expected;
const clone = value => JSON.parse(JSON.stringify(value));
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

async function tree(directory) {
  const files = {};
  async function visit(relative = '') {
    let entries;
    try { entries = await readdir(join(directory, relative), { withFileTypes: true }); }
    catch (error) { if (error.code === 'ENOENT' && relative === '') return; throw error; }
    for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const path = relative ? `${relative}/${entry.name}` : entry.name;
      if (entry.isSymbolicLink()) files[path] = 'symlink';
      else if (entry.isDirectory()) await visit(path);
      else files[path] = digest(await readFile(join(directory, path)));
    }
  }
  await visit(); return files;
}
const currentData = (f, app = f.store.application(ID)) => app.data_path || join(f.appsRoot, 'data', ID);
const stateFile = (f, app) => join(currentData(f, app), 'state.json');

async function fixture(t, { processFactory } = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-upgrade-'));
  const storeFile = join(directory, 'host.sqlite');
  let store = new HostStore(storeFile);
  const workers = [];
  const appsRoot = join(directory, 'applications');
  const factory = (app, data) => {
    const worker = processFactory ? processFactory(app, data) : new AppProcess(app, data, { timeoutMs: 2000 });
    workers.push(worker); return worker;
  };
  let apps = new AppManager(store, appsRoot, { processFactory: factory });
  const p1 = store.createProject('Upgrade synthetic project one');
  const p2 = store.createProject('Upgrade synthetic project two');
  t.after(async () => {
    for (const worker of workers) if (!worker.exitObserved && worker.child) worker.child.kill('SIGKILL');
    try { await apps.close(); } catch { /* Fault-injected busy/failed worker is explicitly reaped below. */ }
    await Promise.all(workers.filter(worker => worker.exitPromise).map(worker => worker.exitPromise));
    store.close(); await rm(directory, { recursive: true, force: true });
  });
  return { directory, appsRoot, workers, p1, p2,
    get store() { return store; }, get apps() { return apps; },
    async restart() {
      await apps.close(); store.close(); store = new HostStore(storeFile);
      apps = new AppManager(store, appsRoot, { processFactory: factory });
    } };
}
async function packageFor(f, { version = '1.0.0', schema = 1, mode = 'ok', id = ID, stopBusy = false } = {}) {
  const suffix = `${version}-${schema}-${mode}-${f.workers.length}-${Math.random().toString(36).slice(2)}`;
  const sourceDirectory = join(f.directory, `source-${suffix}`);
  const outputDirectory = join(f.directory, `package-${suffix}`);
  await mkdir(sourceDirectory);
  const manifest = { schemaVersion: 1, id, version, hostApi: '^1.0.0', entry: 'worker.mjs', dataSchemaVersion: schema,
    pages: [{ id: 'home', title: 'Synthetic upgrade', path: 'page.html' }],
    permissions: [{ id: 'data.read', description: 'Read synthetic fixture data' }, { id: 'data.write', description: 'Write synthetic fixture data' }],
    tools: [
      { name: 'read', description: 'Read synthetic data', inputSchema: { type: 'object', properties: {}, additionalProperties: false }, permission: 'data.read' },
      { name: 'write', description: 'Write synthetic data', inputSchema: { type: 'object', properties: { value: { type: 'string', maxLength: 128 } }, required: ['value'], additionalProperties: false }, permission: 'data.write' },
    ], lifecycle: { start: true, stop: true } };
  const source = `import { readFile,writeFile,mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { startStdioWorker } from './sdk/worker.mjs';
const manifest=JSON.parse(await readFile(new URL('./manifest.json',import.meta.url),'utf8'));
const mode=${JSON.stringify(mode)};let file,state;
await startStdioWorker({manifest,
onStart:async({dataDir})=>{
  await mkdir(dataDir,{recursive:true});file=join(dataDir,'state.json');
  try{state=JSON.parse(await readFile(file,'utf8'));}catch(e){if(e.code!=='ENOENT')throw e;state={schemaVersion:1,values:[]};}
  if(manifest.dataSchemaVersion===2){
    if(state.schemaVersion!==1&&state.schemaVersion!==2)throw Error('synthetic_bad_old_schema');
    state={...state,schemaVersion:2,migratedByPackageDir:fileURLToPath(new URL('.',import.meta.url)),migratedAtDataDir:dataDir};
    await writeFile(file,JSON.stringify(state));
    if(mode==='throw')throw Error('synthetic_migration_failure_after_candidate_write');
    if(mode==='crash')process.exit(42);
  }else if(state.schemaVersion!==manifest.dataSchemaVersion)throw Error('synthetic_wrong_schema');
  await writeFile(file,JSON.stringify(state));return{dataSchemaVersion:manifest.dataSchemaVersion};
},
onStop:async()=>({busy:${stopBusy ? 'true' : 'false'}}),
tools:{
 read:async(_args,ctx)=>({content:[{type:'text',text:'Synthetic saved state'}],structuredContent:{...state,values:state.values.filter(row=>row.projectId===ctx.hostBoundProjectId)}}),
 write:async(args,ctx)=>{state.values.push({projectId:ctx.hostBoundProjectId,value:args.value});await writeFile(file,JSON.stringify(state));return{content:[]};}
}}).closed;
`;
  await writeFile(join(sourceDirectory, 'worker.mjs'), source);
  await writeFile(join(sourceDirectory, 'page.html'), '<!doctype html><title>Local upgrade fixture</title>');
  const packed = await createDirectoryPackage({ sourceDirectory, outputDirectory, manifest, files: ['worker.mjs', 'page.html'], vendorSdk: true });
  return { sourceDirectory, outputDirectory, manifest: packed.manifest };
}
async function installed(t, { packageOptions, ...options } = {}) {
  const f = await fixture(t, options); const old = await packageFor(f, packageOptions);
  await f.apps.install(old.outputDirectory); await f.apps.setEnabled(ID, true);
  f.apps.setGrants(ID, f.p1.id, ['data.read', 'data.write']); f.apps.setGrants(ID, f.p2.id, ['data.read']);
  await f.apps.call(ID, 'write', f.p1.id, { value: 'original synthetic value' }, 1, 'seed-old');
  return { f, old };
}
async function disabled(t, options) {
  const value = await installed(t, options); await value.f.apps.setEnabled(ID, false); return value;
}
function stateSnapshot(f) { return { app: clone(f.store.application(ID)), grants: clone(f.store.grants(ID)) }; }
async function upgrade(f, source, revision = f.store.application(ID).revision) {
  assert.equal(typeof f.apps.upgrade, 'function', 'Versioned packages require an actual AppManager.upgrade operation');
  return f.apps.upgrade(ID, source, { expectedRevision: revision });
}

test('upgrade migrates an installed copy into a distinct data generation and preserves the old release', async t => {
  const { f } = await disabled(t); const old = stateSnapshot(f); const oldData = currentData(f);
  const oldBytes = await tree(oldData); const oldPackage = await tree(old.app.path);
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  const changed = await upgrade(f, candidate.outputDirectory);
  const current = f.store.application(ID);
  assert.equal(changed.version, '2.0.0'); assert.equal(changed.enabled, false);
  assert.equal(current.version, '2.0.0'); assert.equal(current.enabled, false); assert.ok(current.revision > old.app.revision);
  assert.equal(typeof current.data_path, 'string'); assert.equal(typeof current.data_version, 'string');
  assert.ok(current.data_version.length > 0); assert.notEqual(current.data_version, old.app.data_version);
  assert.notEqual(resolve(current.data_path), resolve(oldData));
  assert.ok(!resolve(current.data_path).startsWith(`${resolve(oldData)}/`), 'A new generation cannot be nested in its copy source');
  assert.notEqual(current.path, candidate.outputDirectory); assert.notEqual(current.path, candidate.sourceDirectory);
  const state = JSON.parse(await readFile(stateFile(f), 'utf8'));
  assert.equal(state.schemaVersion, 2); assert.deepEqual(state.values, [{ projectId: f.p1.id, value: 'original synthetic value' }]);
  assert.equal(resolve(state.migratedByPackageDir), resolve(current.path)); assert.equal(resolve(state.migratedAtDataDir), resolve(current.data_path));
  assert.deepEqual(await tree(oldData), oldBytes); assert.deepEqual(await tree(old.app.path), oldPackage);
  assert.ok(f.workers.every(worker => worker.exitObserved), 'Old writer and candidate migration process must both have actually exited');
  assert.deepEqual(f.apps.capabilities(f.p1.id), []); assert.deepEqual(f.apps.capabilities(f.p2.id), []);
  for (const previous of old.grants) { const grant = f.store.grant(ID, previous.projectId); assert.deepEqual(grant.permissions, []); assert.ok(grant.revision > previous.revision); }
  await f.apps.setEnabled(ID, true);
  await assert.rejects(f.apps.call(ID, 'read', f.p1.id, {}, old.grants[0].revision), code('host_tool_permission_denied'));
  assert.deepEqual(f.apps.capabilities(f.p1.id), []);
  f.apps.setGrants(ID, f.p1.id, ['data.read']);
  assert.equal((await f.apps.call(ID, 'read', f.p1.id, {})).structuredContent.schemaVersion, 2);
});

test('upgrade pointer survives a host restart and re-enable cannot restore old project grants', async t => {
  const { f } = await disabled(t); const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await upgrade(f, candidate.outputDirectory); const current = stateSnapshot(f);
  await f.restart(); assert.deepEqual(stateSnapshot(f), current);
  await f.apps.setEnabled(ID, true); assert.deepEqual(f.apps.capabilities(f.p1.id), []);
  f.apps.setGrants(ID, f.p1.id, ['data.read']); f.apps.setGrants(ID, f.p2.id, ['data.read']);
  assert.equal((await f.apps.call(ID, 'read', f.p1.id, {})).structuredContent.values.length, 1);
  assert.equal((await f.apps.call(ID, 'read', f.p2.id, {})).structuredContent.values.length, 0);
  assert.equal(resolve(f.workers.at(-1).dataDir), resolve(current.app.data_path));
});

for (const mode of ['throw', 'crash']) test(`candidate migration ${mode} leaves old registry grants and data unchanged and can restart old code`, async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2, mode });
  await assert.rejects(upgrade(f, candidate.outputDirectory));
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes);
  const migrated = f.workers.filter(worker => worker.app.version === '2.0.0');
  assert.equal(migrated.length, 1, 'The failure must occur in the real candidate, after copy and process startup');
  assert.equal(migrated[0].exitObserved, true);
  assert.equal(JSON.parse(await readFile(join(migrated[0].dataDir, 'state.json'), 'utf8')).schemaVersion, 2, 'Candidate wrote its isolated copy before failing');
  await f.apps.setEnabled(ID, true); assert.deepEqual(f.apps.capabilities(f.p1.id), []);
  f.apps.setGrants(ID, f.p1.id, ['data.read']);
  const result = await f.apps.call(ID, 'read', f.p1.id, {});
  assert.equal(result.structuredContent.schemaVersion, 1); assert.equal(result.structuredContent.values[0].value, 'original synthetic value');
  assert.equal(f.workers.at(-1).app.version, '1.0.0');
});

test('an enabled app refuses upgrade before migration or registry changes', async t => {
  const { f } = await installed(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f)); const workers = f.workers.length;
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await assert.rejects(upgrade(f, candidate.outputDirectory), code('host_app_upgrade_requires_disabled'));
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes); assert.equal(f.workers.length, workers);
});

test('stale revision identity mismatch same-version replacement and schema downgrade all refuse without candidate execution', async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const valid = await packageFor(f, { version: '2.0.0', schema: 2 });
  await assert.rejects(upgrade(f, valid.outputDirectory, before.app.revision - 1), code('host_app_revision_conflict'));
  const wrongId = await packageFor(f, { version: '2.0.0', schema: 2, id: 'org.oviraptor.other-fixture' });
  const sameVersion = await packageFor(f, { version: '1.0.0', schema: 1, mode: 'different-bytes' });
  for (const candidate of [wrongId, sameVersion]) await assert.rejects(upgrade(f, candidate.outputDirectory));
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes);
  assert.ok(!f.workers.some(worker => worker.app.version === '2.0.0'));
  await upgrade(f, valid.outputDirectory);
  const upgraded = stateSnapshot(f); const downSchema = await packageFor(f, { version: '3.0.0', schema: 1 });
  await assert.rejects(upgrade(f, downSchema.outputDirectory)); assert.deepEqual(stateSnapshot(f), upgraded);
});

test('busy old worker refuses upgrade without a new process or data generation', async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const oldWorker = f.apps.workers.get(ID); oldWorker.active = 1;
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  try { await assert.rejects(upgrade(f, candidate.outputDirectory), code('host_app_busy')); }
  finally { oldWorker.active = 0; }
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes); assert.equal(f.workers.length, 1);
});

test('upgrade does not copy or commit until old process exit is confirmed', async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const oldWorker = f.apps.workers.get(ID); const realStop = oldWorker.stop.bind(oldWorker);
  let release; let entered;
  const stopped = new Promise(resolve => { entered = resolve; });
  const barrier = new Promise(resolve => { release = resolve; });
  oldWorker.stop = async () => { entered(); await barrier; await realStop(); };
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  const pending = upgrade(f, candidate.outputDirectory); const observed = pending.then(value => ({ value }), error => ({ error }));
  await stopped;
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes); assert.equal(f.workers.length, 1);
  await assert.rejects(f.apps.setEnabled(ID, true), code('host_app_changing'));
  release(); const result = await observed; assert.equal(result.error, undefined); assert.equal(result.value.version, '2.0.0');
  assert.equal(oldWorker.exitObserved, true);
});

test('symlinked app data is rejected without following or rewriting the external file', async t => {
  const { f } = await disabled(t); const old = stateSnapshot(f);
  const outside = join(f.directory, 'outside-synthetic.json'); await writeFile(outside, 'outside must remain unchanged');
  await symlink(outside, join(currentData(f), 'unsafe-link'));
  const before = await tree(currentData(f)); const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await assert.rejects(upgrade(f, candidate.outputDirectory));
  assert.deepEqual(stateSnapshot(f), old); assert.deepEqual(await tree(currentData(f)), before);
  assert.equal(await readFile(outside, 'utf8'), 'outside must remain unchanged');
});

test('a cold current owner can veto upgrade without any active tool call', async t => {
  const { f } = await disabled(t, { packageOptions: { stopBusy: true } });
  const originalWorker = f.apps.workers.get(ID); originalWorker.child.kill('SIGKILL'); await originalWorker.exitPromise;
  f.apps.workers.delete(ID); // Model an old process lost before this host recreated its owner.
  const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await assert.rejects(upgrade(f, candidate.outputDirectory), code('host_app_busy'));
  assert.equal(f.workers.length, 2); assert.equal(f.workers.at(-1).app.version, '1.0.0'); assert.equal(f.workers.at(-1).active, 0);
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes);
});

test('a busy candidate is retained under supervision and never becomes the current app', async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2, stopBusy: true });
  await assert.rejects(upgrade(f, candidate.outputDirectory), code('host_app_busy'));
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes);
  const pending = f.apps.upgradeWorkers.get(ID);
  assert.ok(pending); assert.equal(pending.app.version, '2.0.0'); assert.equal(pending.exitObserved, false);
  assert.equal(JSON.parse(await readFile(join(pending.dataDir, 'state.json'), 'utf8')).schemaVersion, 2);
  await assert.rejects(f.apps.setEnabled(ID, true), code('host_app_busy'));
  assert.deepEqual(f.apps.capabilities(f.p1.id), []);
});

test('registry transaction failure rolls back package data pointers retained history and grants together', async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const bytes = await tree(currentData(f));
  const historyBefore = f.store.db.prepare('SELECT * FROM application_versions ORDER BY rowid').all();
  f.store.db.exec("CREATE TRIGGER synthetic_upgrade_abort BEFORE UPDATE OF version ON applications WHEN NEW.version='2.0.0' BEGIN SELECT RAISE(ABORT,'synthetic_upgrade_commit_failure'); END");
  const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await assert.rejects(upgrade(f, candidate.outputDirectory), /synthetic_upgrade_commit_failure/);
  assert.deepEqual(stateSnapshot(f), before); assert.deepEqual(await tree(currentData(f)), bytes);
  assert.deepEqual(f.store.db.prepare('SELECT * FROM application_versions ORDER BY rowid').all(), historyBefore);
  assert.equal(f.workers.filter(worker => worker.app.version === '2.0.0').length, 1);
  assert.ok(f.workers.every(worker => worker.exitObserved));
});

test('upgrade requires an explicit current revision before invoking an app', async t => {
  const { f } = await disabled(t); const before = stateSnapshot(f); const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  assert.equal(typeof f.apps.upgrade, 'function');
  for (const options of [undefined, {}, { expectedRevision: 0 }, { expectedRevision: before.app.revision, extra: true }]) {
    await assert.rejects(f.apps.upgrade(ID, candidate.outputDirectory, options), code('host_app_revision_required'));
  }
  assert.deepEqual(stateSnapshot(f), before); assert.equal(f.workers.length, 1);
});

test('upgraded data written before uninstall is restored by exact-package reinstall without restoring grants', async t => {
  const { f } = await disabled(t); const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await upgrade(f, candidate.outputDirectory); await f.apps.setEnabled(ID, true);
  f.apps.setGrants(ID, f.p1.id, ['data.read', 'data.write']);
  await f.apps.call(ID, 'write', f.p1.id, { value: 'newest upgraded value' }, undefined, 'write-after-upgrade');
  await f.apps.setEnabled(ID, false);
  const current = f.store.application(ID); const saved = await tree(currentData(f));
  assert.equal((await f.apps.uninstall(ID)).dataRetained, true);
  assert.deepEqual(f.store.applications(), []); assert.deepEqual(await tree(current.data_path), saved);
  const reinstalled = await f.apps.install(candidate.outputDirectory);
  assert.equal(reinstalled.enabled, false); assert.deepEqual(f.apps.capabilities(f.p1.id), []);
  assert.equal(f.store.application(ID).data_path, current.data_path); assert.equal(f.store.application(ID).data_version, current.data_version);
  await f.apps.setEnabled(ID, true); assert.deepEqual(f.apps.capabilities(f.p1.id), []);
  await assert.rejects(f.apps.call(ID, 'read', f.p1.id, {}), code('host_tool_permission_denied'));
  f.apps.setGrants(ID, f.p1.id, ['data.read']);
  const values = (await f.apps.call(ID, 'read', f.p1.id, {})).structuredContent.values;
  assert.deepEqual(values, [{ projectId: f.p1.id, value: 'original synthetic value' }, { projectId: f.p1.id, value: 'newest upgraded value' }]);
  assert.deepEqual(f.store.db.prepare('SELECT * FROM application_retained WHERE app_id=?').all(ID), []);
});

test('incompatible reinstall cannot adopt or overwrite a retained current data generation', async t => {
  const { f, old } = await disabled(t); const candidate = await packageFor(f, { version: '2.0.0', schema: 2 });
  await upgrade(f, candidate.outputDirectory);
  const current = f.store.application(ID); const saved = await tree(currentData(f));
  await f.apps.uninstall(ID);
  const retained = f.store.db.prepare('SELECT * FROM application_retained WHERE app_id=?').all(ID);
  assert.equal(retained.length, 1);
  const changedBytes = await packageFor(f, { version: '2.0.0', schema: 2, mode: 'same-version-different-bytes' });
  const changedSchema = await packageFor(f, { version: '2.0.0', schema: 3 });
  for (const source of [old.outputDirectory, changedBytes.outputDirectory, changedSchema.outputDirectory]) {
    await assert.rejects(f.apps.install(source), code('host_app_retained_version_conflict'));
    assert.deepEqual(f.store.applications(), []);
    assert.deepEqual(f.store.db.prepare('SELECT * FROM application_retained WHERE app_id=?').all(ID), retained);
    assert.deepEqual(await tree(current.data_path), saved);
  }
});
