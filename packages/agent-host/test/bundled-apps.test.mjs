import test from 'node:test';
import assert from 'node:assert/strict';
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const repo = fileURLToPath(new URL('../../../', import.meta.url));

async function packageApp(sourceDirectory, outputDirectory) {
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  return createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
}

async function stageRoot(t) {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-bundled-apps-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'packages', 'agent-host'), { recursive: true });
  await mkdir(join(root, 'apps'), { recursive: true });
  await cp(join(repo, 'packages/agent-host/bundled-apps.json'), join(root, 'packages/agent-host/bundled-apps.json'));
  for (const name of ['assets', 'scanning', 'hackerone']) {
    const sourceDirectory = join(repo, 'apps', name);
    const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
    await packageApp(sourceDirectory, join(root, 'apps', `${spec.manifest.id}-${spec.manifest.version}`));
  }
  return root;
}

test('ensureBundledApps installs and enables assets plus scanning once, and refuses builtin uninstall', async t => {
  const root = await stageRoot(t);
  const dataDir = join(root, 'data');
  const host = new AgentHost({ dataDir });
  t.after(() => host.close());
  const first = await host.ensureBundledApps({ root });
  assert.equal(first.missingManifest, false);
  assert.equal(first.items.length, 3);
  assert.deepEqual(first.items.map(item => item.id).sort(), ['org.oviraptor.assets', 'org.oviraptor.hackerone', 'org.oviraptor.scanning']);
  const core = first.items.filter(item => item.id !== 'org.oviraptor.hackerone');
  for (const item of core) {
    assert.equal(item.installed, true);
    assert.equal(item.enabled, true);
    assert.equal(item.builtin, true);
    assert.equal(item.uninstallable, false);
    assert.equal(item.error, null);
  }
  const hackerone = first.items.find(item => item.id === 'org.oviraptor.hackerone');
  assert.equal(hackerone.installed, true);
  assert.equal(hackerone.enabled, false);
  assert.equal(hackerone.builtin, false);
  assert.equal(hackerone.uninstallable, true);
  assert.equal(hackerone.error, null);
  const listed = host.apps.list();
  assert.equal(listed.length, 3);
  assert.ok(listed.filter(app => app.id !== 'org.oviraptor.hackerone').every(app => app.enabled && app.builtin && app.uninstallable === false));
  assert.equal(listed.find(app => app.id === 'org.oviraptor.hackerone').enabled, false);
  assert.equal(listed.find(app => app.id === 'org.oviraptor.scanning').manifest.tools.some(tool => tool.name === 'task_status'), true);
  assert.equal(listed.find(app => app.id === 'org.oviraptor.scanning').manifest.tools.some(tool => tool.name === 'scan_start'), false);

  const second = await host.ensureBundledApps({ root });
  assert.equal(second.items.length, 3);
  assert.equal(host.apps.list().length, 3);
  await assert.rejects(host.apps.uninstall('org.oviraptor.assets'), error => error.code === 'host_app_builtin');
  await assert.rejects(host.apps.uninstall('org.oviraptor.scanning'), error => error.code === 'host_app_builtin');

  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  const bundled = await (await fetch(`${web.origin}/api/apps/bundled`, { headers })).json();
  assert.equal(bundled.items.length, 3);
  assert.ok(bundled.items.filter(item => item.id !== 'org.oviraptor.hackerone').every(item => item.enabled && item.status === 'enabled'));
  const bundledH1 = bundled.items.find(item => item.id === 'org.oviraptor.hackerone');
  assert.equal(bundledH1.enabled, false);
  assert.equal(bundledH1.status, 'installed');
  const apps = await (await fetch(`${web.origin}/api/apps`, { headers })).json();
  assert.equal(apps.items.length, 3);
  await host.apps.uninstall('org.oviraptor.hackerone');
  assert.equal(host.apps.list().length, 2);
});

test('ensureBundledApps records missing package errors without blocking an empty host', async t => {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-bundled-missing-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'packages', 'agent-host'), { recursive: true });
  await writeFile(join(root, 'packages/agent-host/bundled-apps.json'), JSON.stringify({
    schemaVersion: 1,
    apps: [{ id: 'org.oviraptor.assets', package: 'apps/org.oviraptor.assets-1.0.0', enable: true, builtin: true, uninstallable: false }],
  }));
  const events = [];
  const host = new AgentHost({ dataDir: join(root, 'data'), onEvent: event => events.push(event) });
  t.after(() => host.close());
  const result = await host.ensureBundledApps({ root });
  assert.equal(result.items[0].installed, false);
  assert.equal(result.items[0].error, 'host_bundled_package_missing');
  assert.deepEqual(host.apps.list(), []);
  assert.ok(events.some(event => event.type === 'application.bundled_failed' && event.code === 'host_bundled_package_missing'));
  assert.equal(host.bundledApps().items[0].error, 'host_bundled_package_missing');
});
