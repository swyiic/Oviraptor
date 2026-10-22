import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, readFile, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { ensureAssetInventorySchema } from '../../app-sdk/src/asset-inventory-schema.mjs';

// Real Host/AppManager/package/SQLite state; worker responses are injected so no
// installed application, external request, or real user inventory is touched.
async function fixture(t, { externalId, malformedDesktop = false, legacyInventory = false } = {}) {
  const temporary = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-desktop-read-guard-'));
  const host = new AgentHost({ dataDir: join(temporary, 'host'), runtime: { status: { pid: 9876 }, close: async () => {} } });
  t.after(async () => { host.runs.clear(); await host.close(); await rm(temporary, { recursive: true, force: true }); });
  let calls = 0;
  host.apps.processFactory = () => ({ stopped: false, start: async () => {}, stop: async () => {}, call: async () => {
    calls++;
    return { content: [{ type: 'text', text: JSON.stringify({ total: 1, title: 'previous inventory observation' }) }] };
  } });
  const sourceDirectory = fileURLToPath(new URL('../../../examples/echo-app/', import.meta.url));
  const original = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const manifest = { ...original.manifest, id: 'org.oviraptor.assets', tools: [{ name: 'inventory_stats', description: 'Read temporary inventory', permission: 'inventory.read', inputSchema: { type: 'object', properties: {}, additionalProperties: false } }], permissions: [{ id: 'inventory.read', description: 'Read temporary project inventory' }] };
  const packaged = await createDirectoryPackage({ ...original, manifest, sourceDirectory, outputDirectory: join(temporary, 'package') });
  await host.apps.install(packaged.directory); await host.apps.setEnabled(manifest.id, true);
  const project = host.store.createProject('Temporary guarded asset read', { externalId });
  const conversation = host.store.createConversation(project.id);
  host.apps.setGrants(manifest.id, project.id, ['inventory.read'], undefined, host.runtimeIdentity());
  host.runs.set(conversation.id, { id: 'temporary-read-run', conversationId: conversation.id, state: 'running' });
  host.mcpScopes.set('temporary-scope', { conversationId: conversation.id, runtimePid: 9876 });
  const inventoryPath = join(host.apps.dataDirectoryFor(host.store.application(manifest.id)), 'inventory.sqlite3');
  const inventory = new DatabaseSync(inventoryPath);
  if (legacyInventory) inventory.exec('CREATE TABLE inventory(id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL, source TEXT NOT NULL, external_id TEXT NOT NULL, observation_json TEXT NOT NULL, received_at TEXT NOT NULL, UNIQUE(project_id,source,external_id))');
  else ensureAssetInventorySchema(inventory);
  inventory.prepare('INSERT INTO inventory(project_id,source,external_id,observation_json,received_at) VALUES(?,?,?,?,?)').run(project.id, 'desktop-assets', '1', JSON.stringify({ title: 'previous desktop observation' }), '2026-01-01T00:00:00Z');
  inventory.close();
  if (malformedDesktop) {
    const desktop = new DatabaseSync(join(temporary, 'oviraptor.sqlite3'));
    desktop.exec('CREATE TABLE unrelated(id INTEGER PRIMARY KEY)'); desktop.close();
  }
  return {
    host, conversation, inventoryPath, calls: () => calls,
    read: () => host.mcp('temporary-scope', conversation.id, 'tools/call', { name: 'org.oviraptor.assets__inventory_stats', arguments: {} }),
  };
}

test('a mapped desktop sync failure blocks reading the previous mirror', async t => {
  const f = await fixture(t, { externalId: '7', malformedDesktop: true });
  await assert.rejects(f.read(), error => {
    assert.equal(error.code, 'host_desktop_asset_sync_failed');
    assert.equal(error.status, 503);
    assert.equal(error.retryable, false);
    return true;
  });
  assert.equal(f.calls(), 0, 'The app worker must never return the old mirror after a known synchronization failure');
  const inventory = new DatabaseSync(f.inventoryPath, { readOnly: true });
  try { assert.equal(JSON.parse(inventory.prepare('SELECT observation_json FROM inventory').get().observation_json).title, 'previous desktop observation'); }
  finally { inventory.close(); }
});

for (const externalId of [undefined, 'app-only-project']) {
  test(`an unmapped app-only project can read its inventory (${externalId ?? 'no external ID'})`, async t => {
    const f = await fixture(t, { externalId, malformedDesktop: true });
    assert.match((await f.read()).content[0].text, /previous inventory observation/);
    assert.equal(f.calls(), 1);
  });
}

test('a mapped project with a missing desktop source refuses to return its previous inventory mirror', async t => {
  const f = await fixture(t, { externalId: '7' });
  await assert.rejects(f.read(), error => error.code === 'host_desktop_asset_sync_failed' && error.retryable === false);
  assert.equal(f.calls(), 0);
});

test('mapped v1 owner data requires explicit application upgrade and is never silently migrated by a read', async t => {
  const f = await fixture(t, { externalId: '7', malformedDesktop: true, legacyInventory: true });
  await assert.rejects(f.read(), error => error.code === 'host_desktop_inventory_upgrade_required' && error.retryable === false);
  assert.equal(f.calls(), 0);
  const inventory = new DatabaseSync(f.inventoryPath, { readOnly: true });
  try {
    assert.doesNotMatch(inventory.prepare("SELECT sql FROM sqlite_master WHERE name='inventory'").get().sql, /AUTOINCREMENT/);
    assert.equal(inventory.prepare('SELECT COUNT(*) AS n FROM inventory').get().n, 1);
    assert.equal(inventory.prepare("SELECT COUNT(*) AS n FROM sqlite_master WHERE name='asset_inventory_schema'").get().n, 0);
  } finally { inventory.close(); }
});
