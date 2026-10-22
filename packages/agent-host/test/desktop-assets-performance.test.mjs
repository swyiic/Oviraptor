import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm, rename } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DesktopAssetSynchronizer } from '../src/desktop-assets.mjs';
import { ensureAssetInventorySchema } from '../../app-sdk/src/asset-inventory-schema.mjs';
import { syncDesktopAssetSnapshot } from '../../app-sdk/src/desktop-asset-sync.mjs';

async function fixture(t, count = 2) {
  const dir = await mkdtemp(join(tmpdir(), 'ovi-sync-worker-'));
  const input = { desktopDbPath: join(dir, 'desktop.sqlite'), inventoryDbPath: join(dir, 'inventory.sqlite'), hostProjectId: 1, desktopProjectId: 9 };
  const desktop = new DatabaseSync(input.desktopDbPath), mirror = new DatabaseSync(input.inventoryDbPath), sync = new DesktopAssetSynchronizer();
  desktop.exec(`PRAGMA journal_mode=WAL;
    CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT);
    CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER);
    CREATE TABLE asset_discovery_scopes(project_id INTEGER PRIMARY KEY,roots_json TEXT);
    INSERT INTO asset_discovery_scopes VALUES(9,'["one.example"]');
    BEGIN;`);
  const insert = desktop.prepare("INSERT INTO assets(id,host,title) VALUES(?,?,'old')"), member = desktop.prepare('INSERT INTO project_assets VALUES(?,9,0)');
  for (let n = 1; n <= count; n++) { insert.run(n, `n${n}.example`); member.run(n); }
  desktop.exec('COMMIT'); ensureAssetInventorySchema(mirror);
  t.after(async () => { await sync.close(); mirror.close(); desktop.close(); await rm(dir, { recursive: true, force: true }); });
  return { dir, input, desktop, mirror, sync };
}

test('large project sync never runs SQLite copy on the Host event loop; duplicate readers share one flight', async t => {
  const { input, sync } = await fixture(t, 30_000);
  let pulses = 0; const heartbeat = setInterval(() => pulses++, 5);
  try {
    const first = sync.sync(input), second = sync.sync(input);
    assert.equal(first, second);
    assert.equal((await first).synced, 30_000);
    assert.ok(pulses > 2, `Host heartbeat must continue during the copy; observed ${pulses}`);
    const cached = await sync.sync(input);
    assert.equal(cached.cached, true); assert.equal(cached.unchanged, true); assert.equal(cached.updated, 0);
  } finally { clearInterval(heartbeat); }
});

test('revision cache invalidates equal-count edits, removals, scope-only changes and mirror damage', async t => {
  const { input, desktop, mirror, sync } = await fixture(t);
  await sync.sync(input); assert.equal((await sync.sync(input)).cached, true);
  desktop.exec("UPDATE assets SET title='new' WHERE id=1");
  assert.equal((await sync.sync(input)).updated, 1);
  assert.equal(JSON.parse(mirror.prepare("SELECT observation_json FROM inventory WHERE external_id='1'").get().observation_json).title, 'new');
  desktop.exec("UPDATE asset_discovery_scopes SET roots_json='[\"other.example\"]'");
  assert.equal((await sync.sync(input)).cached, false);
  assert.deepEqual(JSON.parse(mirror.prepare('SELECT summary_json FROM inventory_discovery_summary').get().summary_json).confirmedScope.roots, ['other.example']);
  desktop.exec('UPDATE project_assets SET is_deleted=1 WHERE asset_id=2');
  assert.equal((await sync.sync(input)).removed, 1);
  mirror.exec("UPDATE inventory SET observation_json='{}' WHERE external_id='1'");
  assert.equal((await sync.sync(input)).updated, 1);
  assert.equal((await sync.sync(input)).cached, true);
});

test('missing source after a successful read fails closed instead of returning a cached snapshot', async t => {
  const { input, sync } = await fixture(t);
  await sync.sync(input); await rename(input.desktopDbPath, input.desktopDbPath + '.moved');
  assert.equal((await sync.sync(input)).reason, 'desktop_sync_failed');
});

test('closing the Host releases an in-flight read and refuses new work', async t => {
  const { input, sync } = await fixture(t, 1000);
  const pending = sync.sync(input); await sync.close();
  assert.equal((await pending).reason, 'desktop_sync_failed');
  await assert.rejects(sync.sync(input), /desktop_synchronizer_closed/);
});

test('rebinding a Host namespace invalidates the former mapping even though the writer connection is unchanged', async t => {
  const { input, desktop, mirror, sync } = await fixture(t);
  desktop.exec("INSERT INTO assets(id,host,title) VALUES(3,'other.test','other'); INSERT INTO project_assets VALUES(3,10,0)");
  await sync.sync(input); assert.equal((await sync.sync(input)).cached, true);
  const other = { ...input, desktopProjectId: 10 };
  assert.equal((await sync.sync(other)).cached, false);
  assert.deepEqual(mirror.prepare('SELECT external_id FROM inventory').all().map(r => r.external_id), ['3']);
  assert.equal((await sync.sync(input)).cached, false);
  assert.deepEqual(mirror.prepare('SELECT external_id FROM inventory ORDER BY external_id').all().map(r => r.external_id), ['1', '2']);
});

test('borrowed persistent writer stays open and distinguishes an external commit immediately after copy', async t => {
  const { input, mirror } = await fixture(t);
  const external = new DatabaseSync(input.inventoryDbPath); t.after(() => external.close());
  const version = () => mirror.prepare('PRAGMA data_version').get().data_version;
  const before = version();
  assert.equal(syncDesktopAssetSnapshot({ ...input, inventoryConnection: mirror }).synced, 2);
  assert.equal(version(), before, 'own commit must not masquerade as external invalidation');
  const borrowed = new Proxy(mirror, { get(target, key) {
    if (key === 'exec') return sql => { target.exec(sql); if (sql === 'COMMIT') external.exec("UPDATE inventory SET observation_json='{}' WHERE external_id='1'"); };
    const value = target[key]; return typeof value === 'function' ? value.bind(target) : value;
  } });
  assert.equal(syncDesktopAssetSnapshot({ ...input, inventoryConnection: borrowed }).complete, true);
  assert.notEqual(version(), before, 'the next cache decision must reject this concurrent-write window');
  assert.equal(syncDesktopAssetSnapshot({ ...input, inventoryConnection: mirror }).updated, 1);
  assert.equal(mirror.prepare('SELECT COUNT(*) n FROM inventory').get().n, 2);
});
