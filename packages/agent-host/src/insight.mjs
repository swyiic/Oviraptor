// Project-scoped insight propose/commit (Wave3 Asset teammate contract).
// Persists to inventory.sqlite3 (insight_proposals + inventory_insights) when a db/dbPath is provided.
// Ownership/auth stay off the tool surface. Host HTTP commit still requires userConfirmed === true.
import { randomUUID } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { existsSync, mkdirSync, lstatSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { hostError } from './store.mjs';

export const INSIGHT_FIELD_WHITELIST = Object.freeze(['tags', 'note', 'labels', 'classification']);
export const INSIGHT_PROPOSAL_TTL_MS = 15 * 60 * 1000;
export const INSIGHT_INVENTORY_FILENAME = 'inventory.sqlite3';

const FAIL = Object.freeze({
  proposal_expired: 'proposal_expired',
  mismatch: 'mismatch',
  base_stale: 'base_stale',
  field_denied: 'field_denied',
  confirm_required: 'confirm_required',
});

function insightError(code, status = 400) {
  return hostError(`host_insight_${code}`, status, { insightCode: code });
}

export function ensureInsightSchema(db) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS insight_proposals(
      proposal_id TEXT PRIMARY KEY,
      project_id INTEGER NOT NULL,
      inventory_record_id TEXT NOT NULL,
      fields_json TEXT NOT NULL,
      base_version INTEGER NOT NULL DEFAULT 0,
      operation_id TEXT NOT NULL,
      created_at_ms INTEGER NOT NULL,
      expires_at_ms INTEGER NOT NULL,
      status TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS inventory_insights(
      id INTEGER PRIMARY KEY,
      project_id INTEGER NOT NULL,
      inventory_record_id TEXT NOT NULL,
      proposal_id TEXT NOT NULL UNIQUE,
      operation_id TEXT NOT NULL,
      fields_json TEXT NOT NULL,
      base_version INTEGER NOT NULL DEFAULT 0,
      version INTEGER NOT NULL,
      committed_at TEXT NOT NULL,
      UNIQUE(project_id, operation_id)
    );
    CREATE INDEX IF NOT EXISTS inventory_insights_record
      ON inventory_insights(project_id, inventory_record_id);
  `);
}

/** Open (or create) inventory.sqlite3 for insight persistence. Caller must close. */
export function openInsightInventoryDb(dbPath) {
  if (typeof dbPath !== 'string' || !dbPath) throw hostError('host_insight_db_invalid');
  mkdirSync(dirname(dbPath), { recursive: true, mode: 0o700 });
  if (existsSync(dbPath) && (lstatSync(dbPath).isSymbolicLink() || !lstatSync(dbPath).isFile())) {
    throw hostError('host_insight_db_invalid');
  }
  const db = new DatabaseSync(dbPath);
  db.exec('PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA busy_timeout=2000;');
  ensureInsightSchema(db);
  return db;
}

function normalizeFields(fields) {
  if (!fields || typeof fields !== 'object' || Array.isArray(fields)) {
    throw insightError(FAIL.field_denied);
  }
  const clean = {};
  for (const [key, value] of Object.entries(fields)) {
    if (!INSIGHT_FIELD_WHITELIST.includes(key)) throw insightError(FAIL.field_denied);
    if (value === undefined) continue;
    if (key === 'tags' || key === 'labels') {
      if (!Array.isArray(value) || value.length > 32 || value.some(item => typeof item !== 'string' || item.length > 64)) {
        throw insightError(FAIL.field_denied);
      }
      clean[key] = value.map(item => item.trim()).filter(Boolean);
    } else if (typeof value !== 'string' || value.length > 2048) {
      throw insightError(FAIL.field_denied);
    } else {
      clean[key] = value;
    }
  }
  if (Object.keys(clean).length === 0) throw insightError(FAIL.field_denied);
  return clean;
}


export function formatInsightDiffSummary({ inventoryRecordId, fields, baseVersion } = {}) {
  const record = typeof inventoryRecordId === 'string' && inventoryRecordId ? inventoryRecordId : '?';
  const version = Number.isSafeInteger(baseVersion) ? baseVersion : 0;
  const patch = fields && typeof fields === 'object' && !Array.isArray(fields) ? fields : {};
  const parts = [];
  for (const key of INSIGHT_FIELD_WHITELIST) {
    if (!(key in patch)) continue;
    const value = patch[key];
    if (Array.isArray(value)) parts.push(`${key}=[${value.join(', ')}]`);
    else parts.push(`${key}=${String(value)}`);
  }
  return `inventory ${record} @ base v${version}: ${parts.join('; ') || '(no fields)'}`;
}

function latestVersion(db, projectId, inventoryRecordId) {
  const row = db.prepare(
    'SELECT MAX(version) AS v FROM inventory_insights WHERE project_id=? AND inventory_record_id=?',
  ).get(projectId, inventoryRecordId);
  return Number.isSafeInteger(row?.v) ? row.v : 0;
}

function savedRow(row, { idempotentReplay = false } = {}) {
  return {
    proposalId: row.proposal_id,
    projectId: row.project_id,
    inventoryRecordId: row.inventory_record_id,
    fields: JSON.parse(row.fields_json),
    baseVersion: row.base_version,
    version: row.version,
    operationId: row.operation_id,
    committedAt: row.committed_at,
    committed: true,
    persistence: 'inventory.sqlite3',
    ...(idempotentReplay ? { idempotentReplay: true } : {}),
  };
}

/** Durable propose/commit ledger backed by inventory.sqlite3 (or :memory: for isolated unit tests). */
export class InsightLedger {
  constructor({ now = () => Date.now(), ttlMs = INSIGHT_PROPOSAL_TTL_MS, db, dbPath } = {}) {
    this.now = now;
    this.ttlMs = ttlMs;
    this._ownsDb = false;
    if (db) {
      this.db = db;
      ensureInsightSchema(this.db);
    } else if (dbPath) {
      this.db = openInsightInventoryDb(dbPath);
      this._ownsDb = true;
    } else {
      this.db = new DatabaseSync(':memory:');
      this.db.exec('PRAGMA foreign_keys=ON;');
      ensureInsightSchema(this.db);
      this._ownsDb = true;
    }
  }

  close() {
    if (this._ownsDb && this.db) {
      this.db.close();
      this.db = null;
    }
  }

  propose({ projectId, inventoryRecordId, fields, baseVersion, operationId }) {
    if (!Number.isSafeInteger(projectId) || projectId < 1) throw hostError('host_project_invalid');
    if (typeof inventoryRecordId !== 'string' || !inventoryRecordId || inventoryRecordId.length > 128) {
      throw hostError('host_insight_target_invalid');
    }
    if (typeof operationId !== 'string' || !operationId || operationId.length > 128) {
      throw hostError('host_operation_invalid');
    }
    if (baseVersion !== undefined && !(Number.isSafeInteger(baseVersion) && baseVersion >= 0)) {
      throw hostError('host_insight_base_invalid');
    }
    const patch = normalizeFields(fields);
    const proposalId = randomUUID();
    const createdAt = this.now();
    const expiresAt = createdAt + this.ttlMs;
    const resolvedBase = baseVersion ?? latestVersion(this.db, projectId, inventoryRecordId);
    this.db.prepare(`INSERT INTO insight_proposals(
      proposal_id, project_id, inventory_record_id, fields_json, base_version, operation_id, created_at_ms, expires_at_ms, status
    ) VALUES(?,?,?,?,?,?,?,?, 'proposed')`).run(
      proposalId, projectId, inventoryRecordId, JSON.stringify(patch), resolvedBase, operationId, createdAt, expiresAt,
    );
    const proposal = {
      proposalId,
      projectId,
      inventoryRecordId,
      fields: patch,
      baseVersion: resolvedBase,
      operationId,
      expiresAt: new Date(expiresAt).toISOString(),
      committed: false,
      note: 'Proposal only; call insight_commit with userConfirmed after UI confirmation.',
    };
    proposal.diffSummary = formatInsightDiffSummary(proposal);
    return proposal;
  }

  commit({ projectId, proposalId, operationId, userConfirmed, baseVersion } = {}) {
    if (userConfirmed !== true) throw insightError(FAIL.confirm_required, 403);
    if (!Number.isSafeInteger(projectId) || projectId < 1) throw hostError('host_project_invalid');
    if (typeof proposalId !== 'string' || !proposalId) throw insightError(FAIL.mismatch);
    if (typeof operationId !== 'string' || !operationId) throw insightError(FAIL.mismatch);

    const prior = this.db.prepare(
      'SELECT * FROM inventory_insights WHERE project_id=? AND operation_id=?',
    ).get(projectId, operationId);
    if (prior) {
      if (prior.proposal_id !== proposalId) throw insightError(FAIL.mismatch, 409);
      return savedRow(prior, { idempotentReplay: true });
    }

    const proposal = this.db.prepare('SELECT * FROM insight_proposals WHERE proposal_id=?').get(proposalId);
    if (!proposal || proposal.project_id !== projectId) throw insightError(FAIL.mismatch, 404);
    if (this.now() > proposal.expires_at_ms || proposal.status === 'expired') {
      this.db.prepare("UPDATE insight_proposals SET status='expired' WHERE proposal_id=?").run(proposalId);
      throw insightError(FAIL.proposal_expired, 409);
    }
    if (proposal.status === 'committed') {
      const existing = this.db.prepare('SELECT * FROM inventory_insights WHERE proposal_id=?').get(proposalId);
      if (existing) return savedRow(existing, { idempotentReplay: true });
    }
    const expectedBase = baseVersion !== undefined ? baseVersion : proposal.base_version;
    if (expectedBase !== proposal.base_version) throw insightError(FAIL.base_stale, 409);
    const current = latestVersion(this.db, projectId, proposal.inventory_record_id);
    if (current > proposal.base_version) throw insightError(FAIL.base_stale, 409);

    const version = proposal.base_version + 1;
    const committedAt = new Date(this.now()).toISOString();
    this.db.exec('BEGIN IMMEDIATE');
    try {
      this.db.prepare(`INSERT INTO inventory_insights(
        project_id, inventory_record_id, proposal_id, operation_id, fields_json, base_version, version, committed_at
      ) VALUES(?,?,?,?,?,?,?,?)`).run(
        projectId, proposal.inventory_record_id, proposalId, operationId,
        proposal.fields_json, proposal.base_version, version, committedAt,
      );
      this.db.prepare("UPDATE insight_proposals SET status='committed' WHERE proposal_id=?").run(proposalId);
      this.db.exec('COMMIT');
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
    return {
      proposalId,
      projectId,
      inventoryRecordId: proposal.inventory_record_id,
      fields: JSON.parse(proposal.fields_json),
      baseVersion: proposal.base_version,
      version,
      operationId,
      committedAt,
      committed: true,
      persistence: 'inventory.sqlite3',
      note: 'Committed to inventory_insights in inventory.sqlite3.',
    };
  }
}

export const insightFailCodes = FAIL;

/** Resolve inventory.sqlite3 path for the assets app data directory. */
export function assetsInventoryDbPath(appsRootOrDataDir, app = null) {
  if (app?.data_path && typeof app.data_path === 'string') {
    return join(app.data_path, INSIGHT_INVENTORY_FILENAME);
  }
  if (app?.id && appsRootOrDataDir) {
    return join(appsRootOrDataDir, 'data', app.id, INSIGHT_INVENTORY_FILENAME);
  }
  return null;
}
