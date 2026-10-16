import { randomUUID } from 'node:crypto';
import { isAbsolute } from 'node:path';
import { ChildProcess } from 'node:child_process';

const error = code => Object.assign(new Error(code), { code, status: 409 });
const stamp = () => new Date().toISOString();
const states = new Set(['starting', 'running', 'unresolved', 'stopped']);
const nodeUpgradeProofs = new WeakMap();

// An in-memory capability, never serialized or accepted from a package/RPC.
// A real stopped Node child establishes only process custody, not that its data
// has been correctly migrated by the native candidate.
export function issueNodeUpgradeProof(original, worker, candidate) {
  if (original.manifest?.schemaVersion !== 1 || candidate.manifest?.schemaVersion !== 2
    || worker.native !== false || worker.active !== 0 || worker.stopped !== true || worker.idleStopConfirmed !== true || worker.exitObserved !== true
    || !(worker.child instanceof ChildProcess) || worker.child.exitCode === null && worker.child.signalCode === null
    || worker.app.id !== original.id || worker.app.version !== original.version || worker.app.digest !== original.digest || worker.app.path !== original.path
    || original.id !== candidate.id || typeof worker.dataDir !== 'string' || !isAbsolute(worker.dataDir)) throw error('host_app_upgrade_owner_proof_invalid');
  const proof = Object.freeze(Object.create(null));
  nodeUpgradeProofs.set(proof, {
    original: { id: original.id, version: original.version, digest: original.digest, path: original.path, revision: original.revision,
      dataPath: worker.dataDir, dataVersion: original.data_version, manifest: JSON.stringify(original.manifest) },
    candidate: { id: candidate.id, version: candidate.version, digest: candidate.digest, path: candidate.path, dataPath: candidate.data_path, dataVersion: candidate.data_version },
  });
  return proof;
}

function validNodeUpgrade(db, app, dataPath, proof) {
  const binding = proof && nodeUpgradeProofs.get(proof);
  if (!binding) return false;
  const old = binding.original, next = binding.candidate;
  if (next.id !== app.id || next.version !== app.version || next.digest !== app.digest || next.path !== app.path || next.dataPath !== dataPath || next.dataVersion !== app.data_version) return false;
  const current = db.prepare('SELECT * FROM applications WHERE id=?').get(old.id);
  return Boolean(current && current.enabled === 0 && current.version === old.version && current.digest === old.digest && current.path === old.path
    && current.revision === old.revision && current.data_path === old.dataPath && current.data_version === old.dataVersion && current.manifest === old.manifest);
}

/** Host process custody only. No application business data, lease or financial SQL. */
export function initializeNativeOwners(db) {
  db.exec(`CREATE TABLE IF NOT EXISTS native_application_owners(
    app_id TEXT PRIMARY KEY,version TEXT NOT NULL,digest TEXT NOT NULL,data_path TEXT,
    generation_nonce TEXT NOT NULL UNIQUE,state TEXT NOT NULL CHECK(state IN ('starting','running','unresolved','stopped')),
    pid INTEGER,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
    CREATE TRIGGER IF NOT EXISTS native_owner_refuse_unresolved_delete BEFORE DELETE ON native_application_owners
    WHEN OLD.state<>'stopped' BEGIN SELECT RAISE(ABORT,'native owner unresolved');END;`);
  db.exec('BEGIN IMMEDIATE');
  try {
    const version = db.prepare("SELECT value FROM host_settings WHERE key='native_owner_ledger_version'").get();
    if (version && version.value !== '1') throw error('host_app_owner_schema_invalid');
    if (!version) {
      // Older Host databases cannot prove that a previously installed native
      // worker exited idle. Neither a dead PID nor disabled state proves that.
      for (const app of db.prepare('SELECT id,version,digest,data_path,manifest FROM applications').all()) {
        let manifest;
        try { manifest = JSON.parse(app.manifest); } catch { throw error('host_app_owner_schema_invalid'); }
        if (manifest.schemaVersion !== 2) continue;
        const now = stamp();
        db.prepare(`INSERT INTO native_application_owners(app_id,version,digest,data_path,generation_nonce,state,pid,created_at,updated_at)
          VALUES(?,?,?,?,?,'unresolved',NULL,?,?) ON CONFLICT(app_id) DO NOTHING`)
          .run(app.id, app.version, app.digest, app.data_path, randomUUID(), now, now);
      }
      db.prepare("INSERT INTO host_settings(key,value) VALUES('native_owner_ledger_version','1')").run();
    }
    db.exec('COMMIT');
  } catch (failure) { db.exec('ROLLBACK'); throw failure; }
}

export class NativeOwners {
  constructor(db) { this.db = db; }
  get(id) { return this.db.prepare('SELECT * FROM native_application_owners WHERE app_id=?').get(id) ?? null; }
  assertSettled(id) {
    const row = this.get(id);
    if (row && row.state !== 'stopped') throw error('host_app_owner_unresolved');
    return row;
  }
  reserve(app, dataPath, nodeUpgradeProof) {
    if (typeof app.id !== 'string' || !app.id || app.id.length > 128 || typeof app.version !== 'string' || !app.version || app.version.length > 80
      || !/^[0-9a-f]{64}$/u.test(app.digest) || typeof dataPath !== 'string' || !isAbsolute(dataPath) || dataPath.length > 8192 || dataPath.includes('\0')) throw error('host_app_owner_identity_invalid');
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const previous = this.assertSettled(app.id);
      const fresh = app.enabled === false && app.revision === 1 && app.data_path == null && app.data_version === 'legacy';
      if (!previous && !fresh && !validNodeUpgrade(this.db, app, dataPath, nodeUpgradeProof)) throw error('host_app_owner_unresolved');
      const nonce = randomUUID(), now = stamp();
      this.db.prepare(`INSERT INTO native_application_owners(app_id,version,digest,data_path,generation_nonce,state,pid,created_at,updated_at)
        VALUES(?,?,?,?,?,'starting',NULL,?,?) ON CONFLICT(app_id) DO UPDATE SET version=excluded.version,digest=excluded.digest,data_path=excluded.data_path,
        generation_nonce=excluded.generation_nonce,state='starting',pid=NULL,created_at=excluded.created_at,updated_at=excluded.updated_at WHERE native_application_owners.state='stopped'`)
        .run(app.id, app.version, app.digest, dataPath, nonce, now, now);
      const actual = this.get(app.id);
      if (actual?.generation_nonce !== nonce || actual.state !== 'starting') throw error('host_app_owner_generation_conflict');
      this.db.exec('COMMIT');
      if (nodeUpgradeProof) nodeUpgradeProofs.delete(nodeUpgradeProof);
      return Object.freeze({ appId: app.id, version: app.version, digest: app.digest, dataPath, nonce });
    } catch (failure) { this.db.exec('ROLLBACK'); throw failure; }
  }
  transition(ticket, event) {
    if (!event || !states.has(event.state) || event.pid !== undefined && event.pid !== null && (!Number.isSafeInteger(event.pid) || event.pid < 1)) throw error('host_app_owner_event_invalid');
    if (event.state === 'stopped' && (event.idleStopConfirmed !== true || event.exitObserved !== true)) throw error('host_app_stop_unconfirmed');
    const current = this.get(ticket.appId);
    if (!current || current.generation_nonce !== ticket.nonce || current.version !== ticket.version || current.digest !== ticket.digest || current.data_path !== ticket.dataPath) throw error('host_app_owner_generation_conflict');
    if (current.state === 'stopped') {
      if (event.state === 'stopped') return current;
      throw error('host_app_owner_generation_conflict');
    }
    if (current.state === 'unresolved' && !['unresolved', 'stopped'].includes(event.state)
      || current.state === 'running' && event.state === 'starting') throw error('host_app_owner_unresolved');
    const changed = this.db.prepare(`UPDATE native_application_owners SET state=?,pid=COALESCE(?,pid),updated_at=?
      WHERE app_id=? AND generation_nonce=? AND version=? AND digest=? AND data_path=? AND state=?`)
      .run(event.state, event.pid ?? null, stamp(), ticket.appId, ticket.nonce, ticket.version, ticket.digest, ticket.dataPath, current.state);
    if (changed.changes !== 1) throw error('host_app_owner_generation_conflict');
    return this.get(ticket.appId);
  }
}
