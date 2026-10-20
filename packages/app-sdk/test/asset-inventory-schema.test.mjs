import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { ensureAssetInventorySchema, assertAssetInventorySchema } from '../src/asset-inventory-schema.mjs';
import { ensureInsightSchema, proposeInsight, commitInsight } from '../src/insight.mjs';

const v1 = 'CREATE TABLE inventory(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,source TEXT NOT NULL,external_id TEXT NOT NULL,observation_json TEXT NOT NULL,received_at TEXT NOT NULL,UNIQUE(project_id,source,external_id))';
const insert = (db, externalId) => db.prepare("INSERT INTO inventory(project_id,source,external_id,observation_json,received_at) VALUES(1,'desktop-assets',?,'{}','retained')").run(externalId).lastInsertRowid;
const code = expected => error => error?.code === expected;

test('fresh v2 inventory never reuses deleted record identities', () => {
  const db = new DatabaseSync(':memory:');
  try {
    assert.deepEqual(ensureAssetInventorySchema(db), { version: 2, migrated: false });
    assert.deepEqual(assertAssetInventorySchema(db), { version: 2 });
    const first = insert(db, 'first'); db.exec('DELETE FROM inventory');
    assert.ok(insert(db, 'second') > first);
    assert.equal(ensureAssetInventorySchema(db).migrated, false);
  } finally { db.close(); }
});

test('explicit v1 migration preserves rows, indexes, insight history and historical identity floor', () => {
  const db = new DatabaseSync(':memory:');
  try {
    db.exec(v1);
    insert(db, 'old');
    db.exec('CREATE INDEX inventory_source_custom ON inventory(project_id,source)');
    ensureInsightSchema(db);
    const old = proposeInsight(db, { projectId: 1, inventoryRecordId: '1', fields: { note: 'retained old note' }, operationId: 'old-operation' });
    commitInsight(db, { projectId: 1, proposalId: old.proposalId, operationId: 'old-operation', userConfirmed: true });
    proposeInsight(db, { projectId: 1, inventoryRecordId: '99', fields: { note: 'retained deleted record history' }, operationId: 'deleted-history' });
    const rows = db.prepare('SELECT * FROM inventory').all();
    const history = db.prepare('SELECT * FROM inventory_insights').all();
    const proposals = db.prepare('SELECT * FROM insight_proposals ORDER BY proposal_id').all();
    assert.throws(() => assertAssetInventorySchema(db), code('asset_inventory_schema_upgrade_required'));
    assert.equal(ensureAssetInventorySchema(db).migrated, true);
    assert.deepEqual(db.prepare('SELECT * FROM inventory').all(), rows);
    assert.deepEqual(db.prepare('SELECT * FROM inventory_insights').all(), history);
    assert.deepEqual(db.prepare('SELECT * FROM insight_proposals ORDER BY proposal_id').all(), proposals);
    assert.ok(db.prepare("SELECT sql FROM sqlite_master WHERE name='inventory_source_custom'").get());
    db.exec('DELETE FROM inventory');
    assert.ok(insert(db, 'new') > 99);
  } finally { db.close(); }
});

for (const extra of [
  'ALTER TABLE inventory ADD COLUMN future_field TEXT',
  "CREATE TRIGGER future_inventory_trigger AFTER INSERT ON inventory BEGIN SELECT 1; END",
  'CREATE VIEW future_inventory_view AS SELECT * FROM inventory',
  'CREATE TABLE future_inventory_child(id INTEGER REFERENCES inventory(id))',
  'CREATE TABLE side_effects(id INTEGER); CREATE TRIGGER future_cross_table_trigger AFTER INSERT ON side_effects BEGIN DELETE FROM inventory; END',
]) {
  test(`unknown future schema dependencies fail closed without rebuilding: ${extra.split(' ')[1]}`, () => {
    const db = new DatabaseSync(':memory:');
    try {
      db.exec(v1); insert(db, 'retained'); db.exec(extra);
      const before = db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY name").all();
      assert.throws(() => ensureAssetInventorySchema(db), code('asset_inventory_schema_unsupported'));
      assert.deepEqual(db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY name").all(), before);
      assert.equal(db.prepare('SELECT external_id FROM inventory').get().external_id, 'retained');
    } finally { db.close(); }
  });
}

test('a future owner schema version is not downgraded', () => {
  const db = new DatabaseSync(':memory:');
  try {
    ensureAssetInventorySchema(db);
    db.exec('UPDATE asset_inventory_schema SET version=3');
    assert.throws(() => ensureAssetInventorySchema(db), code('asset_inventory_schema_unsupported'));
    assert.equal(db.prepare('SELECT version FROM asset_inventory_schema').get().version, 3);
  } finally { db.close(); }
});

test('migration failure after table copy rolls back original identities, schema and indexes', () => {
  const db = new DatabaseSync(':memory:');
  try {
    db.exec(v1); insert(db, 'retained'); db.exec('CREATE INDEX retained_index ON inventory(source)');
    const before = db.prepare('SELECT * FROM inventory').all();
    const wrapped = {
      prepare: db.prepare.bind(db),
      exec(sql) { if (sql === 'DROP TABLE asset_inventory_v1_migration') throw Error('temporary injected migration failure'); return db.exec(sql); },
    };
    assert.throws(() => ensureAssetInventorySchema(wrapped), /temporary injected migration failure/);
    assert.deepEqual(db.prepare('SELECT * FROM inventory').all(), before);
    assert.ok(db.prepare("SELECT sql FROM sqlite_master WHERE name='retained_index'").get());
    assert.equal(db.prepare("SELECT COUNT(*) AS n FROM sqlite_master WHERE name='asset_inventory_v1_migration'").get().n, 0);
    assert.doesNotMatch(db.prepare("SELECT sql FROM sqlite_master WHERE name='inventory'").get().sql, /AUTOINCREMENT/);
  } finally { db.close(); }
});
