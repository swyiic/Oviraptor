import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, mkdir, readFile, writeFile, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { InsightLedger } from '../src/insight.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

test('explicit Assets 1.0 to 1.2 upgrade migrates only an isolated candidate data generation', async () => {
  const temporary = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-owner-upgrade-'));
  const store = new HostStore(join(temporary, 'host.sqlite'));
  const apps = new AppManager(store, join(temporary, 'applications'));
  const id = 'org.oviraptor.assets';
  try {
    const sourceDirectory = fileURLToPath(new URL('../../../apps/assets/', import.meta.url));
    const specification = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
    assert.equal(specification.manifest.version, '1.2.0'); assert.equal(specification.manifest.dataSchemaVersion, 2);
    const oldSource = join(temporary, 'old-source'); await mkdir(oldSource);
    await writeFile(join(oldSource, 'page.html'), '<!doctype html>');
    await writeFile(join(oldSource, 'worker.mjs'), `
      import {readFile} from 'node:fs/promises';
      import {startStdioWorker} from './sdk/worker.mjs';
      import {open} from './sdk/sqlite-data.mjs';
      const manifest=JSON.parse(await readFile(new URL('./manifest.json',import.meta.url),'utf8'));
      let db;
      await startStdioWorker({manifest,tools:{},onStart:async({dataDir})=>{
        db=open(dataDir,'inventory.sqlite3');
        db.exec("CREATE TABLE IF NOT EXISTS inventory(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,source TEXT NOT NULL,external_id TEXT NOT NULL,observation_json TEXT NOT NULL,received_at TEXT NOT NULL,UNIQUE(project_id,source,external_id));INSERT OR IGNORE INTO inventory VALUES(7,1,'desktop-assets','old','{\\\"host\\\":\\\"old.invalid\\\"}','retained')");
        return {dataSchemaVersion:1};
      },onStop:async()=>{db.close();return{busy:false}}}).closed;
    `);
    const old = await createDirectoryPackage({ sourceDirectory: oldSource, outputDirectory: join(temporary, 'old-package'), files: ['worker.mjs', 'page.html'], vendorSdk: true, manifest: { ...specification.manifest, version: '1.0.0', dataSchemaVersion: 1, tools: [], permissions: [] } });
    await apps.install(old.directory); await apps.setEnabled(id, true);
    const oldApp = store.application(id), originalPath = join(apps.dataDirectoryFor(oldApp), 'inventory.sqlite3');
    const original = new DatabaseSync(originalPath);
    const ledger = new InsightLedger({ db: original });
    const proposal = ledger.propose({ projectId: 1, inventoryRecordId: '99', fields: { note: 'historical identity' }, operationId: 'retained-proposal' });
    original.exec('CREATE INDEX retained_inventory_source ON inventory(source)'); original.close();
    const candidate = await createDirectoryPackage({ ...specification, sourceDirectory, outputDirectory: join(temporary, 'candidate-package') });
    await apps.setEnabled(id, false);
    await apps.upgrade(id, candidate.directory, { expectedRevision: store.application(id).revision });
    const upgraded = store.application(id), upgradedPath = join(apps.dataDirectoryFor(upgraded), 'inventory.sqlite3');
    assert.equal(upgraded.version, '1.2.0'); assert.notEqual(upgradedPath, originalPath);
    const previous = new DatabaseSync(originalPath, { readOnly: true });
    const current = new DatabaseSync(upgradedPath, { readOnly: true });
    try {
      assert.doesNotMatch(previous.prepare("SELECT sql FROM sqlite_master WHERE name='inventory'").get().sql, /AUTOINCREMENT/);
      assert.match(current.prepare("SELECT sql FROM sqlite_master WHERE name='inventory'").get().sql, /AUTOINCREMENT/);
      assert.equal(current.prepare('SELECT id FROM inventory').get().id, 7);
      assert.equal(current.prepare('SELECT proposal_id FROM insight_proposals').get().proposal_id, proposal.proposalId);
      assert.ok(current.prepare("SELECT sql FROM sqlite_master WHERE name='retained_inventory_source'").get());
    } finally { previous.close(); current.close(); }
    const project = store.createProject('Temporary owner upgrade'); assert.equal(project.id, 1);
    await apps.setEnabled(id, true);
    apps.setGrants(id, project.id, ['inventory.read', 'inventory.write']);
    await apps.call(id, 'inventory_import', project.id, { source: 'explicit-upload', records: [{ externalId: 'new', host: 'new.invalid' }] });
    const after = new DatabaseSync(upgradedPath, { readOnly: true });
    try { assert.ok(after.prepare("SELECT id FROM inventory WHERE external_id='new'").get().id > 99); }
    finally { after.close(); }
  } finally { await apps.close(); store.close(); await rm(temporary, { recursive: true, force: true }); }
});
