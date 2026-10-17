// Project-scoped insight propose/commit (Wave3 Asset teammate contract).
// Host API stub: propose is ready to become a model tool; commit requires userConfirmed.
// Does not write through inventory_import free-text. Ownership/auth stay off the tool surface.
import { randomUUID } from 'node:crypto';
import { hostError } from './store.mjs';

export const INSIGHT_FIELD_WHITELIST = Object.freeze(['tags', 'note', 'labels', 'classification']);
export const INSIGHT_PROPOSAL_TTL_MS = 15 * 60 * 1000;

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

/** In-memory proposal ledger keyed by proposalId. Host process lifetime only. */
export class InsightLedger {
  constructor({ now = () => Date.now(), ttlMs = INSIGHT_PROPOSAL_TTL_MS } = {}) {
    this.proposals = new Map();
    this.commits = new Map(); // operationId -> result (idempotent)
    this.now = now;
    this.ttlMs = ttlMs;
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
    const record = {
      proposalId,
      projectId,
      inventoryRecordId,
      fields: patch,
      baseVersion: baseVersion ?? 0,
      operationId,
      createdAt,
      expiresAt: createdAt + this.ttlMs,
      status: 'proposed',
    };
    this.proposals.set(proposalId, record);
    return {
      proposalId,
      projectId,
      inventoryRecordId,
      fields: patch,
      baseVersion: record.baseVersion,
      operationId,
      expiresAt: new Date(record.expiresAt).toISOString(),
      committed: false,
      note: 'Proposal only; call insight_commit with userConfirmed after UI confirmation.',
    };
  }

  commit({ projectId, proposalId, operationId, userConfirmed, baseVersion } = {}) {
    if (userConfirmed !== true) throw insightError(FAIL.confirm_required, 403);
    if (!Number.isSafeInteger(projectId) || projectId < 1) throw hostError('host_project_invalid');
    if (typeof proposalId !== 'string' || !proposalId) throw insightError(FAIL.mismatch);
    if (typeof operationId !== 'string' || !operationId) throw insightError(FAIL.mismatch);

    const prior = this.commits.get(operationId);
    if (prior) {
      if (prior.proposalId !== proposalId || prior.projectId !== projectId) throw insightError(FAIL.mismatch, 409);
      return { ...prior, idempotentReplay: true };
    }

    const proposal = this.proposals.get(proposalId);
    if (!proposal || proposal.projectId !== projectId) throw insightError(FAIL.mismatch, 404);
    if (proposal.operationId !== operationId && proposal.status === 'proposed') {
      // Commit may use a fresh operationId; bind to proposalId. Mismatch only if proposal was for another op already committed under different id — keep simple: allow new operationId.
    }
    if (this.now() > proposal.expiresAt || proposal.status === 'expired') {
      proposal.status = 'expired';
      throw insightError(FAIL.proposal_expired, 409);
    }
    if (baseVersion !== undefined && baseVersion !== proposal.baseVersion) {
      throw insightError(FAIL.base_stale, 409);
    }
    if (proposal.status === 'committed') {
      const existing = [...this.commits.values()].find(item => item.proposalId === proposalId);
      if (existing) return { ...existing, idempotentReplay: true };
    }

    proposal.status = 'committed';
    const committedAt = this.now();
    const saved = {
      proposalId,
      projectId,
      inventoryRecordId: proposal.inventoryRecordId,
      fields: proposal.fields,
      baseVersion: proposal.baseVersion,
      operationId,
      committedAt: new Date(committedAt).toISOString(),
      committed: true,
      // Stub: durable inventory write lands when UI confirm + assets table are wired.
      persistence: 'host_ledger_stub',
      note: 'Committed in host insight ledger stub; inventory row write follows when assets insight table ships.',
    };
    this.commits.set(operationId, saved);
    return saved;
  }
}

export const insightFailCodes = FAIL;
