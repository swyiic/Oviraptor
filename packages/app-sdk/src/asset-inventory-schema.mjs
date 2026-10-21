// Assets owns this schema migration. Host reads may validate v2 but must never
// migrate an installed v1 application's private data as a side effect of chat.
export const ASSET_INVENTORY_DATA_SCHEMA_VERSION = 2;
const columns = 'id,project_id,source,external_id,observation_json,received_at';
const tableDefinition = auto => `(id INTEGER PRIMARY KEY${auto ? ' AUTOINCREMENT' : ''},project_id INTEGER NOT NULL,source TEXT NOT NULL,external_id TEXT NOT NULL,observation_json TEXT NOT NULL,received_at TEXT NOT NULL,UNIQUE(project_id,source,external_id))`;
const versionTable = 'CREATE TABLE asset_inventory_schema(singleton INTEGER PRIMARY KEY CHECK(singleton=1),version INTEGER NOT NULL)';
const canonical = sql => String(sql ?? '').replace(/\bIF\s+NOT\s+EXISTS\b/giu, '').replace(/[\s"`\[\]]/gu, '').replace(/;$/u, '').toLowerCase();
const fail = code => Object.assign(Error(code), { code, retryable: false });

function layout(db) {
  const row = db.prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='inventory'").get();
  if (!row) return 0;
  const sql = canonical(row.sql);
  if (sql === canonical(`CREATE TABLE inventory${tableDefinition(false)}`)) return 1;
  if (sql === canonical(`CREATE TABLE inventory${tableDefinition(true)}`)) return 2;
  throw fail('asset_inventory_schema_unsupported');
}

function recordedVersion(db) {
  const table = db.prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='asset_inventory_schema'").get();
  if (!table) return null;
  if (canonical(table.sql) !== canonical(versionTable)) throw fail('asset_inventory_schema_unsupported');
  try {
    const rows = db.prepare('SELECT singleton,version FROM asset_inventory_schema').all();
    if (rows.length !== 1 || rows[0].singleton !== 1 || rows[0].version !== 2) throw fail('asset_inventory_schema_unsupported');
    return rows[0].version;
  } catch { throw fail('asset_inventory_schema_unsupported'); }
}

export function assertAssetInventorySchema(db) {
  if (layout(db) !== 2 || recordedVersion(db) !== 2) throw fail('asset_inventory_schema_upgrade_required');
  return { version: ASSET_INVENTORY_DATA_SCHEMA_VERSION };
}

function identityFloor(db) {
  let maximum = db.prepare('SELECT COALESCE(MAX(id),0) AS id FROM inventory').get().id;
  for (const table of ['insight_proposals', 'inventory_insights']) {
    if (!db.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?").get(table)) continue;
    for (const row of db.prepare(`SELECT inventory_record_id FROM ${table}`).iterate()) {
      const value = String(row.inventory_record_id);
      if (!/^[0-9]+$/u.test(value)) continue;
      const number = Number(value);
      if (!Number.isSafeInteger(number)) throw fail('asset_inventory_identity_exhausted');
      maximum = Math.max(maximum, number);
    }
  }
  if (!Number.isSafeInteger(maximum) || maximum < 0 || maximum >= Number.MAX_SAFE_INTEGER) throw fail('asset_inventory_identity_exhausted');
  return maximum;
}

function migrationIndices(db) {
  // Unknown future dependency semantics are not permission to rebuild a table.
  for (const trigger of db.prepare("SELECT tbl_name,sql FROM sqlite_master WHERE type='trigger'").iterate()) {
    if (trigger.tbl_name === 'inventory' || /\binventory\b/iu.test(trigger.sql ?? '')) throw fail('asset_inventory_schema_unsupported');
  }
  for (const view of db.prepare("SELECT sql FROM sqlite_master WHERE type='view'").iterate()) {
    if (/\binventory\b/iu.test(view.sql ?? '')) throw fail('asset_inventory_schema_unsupported');
  }
  for (const table of db.prepare("SELECT name FROM sqlite_master WHERE type='table'").iterate()) {
    const quoted = String(table.name).replaceAll('"', '""');
    if (db.prepare(`PRAGMA foreign_key_list("${quoted}")`).all().some(key => key.table === 'inventory')) throw fail('asset_inventory_schema_unsupported');
  }
  return db.prepare("SELECT sql FROM sqlite_master WHERE type='index' AND tbl_name='inventory' AND sql IS NOT NULL ORDER BY name").all().map(row => row.sql);
}

/** Explicit owner-only, transactional migration of the exact known v1 layout. */
export function ensureAssetInventorySchema(db) {
  db.exec('BEGIN IMMEDIATE');
  try {
    const version = layout(db), recorded = recordedVersion(db);
    if (recorded !== null && version !== 2) throw fail('asset_inventory_schema_unsupported');
    if (version === 0) db.exec(`CREATE TABLE inventory${tableDefinition(true)}`);
    if (version === 1) {
      const indices = migrationIndices(db);
      if (db.prepare("SELECT 1 FROM sqlite_master WHERE name='asset_inventory_v1_migration'").get()) throw fail('asset_inventory_schema_unsupported');
      const floor = identityFloor(db);
      db.exec('ALTER TABLE inventory RENAME TO asset_inventory_v1_migration');
      db.exec(`CREATE TABLE inventory${tableDefinition(true)}`);
      db.exec(`INSERT INTO inventory(${columns}) SELECT ${columns} FROM asset_inventory_v1_migration`);
      db.exec('DROP TABLE asset_inventory_v1_migration');
      for (const sql of indices) db.exec(sql);
      setIdentityFloor(db, floor);
    }
    setIdentityFloor(db, identityFloor(db));
    if (recorded === null) db.exec(versionTable);
    db.prepare('INSERT INTO asset_inventory_schema(singleton,version) VALUES(1,2) ON CONFLICT(singleton) DO UPDATE SET version=excluded.version').run();
    db.exec('COMMIT');
    return { version: ASSET_INVENTORY_DATA_SCHEMA_VERSION, migrated: version === 1 };
  } catch (error) {
    try { db.exec('ROLLBACK'); } catch { /* transaction already closed */ }
    throw error;
  }
}

function setIdentityFloor(db, floor) {
  const prior = db.prepare("SELECT seq FROM sqlite_sequence WHERE name='inventory'").get();
  if (prior) db.prepare("UPDATE sqlite_sequence SET seq=MAX(seq,?) WHERE name='inventory'").run(floor);
  else db.prepare("INSERT INTO sqlite_sequence(name,seq) VALUES('inventory',?)").run(floor);
}
