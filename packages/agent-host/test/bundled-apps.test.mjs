import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
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
  for (const name of ['assets', 'scanning']) {
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
  assert.equal(first.items.length, 2);
  assert.deepEqual(first.items.map(item => item.id).sort(), ['org.oviraptor.assets', 'org.oviraptor.scanning']);
  for (const item of first.items) {
    assert.equal(item.installed, true);
    assert.equal(item.enabled, true);
    assert.equal(item.builtin, true);
    assert.equal(item.uninstallable, false);
    assert.equal(item.error, null);
  }
  const listed = host.apps.list();
  assert.equal(listed.length, 2);
  assert.ok(listed.every(app => app.enabled && app.builtin && app.uninstallable === false));
  assert.equal(listed.find(app => app.id === 'org.oviraptor.scanning').manifest.tools.some(tool => tool.name === 'task_status'), true);
  assert.equal(listed.find(app => app.id === 'org.oviraptor.scanning').manifest.tools.some(tool => tool.name === 'scan_start'), false);

  const second = await host.ensureBundledApps({ root });
  assert.equal(second.items.length, 2);
  assert.equal(host.apps.list().length, 2);
  await assert.rejects(host.apps.uninstall('org.oviraptor.assets'), error => error.code === 'host_app_builtin');
  await assert.rejects(host.apps.uninstall('org.oviraptor.scanning'), error => error.code === 'host_app_builtin');

  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  const bundled = await (await fetch(`${web.origin}/api/apps/bundled`, { headers })).json();
  assert.equal(bundled.items.length, 2);
  assert.ok(bundled.items.every(item => item.enabled && item.status === 'enabled'));
  const apps = await (await fetch(`${web.origin}/api/apps`, { headers })).json();
  assert.equal(apps.items.length, 2);
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


test('ensureBundledApps packages from apps/assets source when prebuilt package is missing', async t => {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-bundled-source-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'packages', 'agent-host'), { recursive: true });
  await mkdir(join(root, 'apps'), { recursive: true });
  await cp(join(repo, 'packages/agent-host/bundled-apps.json'), join(root, 'packages/agent-host/bundled-apps.json'));
  for (const name of ['assets', 'scanning']) {
    await cp(join(repo, 'apps', name), join(root, 'apps', name), { recursive: true });
  }
  const host = new AgentHost({ dataDir: join(root, 'data') });
  t.after(() => host.close());
  const result = await host.ensureBundledApps({ root });
  assert.equal(result.missingManifest, false);
  assert.equal(result.items.length, 2);
  assert.ok(result.items.every(item => item.installed && item.enabled && item.error === null));
  assert.equal(host.apps.list().length, 2);
  assert.ok(existsSync(join(root, '.agent-build', 'apps', 'org.oviraptor.assets-1.0.0', 'manifest.json')));
  assert.ok(existsSync(join(root, '.agent-build', 'apps', 'org.oviraptor.scanning-1.0.0', 'manifest.json')));
});

test('POST /api/apps/bundled/ensure installs builtins and returns shell catalog', async t => {
  const root = await stageRoot(t);
  const host = new AgentHost({ dataDir: join(root, 'data') });
  t.after(() => host.close());
  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  const response = await fetch(`${web.origin}/api/apps/bundled/ensure`, { method: 'POST', headers, body: JSON.stringify({ force: true }) });
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.missingManifest, false);
  assert.equal(body.items.length, 2);
  assert.ok(body.items.every(item => item.installed && item.enabled));
  assert.ok(Array.isArray(body.shell?.items));
  assert.equal(body.shell.items.find(item => item.id === 'asset').installed, true);
  assert.equal(body.shell.items.find(item => item.id === 'asset').enabled, true);
  assert.equal(body.shell.items.find(item => item.id === 'sentinel').installed, true);
});
