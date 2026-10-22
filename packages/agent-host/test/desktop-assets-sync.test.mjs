import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { syncDesktopAssets } from '../src/desktop-assets.mjs';
import { ensureAssetInventorySchema } from '../../app-sdk/src/asset-inventory-schema.mjs';

test('confirmed scope and bulk review are read-only project-bound context, refreshed even with equal inventory counts', async () => {
  const directory=await mkdtemp(join(tmpdir(),'ovi-scope-context-'));
  const desktopDbPath=join(directory,'desktop.sqlite'),inventoryDbPath=join(directory,'inventory.sqlite3');
  const desktop=new DatabaseSync(desktopDbPath),inventory=new DatabaseSync(inventoryDbPath);
  try {
    desktop.exec(`CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT);
      CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER);
      CREATE TABLE asset_discovery_scopes(project_id INTEGER PRIMARY KEY,roots_json TEXT);
      INSERT INTO asset_discovery_scopes VALUES(9,'["confirmed.example"]'),(99,'["other.example"]');
      CREATE TABLE asset_seed_reviews(id TEXT PRIMARY KEY,project_id INTEGER,organization TEXT,status TEXT,total INTEGER,processed INTEGER,requests INTEGER,input_tokens INTEGER,output_tokens INTEGER,unknown_requests INTEGER,updated_at TEXT,recipient TEXT);
      CREATE TABLE asset_seed_review_items(run_id TEXT,state TEXT,advice TEXT);
      INSERT INTO asset_seed_reviews VALUES('ours',9,'明确单位','paused',98000,98000,1,1909,861,0,'saved','private-recipient-never-exported'),('theirs',99,'其他项目','completed',2,2,5,5000,2000,1,'saved','');
      INSERT INTO asset_seed_review_items VALUES('ours','done','retain'),('ours','done','review'),('ours','pending','review'),('theirs','done','exclude');`);
    ensureAssetInventorySchema(inventory);
    const input={desktopDbPath,inventoryDbPath,hostProjectId:1,desktopProjectId:9};
    assert.equal(syncDesktopAssets(input).synced,0);
    const summary=()=>JSON.parse(inventory.prepare('SELECT summary_json FROM inventory_discovery_summary WHERE project_id=1').get().summary_json);
    assert.deepEqual(summary().confirmedScope.roots,['confirmed.example']);
    assert.equal(summary().confirmedScope.executionPermissionGranted,false);
    assert.equal(summary().seedReview.id,'ours');assert.equal(summary().seedReview.groups,3);
    assert.deepEqual(summary().seedReview.counts,{retain:1,evidence:1,pending:1});
    assert.equal(summary().seedReview.processedAssets,98000);assert.equal(summary().seedReview.modelRequests,1);
    desktop.exec(`ALTER TABLE asset_seed_review_items ADD COLUMN kind TEXT DEFAULT 'domain_group';
      ALTER TABLE asset_seed_review_items ADD COLUMN evidence_json TEXT DEFAULT '{}';
      ALTER TABLE asset_seed_review_items ADD COLUMN decision TEXT DEFAULT 'pending';
      UPDATE asset_seed_review_items SET kind='certificate_metadata' WHERE state='done' AND advice='review';
      UPDATE asset_seed_review_items SET decision='rejected' WHERE state='done' AND advice='retain';`);
    syncDesktopAssets(input);
    assert.deepEqual(summary().seedReview.queueCounts,{evidence:1,pending:1});
    assert.deepEqual(summary().seedReview.manualCounts,{rejected:1});
    assert.equal(summary().seedReview.counts.evidence,2);

    assert.equal(summary().seedReview.adviceIsNotScopeApproval,true);assert.equal(summary().seedReview.storedStatusOnly,true);
    assert.equal(JSON.stringify(summary()).includes('other.example'),false);
    assert.equal(JSON.stringify(summary()).includes('private-recipient-never-exported'),false);
    desktop.prepare('UPDATE asset_discovery_scopes SET roots_json=? WHERE project_id=9').run('["updated.example"]');
    assert.equal(syncDesktopAssets(input).unchanged,true);assert.deepEqual(summary().confirmedScope.roots,['updated.example']);
    desktop.prepare('UPDATE asset_discovery_scopes SET roots_json=? WHERE project_id=9').run('[]');
    syncDesktopAssets(input);assert.equal(summary().confirmedScope.available,true);assert.deepEqual(summary().confirmedScope.roots,[]);
    const roots=Array.from({length:100},(_,i)=>`n${i}.${('a'.repeat(60)+'.').repeat(3)}example.com`);
    desktop.prepare('UPDATE asset_discovery_scopes SET roots_json=? WHERE project_id=9').run(JSON.stringify(roots));
    syncDesktopAssets(input);assert.equal(summary().confirmedScope.truncated,true);assert.equal(summary().confirmedScope.totalRoots,100);
    assert.ok(summary().confirmedScope.roots.length>0&&summary().confirmedScope.roots.length<100);
    assert.ok(Buffer.byteLength(JSON.stringify(summary().confirmedScope))<18_000);
    desktop.prepare('UPDATE asset_discovery_scopes SET roots_json=? WHERE project_id=9').run('{"roots":"invalid"}');
    assert.equal(syncDesktopAssets(input).synced,0);assert.equal(summary().confirmedScope.available,false);
    assert.equal(summary().confirmedScope.reason,'saved_scope_invalid');
    assert.equal(desktop.prepare('SELECT roots_json FROM asset_discovery_scopes WHERE project_id=9').get().roots_json,'{"roots":"invalid"}');
    syncDesktopAssets({...input,hostProjectId:2,desktopProjectId:99});
    const other=JSON.parse(inventory.prepare('SELECT summary_json FROM inventory_discovery_summary WHERE project_id=2').get().summary_json);
    assert.deepEqual(other.confirmedScope.roots,['other.example']);assert.equal(other.seedReview.id,'theirs');
  } finally {inventory.close();desktop.close();await rm(directory,{recursive:true,force:true});}
});

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

test('certificate coverage separates source metadata from raw material, excludes deleted and other project assets', async () => {
  const directory=await mkdtemp(join(tmpdir(),'ovi-cert-coverage-'));
  const desktopDbPath=join(directory,'desktop.sqlite'),inventoryDbPath=join(directory,'inventory.sqlite');
  const desktop=new DatabaseSync(desktopDbPath),inventory=new DatabaseSync(inventoryDbPath);
  try {
    desktop.exec('CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT,extra_json TEXT); CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER);');
    ensureAssetInventorySchema(inventory);
    const insert=desktop.prepare('INSERT INTO assets(id,link,extra_json) VALUES(?,?,?)');
    const values=[
      ['https://metadata.example',JSON.stringify({'cert.subject.cn':'same.example','cert.sn':'123'})],
      ['https://saved.example',JSON.stringify({probe_cert_sha256:'a'.repeat(64),probe_cert_der_base64:'candidate'})],
      ['https://invalid.example','{invalid json'],
      ['http://plain.example','{}'],
      ['https://deleted.example',JSON.stringify({probe_cert_sha256:'b'.repeat(64)})],
      ['https://other.example',JSON.stringify({probe_cert_sha256:'c'.repeat(64)})],
    ];
    values.forEach(([link,extra],i)=>insert.run(i+1,link,extra));
    desktop.exec('INSERT INTO project_assets VALUES(1,9,0),(2,9,0),(3,9,0),(4,9,0),(5,9,1),(6,99,0)');
    const input={desktopDbPath,inventoryDbPath,hostProjectId:1,desktopProjectId:9};
    assert.equal(syncDesktopAssets(input).synced,4);
    const summary=()=>JSON.parse(inventory.prepare('SELECT summary_json FROM inventory_discovery_summary WHERE project_id=1').get().summary_json);
    const coverage=summary().certificateCoverage;
    assert.equal(coverage.available,true);assert.equal(coverage.assets,4);assert.equal(coverage.httpsAssets,3);
    assert.equal(coverage.assetsWithCertificateMetadata,1);assert.equal(coverage.assetsWithRawCertificateCandidate,1);
    assert.equal(coverage.assetsWithSavedFingerprint,1);assert.equal(coverage.httpsAssetsMissingSavedCertificate,2);
    assert.equal(coverage.localExtractionRequestsNetwork,false);assert.equal(summary().available,false);
    desktop.prepare('UPDATE assets SET extra_json=? WHERE id=3').run(JSON.stringify({probe_cert_sha256:'z'.repeat(64)}));
    syncDesktopAssets(input);assert.equal(summary().certificateCoverage.assetsWithSavedFingerprint,1);
    desktop.exec('UPDATE project_assets SET is_deleted=1 WHERE project_id=9');syncDesktopAssets(input);
    assert.equal(summary().certificateCoverage.assets,0);assert.equal(summary().certificateCoverage.httpsAssets,0);
  } finally {inventory.close();desktop.close();await rm(directory,{recursive:true,force:true});}
});
