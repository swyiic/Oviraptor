// Project-scoped insight propose/commit (Wave3 Asset teammate contract).
// Persists proposals + commits into inventory.sqlite3 (insight_proposals / inventory_insights).
import { randomUUID } from 'node:crypto';

export const INSIGHT_FIELD_WHITELIST = Object.freeze(['tags', 'note', 'labels', 'classification']);
export const INSIGHT_PROPOSAL_TTL_MS = 15 * 60 * 1000;
export const INSIGHT_FAIL = Object.freeze({
  proposal_expired: 'proposal_expired',
  mismatch: 'mismatch',
  base_stale: 'base_stale',
  field_denied: 'field_denied',
  confirm_required: 'confirm_required',
});

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

function fail(code) {
  return Object.assign(new Error(code), { code, insightCode: code });
}

export function normalizeInsightFields(fields) {
  if (!fields || typeof fields !== 'object' || Array.isArray(fields)) throw fail(INSIGHT_FAIL.field_denied);
  const clean = {};
  for (const [key, value] of Object.entries(fields)) {
    if (!INSIGHT_FIELD_WHITELIST.includes(key)) throw fail(INSIGHT_FAIL.field_denied);
    if (value === undefined) continue;
    if (key === 'tags' || key === 'labels') {
      if (!Array.isArray(value) || value.length > 32 || value.some(item => typeof item !== 'string' || item.length > 64)) {
        throw fail(INSIGHT_FAIL.field_denied);
      }
      clean[key] = value.map(item => item.trim()).filter(Boolean);
    } else if (typeof value !== 'string' || value.length > 2048) {
      throw fail(INSIGHT_FAIL.field_denied);
    } else {
      clean[key] = value;
    }
  }
  if (Object.keys(clean).length === 0) throw fail(INSIGHT_FAIL.field_denied);
  return clean;
}

function latestVersion(db, projectId, inventoryRecordId) {
  const row = db.prepare(
    'SELECT MAX(version) AS v FROM inventory_insights WHERE project_id=? AND inventory_record_id=?',
  ).get(projectId, inventoryRecordId);
  return Number.isSafeInteger(row?.v) ? row.v : 0;
}

export function proposeInsight(db, {
  projectId, inventoryRecordId, fields, baseVersion, operationId, now = Date.now(), ttlMs = INSIGHT_PROPOSAL_TTL_MS,
}) {
  if (!Number.isSafeInteger(projectId) || projectId < 1) throw Error('app_context_invalid');
  if (typeof inventoryRecordId !== 'string' || !inventoryRecordId || inventoryRecordId.length > 128) {
    throw Error('asset_insight_target_invalid');
  }
  if (typeof operationId !== 'string' || !operationId || operationId.length > 128) {
    throw Error('app_arguments_invalid');
  }
  if (baseVersion !== undefined && !(Number.isSafeInteger(baseVersion) && baseVersion >= 0)) {
    throw Error('asset_insight_base_invalid');
  }
  ensureInsightSchema(db);
  const patch = normalizeInsightFields(fields);
  const proposalId = randomUUID();
  const createdAt = now;
  const expiresAt = createdAt + ttlMs;
  const resolvedBase = baseVersion ?? latestVersion(db, projectId, inventoryRecordId);
  db.prepare(`INSERT INTO insight_proposals(
    proposal_id, project_id, inventory_record_id, fields_json, base_version, operation_id, created_at_ms, expires_at_ms, status
  ) VALUES(?,?,?,?,?,?,?,?, 'proposed')`).run(
    proposalId, projectId, inventoryRecordId, JSON.stringify(patch), resolvedBase, operationId, createdAt, expiresAt,
  );
  return {
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
}

export function commitInsight(db, {
  projectId, proposalId, operationId, userConfirmed, baseVersion, now = Date.now(), manageTransaction = true,
}) {
  if (userConfirmed !== true) throw fail(INSIGHT_FAIL.confirm_required);
  if (!Number.isSafeInteger(projectId) || projectId < 1) throw Error('app_context_invalid');
  if (typeof proposalId !== 'string' || !proposalId) throw fail(INSIGHT_FAIL.mismatch);
  if (typeof operationId !== 'string' || !operationId) throw fail(INSIGHT_FAIL.mismatch);
  ensureInsightSchema(db);

  const prior = db.prepare(
    'SELECT * FROM inventory_insights WHERE project_id=? AND operation_id=?',
  ).get(projectId, operationId);
  if (prior) {
    if (prior.proposal_id !== proposalId) throw fail(INSIGHT_FAIL.mismatch);
    return {
      proposalId: prior.proposal_id,
      projectId: prior.project_id,
      inventoryRecordId: prior.inventory_record_id,
      fields: JSON.parse(prior.fields_json),
      baseVersion: prior.base_version,
      version: prior.version,
      operationId: prior.operation_id,
      committedAt: prior.committed_at,
      committed: true,
      persistence: 'inventory.sqlite3',
      idempotentReplay: true,
    };
  }

  const proposal = db.prepare('SELECT * FROM insight_proposals WHERE proposal_id=?').get(proposalId);
  if (!proposal || proposal.project_id !== projectId) throw fail(INSIGHT_FAIL.mismatch);
  if (now > proposal.expires_at_ms || proposal.status === 'expired') {
    db.prepare("UPDATE insight_proposals SET status='expired' WHERE proposal_id=?").run(proposalId);
    throw fail(INSIGHT_FAIL.proposal_expired);
  }
  if (proposal.status === 'committed') {
    const existing = db.prepare('SELECT * FROM inventory_insights WHERE proposal_id=?').get(proposalId);
    if (existing) {
      return {
        proposalId: existing.proposal_id,
        projectId: existing.project_id,
        inventoryRecordId: existing.inventory_record_id,
        fields: JSON.parse(existing.fields_json),
        baseVersion: existing.base_version,
        version: existing.version,
        operationId: existing.operation_id,
        committedAt: existing.committed_at,
        committed: true,
        persistence: 'inventory.sqlite3',
        idempotentReplay: true,
      };
    }
  }
  const expectedBase = baseVersion !== undefined ? baseVersion : proposal.base_version;
  if (expectedBase !== proposal.base_version) throw fail(INSIGHT_FAIL.base_stale);
  const current = latestVersion(db, projectId, proposal.inventory_record_id);
  if (current > proposal.base_version) throw fail(INSIGHT_FAIL.base_stale);

  const fields = JSON.parse(proposal.fields_json);
  const version = proposal.base_version + 1;
  const committedAt = new Date(now).toISOString();
  const write = () => {
    db.prepare(`INSERT INTO inventory_insights(
      project_id, inventory_record_id, proposal_id, operation_id, fields_json, base_version, version, committed_at
    ) VALUES(?,?,?,?,?,?,?,?)`).run(
      projectId, proposal.inventory_record_id, proposalId, operationId,
      proposal.fields_json, proposal.base_version, version, committedAt,
    );
    db.prepare("UPDATE insight_proposals SET status='committed' WHERE proposal_id=?").run(proposalId);
  };
  if (manageTransaction) {
    db.exec('BEGIN IMMEDIATE');
    try { write(); db.exec('COMMIT'); }
    catch (error) { db.exec('ROLLBACK'); throw error; }
  } else {
    write();
  }
  return {
    proposalId,
    projectId,
    inventoryRecordId: proposal.inventory_record_id,
    fields,
    baseVersion: proposal.base_version,
    version,
    operationId,
    committedAt,
    committed: true,
    persistence: 'inventory.sqlite3',
    note: 'Committed to inventory_insights in inventory.sqlite3.',
  };
}
