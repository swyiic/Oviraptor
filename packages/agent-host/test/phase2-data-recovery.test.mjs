import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, rename, lstat, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { AppProcess } from '../src/app-process.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const id = 'org.oviraptor.echo';
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-phase2-data-recovery-'));
  const file = join(directory, 'host.sqlite'), appsRoot = join(directory, 'applications');
  let store = new HostStore(file);
  const workers = [];
  const processFactory = (app, data) => { const worker = new AppProcess(app, data); workers.push(worker); return worker; };
  let apps = new AppManager(store, appsRoot, { processFactory });
  t.after(async () => { await apps.close(); store.close(); await rm(directory, { recursive: true, force: true }); });
  const project = store.createProject('Synthetic generation recovery');
  const sourceDirectory = fileURLToPath(new URL('../../../examples/echo-app/', import.meta.url));
  const specification = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const v1 = await createDirectoryPackage({ ...specification, sourceDirectory, outputDirectory: join(directory, 'package-v1') });
  const v2 = await createDirectoryPackage({ ...specification, manifest: { ...specification.manifest, version: '1.1.0' }, sourceDirectory, outputDirectory: join(directory, 'package-v2') });
  await apps.install(v1.directory); await apps.setEnabled(id, true);
  apps.setGrants(id, project.id, ['echo.read', 'echo.write']);
  await apps.call(id, 'echo', project.id, { text: 'Synthetic retained operation' }, 1, 'synthetic-retained-operation');
  await apps.setEnabled(id, false);
  await apps.upgrade(id, v2.directory, { expectedRevision: store.application(id).revision });
  const data = store.application(id).data_path;
  assert.equal(typeof data, 'string');
  assert.equal(JSON.parse(await readFile(join(data, 'echo-state.json'), 'utf8')).records.length, 1);
  return { directory, project, data, v2, workers,
    get apps() { return apps; }, get store() { return store; },
    async restart() { await apps.close(); store.close(); store = new HostStore(file); apps = new AppManager(store, appsRoot, { processFactory }); },
  };
}

for (const reinstall of [false, true]) {
  test(`a missing upgraded data generation is refused before starting a worker${reinstall ? ' after exact-package reinstall' : ' after restart'}`, async t => {
    const f = await fixture(t);
    const retained = join(f.directory, 'preserved-generation');
    await rename(f.data, retained); // Only the test's synthetic generation is moved, never discarded.
    if (reinstall) {
      await f.apps.uninstall(id);
      await f.apps.install(f.v2.directory);
    } else await f.restart();
    const beforeWorkers = f.workers.length;
    await assert.rejects(f.apps.setEnabled(id, true), error => error.code === 'host_app_data_missing');
    assert.equal(f.workers.length, beforeWorkers, 'A missing generation must not be recreated by a candidate app startup');
    await assert.rejects(lstat(f.data), error => error.code === 'ENOENT');
    assert.equal(f.store.application(id).enabled, false);
    assert.equal(JSON.parse(await readFile(join(retained, 'echo-state.json'), 'utf8')).records.length, 1);
    await rename(retained, f.data);
    await f.apps.setEnabled(id, true);
    f.apps.setGrants(id, f.project.id, ['echo.read']);
    assert.equal((await f.apps.call(id, 'history', f.project.id, {})).structuredContent.total, 1);
  });
}

test('a persisted data generation replaced with a symbolic directory is refused before worker startup', async t => {
  const f = await fixture(t);
  const retained = join(f.directory, 'preserved-symlink-target');
  await rename(f.data, retained);
  await symlink(retained, f.data, 'dir');
  await f.restart();
  const beforeWorkers = f.workers.length;
  await assert.rejects(f.apps.setEnabled(id, true), error => error.code === 'host_app_data_invalid');
  assert.equal(f.workers.length, beforeWorkers);
  assert.equal((await lstat(f.data)).isSymbolicLink(), true);
  assert.equal(JSON.parse(await readFile(join(retained, 'echo-state.json'), 'utf8')).records.length, 1);
});
