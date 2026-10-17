import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm, mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import {
  ASSETS_APP_ID, SCANNING_APP_ID, LEGACY_BRIDGE_TRANSPORT_BLOCKER,
  probeLegacyBridge, stageLegacyInputSnapshot, isLegacySnapshotStaged, createHostLegacyBridge,
} from '../src/legacy-bridge.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const fakeWorker = fileURLToPath(new URL('./fixtures/fake-legacy-app-worker.mjs', import.meta.url));
const assetsRoot = fileURLToPath(new URL('../../../apps/assets', import.meta.url));

async function installAssets(t, { workerPath = fakeWorker } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-ux-legacy-'));
  const store = new HostStore(':memory:');
  const apps = new AppManager(store, join(root, 'applications'), { legacyAppWorkerPath: workerPath });
  t.after(async () => {
    try { await apps.close(); } catch { /* ignore */ }
    await rm(root, { recursive: true, force: true });
  });
  const config = JSON.parse(await readFile(join(assetsRoot, 'app.package.json'), 'utf8'));
  const packageOut = join(root, 'pkg');
  await createDirectoryPackage({ sourceDirectory: assetsRoot, outputDirectory: packageOut, ...config });
  const installed = await apps.install(packageOut);
  await apps.setEnabled(installed.id, true);
  const project = store.createProject('Legacy probe');
  apps.setGrants(installed.id, project.id, ['inventory.read', 'inventory.write', 'inventory.exposure.read']);
  return { root, store, apps, project, app: store.application(installed.id) };
}

test('probe stays unavailable without staged snapshot even if worker exists', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'oviraptor-legacy-probe-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const probe = probeLegacyBridge({ appId: ASSETS_APP_ID, dataDir: dir, workerPath: fakeWorker });
  assert.equal(probe.available, false);
  assert.equal(probe.code, 'legacy_snapshot_unavailable');
});

test('staged snapshot alone does not make bridge available without worker/adapter', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'oviraptor-legacy-stage-only-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const source = join(dir, 'source.sqlite3');
  await writeFile(source, 'not-a-real-db-but-regular-file');
  await stageLegacyInputSnapshot(dir, source);
  assert.equal(isLegacySnapshotStaged(dir), true);
  const probe = probeLegacyBridge({ appId: ASSETS_APP_ID, dataDir: dir });
  assert.equal(probe.available, false);
  assert.equal(probe.code, 'legacy_app_worker_unavailable');
});

test('stage + fake worker exposes legacy_snapshot_stats in capabilities and call', async t => {
  const { apps, project, app } = await installAssets(t);
  const dataDir = apps.dataDirectoryFor(app);
  assert.ok(dataDir);

  const before = apps.capabilities(project.id).map(tool => tool.name);
  assert.equal(before.includes(`${ASSETS_APP_ID}::legacy_snapshot_stats`), false);

  const sourceDir = await mkdtemp(join(tmpdir(), 'oviraptor-legacy-src-'));
  t.after(() => rm(sourceDir, { recursive: true, force: true }));
  const source = join(sourceDir, 'snap.sqlite3');
  await writeFile(source, Buffer.from('sqlite-fixture'));
  await apps.stageLegacySnapshot(ASSETS_APP_ID, source);
  assert.equal(isLegacySnapshotStaged(dataDir), true);

  const caps = apps.capabilities(project.id);
  const legacy = caps.find(tool => tool.name === `${ASSETS_APP_ID}::legacy_snapshot_stats`);
  assert.ok(legacy, 'legacy tool should appear once staged+worker available');
  assert.equal(legacy.permission, 'inventory.read');

  const result = await apps.call(ASSETS_APP_ID, 'legacy_snapshot_stats', project.id, {});
  assert.equal(result.structuredContent.available, true);
  assert.equal(result.structuredContent.activeTotal, 7);
  assert.equal(result.structuredContent.executionAvailable, false);
});

test('without worker path, staged snapshot still keeps legacy tool off capabilities', async t => {
  const { apps, project, app } = await installAssets(t, { workerPath: join(tmpdir(), 'missing-oviraptor-app-worker') });
  const sourceDir = await mkdtemp(join(tmpdir(), 'oviraptor-legacy-src2-'));
  t.after(() => rm(sourceDir, { recursive: true, force: true }));
  await writeFile(join(sourceDir, 'snap.sqlite3'), 'x');
  await apps.stageLegacySnapshot(ASSETS_APP_ID, join(sourceDir, 'snap.sqlite3'));
  assert.equal(isLegacySnapshotStaged(apps.dataDirectoryFor(app)), true);
  assert.equal(apps.legacyBridgeProbe(app).available, false);
  assert.equal(
    apps.capabilities(project.id).some(tool => tool.name === `${ASSETS_APP_ID}::legacy_snapshot_stats`),
    false,
  );
});

test('adapter inject path works without spawning worker binary', async t => {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-legacy-adapter-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const dataDir = join(root, 'data');
  await mkdir(dataDir, { recursive: true });
  const source = join(root, 'in.sqlite3');
  await writeFile(source, 'x');
  await stageLegacyInputSnapshot(dataDir, source);
  const adapter = async ({ name }) => ({ available: true, name, total: 2, unit: 'adapter' });
  const bridge = createHostLegacyBridge({ appId: ASSETS_APP_ID, dataDir, adapter });
  const value = await bridge({
    appId: ASSETS_APP_ID,
    name: 'saved_assets_stats',
    arguments: {},
    context: { hostBoundProjectId: 1, grantRevision: 0, operationId: 'op' },
  });
  assert.equal(value.available, true);
  assert.equal(value.total, 2);
});

test('transport blocker documents why Node worker cannot self-inject', () => {
  assert.equal(LEGACY_BRIDGE_TRANSPORT_BLOCKER.code, 'legacy_host_to_worker_inject_deferred');
  assert.match(LEGACY_BRIDGE_TRANSPORT_BLOCKER.summary, /no child-process grant/i);
});

test('scanning tool stays unavailable without staged snapshot', () => {
  const probe = probeLegacyBridge({
    appId: SCANNING_APP_ID,
    dataDir: tmpdir(),
    workerPath: fakeWorker,
  });
  assert.equal(probe.available, false);
  assert.equal(probe.code, 'legacy_snapshot_unavailable');
});
