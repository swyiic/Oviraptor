import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { shellBuiltinPlugins, shellCatalog, shellEnrichmentForApp } from '../src/shell-apps.mjs';

test('shell registry exposes asset, sentinel and hackerone contributions', () => {
  assert.deepEqual(shellBuiltinPlugins().map(item => item.id), ['asset', 'sentinel', 'hackerone']);
  assert.equal(shellBuiltinPlugins().find(item => item.id === 'sentinel').capabilities.find(item => item.id === 'scanning.native-investigation').requiresModel, true);
  assert.deepEqual(shellEnrichmentForApp('org.oviraptor.assets').shellModuleId, 'asset');
  assert.equal(shellEnrichmentForApp('example.unknown'), undefined);
});

test('GET /api/apps/shell returns navigation even with zero installed apps', async t => {
  const dataDir = await mkdtemp(join(tmpdir(), 'oviraptor-shell-apps-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  const host = new AgentHost({ dataDir });
  t.after(() => host.close());
  const catalog = host.shellApps();
  assert.equal(catalog.schemaVersion, 1);
  assert.deepEqual(catalog.items.map(item => item.id), ['asset', 'sentinel', 'hackerone']);
  assert.equal(catalog.items.find(item => item.id === 'asset').installed, false);
  // Missing host packages must still report enabled so the desktop sidebar stays populated.
  assert.equal(catalog.items.find(item => item.id === 'asset').enabled, true);
  assert.equal(catalog.items.find(item => item.id === 'sentinel').enabled, true);
  assert.equal(catalog.items.find(item => item.id === 'hackerone').status, 'shell-only');
  assert.deepEqual(catalog.items.find(item => item.id === 'asset').pages.map(page => page.view),
    ['dashboard', 'tasks', 'assets', 'ownership', 'exposure', 'changes', 'quarantine', 'query', 'logs']);

  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token };
  const body = await (await fetch(`${web.origin}/api/apps/shell`, { headers })).json();
  assert.deepEqual(body.items.map(item => item.id), ['asset', 'sentinel', 'hackerone']);
  const empty = shellCatalog([]);
  assert.equal(empty.items.every(item => item.id === 'hackerone' || item.installed === false), true);
});
