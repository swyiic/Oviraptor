import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { syncDesktopAssets } from '../src/desktop-assets.mjs';
import { InsightLedger } from '../src/insight.mjs';
import { ensureAssetInventorySchema } from '../../app-sdk/src/asset-inventory-schema.mjs';

test('mirror deletion and later insertion cannot attach an old proposal to a different asset', async () => {
  const temporary = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-mirror-identity-'));
  const desktopDbPath = join(temporary, 'desktop.sqlite'), inventoryDbPath = join(temporary, 'inventory.sqlite');
  const desktop = new DatabaseSync(desktopDbPath), inventory = new DatabaseSync(inventoryDbPath);
  try {
    desktop.exec("CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT); CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER); INSERT INTO assets(id,host) VALUES(1,'old.invalid');INSERT INTO project_assets VALUES(1,9,0)");
    ensureAssetInventorySchema(inventory);
    const input = { desktopDbPath, inventoryDbPath, hostProjectId: 1, desktopProjectId: 9 };
    assert.equal(syncDesktopAssets(input).synced, 1);
    const old = inventory.prepare('SELECT id,external_id FROM inventory').get();
    const ledger = new InsightLedger({ db: inventory });
    const proposal = ledger.propose({ projectId: 1, inventoryRecordId: String(old.id), fields: { note: 'belongs to old asset only' }, operationId: 'temporary-operation' });
    desktop.exec('UPDATE project_assets SET is_deleted=1');
    assert.equal(syncDesktopAssets(input).removed, 1);
    desktop.exec("INSERT INTO assets(id,host) VALUES(2,'new.invalid');INSERT INTO project_assets VALUES(2,9,0)");
    assert.equal(syncDesktopAssets(input).synced, 1);
    const current = inventory.prepare('SELECT id,external_id FROM inventory').get();
    assert.equal(current.external_id, '2');
    assert.ok(current.id > old.id);
    const committed = ledger.commit({ projectId: 1, proposalId: proposal.proposalId, operationId: 'temporary-operation', userConfirmed: true });
    assert.notEqual(String(current.id), committed.inventoryRecordId);
    assert.equal(inventory.prepare('SELECT COUNT(*) AS n FROM inventory_insights WHERE inventory_record_id=?').get(String(current.id)).n, 0);
    assert.equal(inventory.prepare('SELECT COUNT(*) AS n FROM insight_proposals').get().n, 1, 'Historical proposals are retained, not deleted to hide the identity mismatch');
  } finally { inventory.close(); desktop.close(); await rm(temporary, { recursive: true, force: true }); }
});
