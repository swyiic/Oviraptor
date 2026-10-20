import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { syncDesktopAssets } from '../src/desktop-assets.mjs';
import { ensureAssetInventorySchema } from '../../app-sdk/src/asset-inventory-schema.mjs';

test('desktop mirror observes equal-count edits, removals and replacements without touching other scopes', async () => {
  const directory=await mkdtemp(join(tmpdir(),'ovi-sync-regression-'));
  const desktopDbPath=join(directory,'desktop.sqlite'), inventoryDbPath=join(directory,'inventory.sqlite');
  const desktop=new DatabaseSync(desktopDbPath);
  let inventory;
  try {
    desktop.exec("CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT); CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER); INSERT INTO assets(id,host,title) VALUES(1,'one.example','old'),(2,'two.example','two'); INSERT INTO project_assets VALUES(1,9,0),(2,9,0);");
    const input={desktopDbPath,inventoryDbPath,hostProjectId:1,desktopProjectId:9};
    inventory=new DatabaseSync(inventoryDbPath);
    ensureAssetInventorySchema(inventory);
    assert.equal(syncDesktopAssets(input).synced,2);
    inventory.exec("INSERT INTO inventory(project_id,source,external_id,observation_json,received_at) VALUES(2,'desktop-assets','other','{}','kept'),(1,'explicit-upload','other','{}','kept');");
    desktop.exec("UPDATE assets SET title='changed' WHERE id=1;");
    assert.equal(syncDesktopAssets(input).updated,1);
    assert.equal(JSON.parse(inventory.prepare("SELECT observation_json FROM inventory WHERE project_id=1 AND external_id='1'").get().observation_json).title,'changed');
    assert.equal(syncDesktopAssets(input).unchanged,true);
    desktop.exec('UPDATE project_assets SET is_deleted=1 WHERE asset_id=2;');
    assert.equal(syncDesktopAssets(input).removed,1);
    desktop.exec("INSERT INTO assets(id,host) VALUES(3,'three.example'); UPDATE project_assets SET is_deleted=1 WHERE asset_id=1; INSERT INTO project_assets VALUES(3,9,0);");
    assert.equal(syncDesktopAssets(input).removed,1);
    assert.deepEqual(inventory.prepare("SELECT external_id FROM inventory WHERE project_id=1 AND source='desktop-assets'").all().map(row=>row.external_id),['3']);
    assert.equal(inventory.prepare("SELECT COUNT(*) n FROM inventory WHERE external_id='other'").get().n,2);
  } finally { inventory?.close();desktop.close();await rm(directory,{recursive:true,force:true}); }
});

test('certificate seed summary is project bound, updates on every snapshot and distinguishes missing index', async () => {
  const directory=await mkdtemp(join(tmpdir(),'ovi-seed-summary-'));
  const desktopDbPath=join(directory,'desktop.sqlite'), inventoryDbPath=join(directory,'inventory.sqlite3');
  const desktop=new DatabaseSync(desktopDbPath), inventory=new DatabaseSync(inventoryDbPath);
  try {
    desktop.exec('CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT); CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER);');
    ensureAssetInventorySchema(inventory);
    const input={desktopDbPath,inventoryDbPath,hostProjectId:1,desktopProjectId:9};
    assert.equal(syncDesktopAssets(input).synced,0);
    const summary=()=>JSON.parse(inventory.prepare('SELECT summary_json FROM inventory_discovery_summary WHERE project_id=1').get().summary_json);
    assert.equal(summary().available,false);
    desktop.exec("CREATE TABLE asset_discovery_seeds(project_id INTEGER,host TEXT,source TEXT); INSERT INTO asset_discovery_seeds VALUES(9,'fingerprint','certificate_sha256'),(9,'a.example','certificate_domain'),(9,'a.example','certificate_cn'),(99,'other','certificate_sha256');");
    assert.equal(syncDesktopAssets(input).synced,0);
    assert.equal(summary().total,3);assert.equal(summary().certificateFingerprints,1);assert.equal(summary().certificateNames,1);
    const { createApplication } = await import('../../../apps/assets/application.mjs');
    const { readFile } = await import('node:fs/promises');
    const spec = JSON.parse(await readFile(new URL('../../../apps/assets/app.package.json', import.meta.url), 'utf8'));
    const app = createApplication(spec.manifest);
    await app.onStart({dataDir:directory});
    try {
      const stats=(await app.tools.inventory_stats({}, {hostBoundProjectId:1,grantRevision:1,operationId:'summary-read'})).structuredContent;
      assert.equal(stats.discoverySeeds.certificateFingerprints,1);assert.equal(stats.discoverySeeds.certificateNames,1);
      const other=(await app.tools.inventory_stats({}, {hostBoundProjectId:2,grantRevision:1,operationId:'other-read'})).structuredContent;
      assert.equal(other.discoverySeeds.available,false);
    } finally {await app.onStop();}

    desktop.exec('DELETE FROM asset_discovery_seeds WHERE project_id=9');syncDesktopAssets(input);
    assert.equal(summary().available,true);assert.equal(summary().total,0);
  } finally {inventory.close();desktop.close();await rm(directory,{recursive:true,force:true});}
});
