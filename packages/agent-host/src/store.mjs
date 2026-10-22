import { DatabaseSync } from 'node:sqlite';
import { randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { initializeNativeOwners } from './native-owners.mjs';
import { createTask, interruptTask } from './tasks.mjs';

export const RETRYABLE_HOST_CODES = Object.freeze(new Set(['host_grant_revision_conflict', 'host_app_timeout', 'timeout']));
export const hostError = (code, status = 400, details = {}) => {
  const retryable = details.retryable === true || RETRYABLE_HOST_CODES.has(code);
  return Object.assign(new Error(code), { code, status, details, retryable });
};
export const mcpErrorPayload = error => {
  const code = typeof error?.code === 'string' ? error.code.split(':')[0] : 'host_request_failed';
  const known = /^(host_|skill_package_|app_)[a-z0-9_]+$/.test(code) || code === 'timeout';
  const payload = { error: known ? code : 'host_request_failed' };
  const retryable = error?.retryable === true || (known && RETRYABLE_HOST_CODES.has(payload.error) && error?.retryable !== false);
  // Only annotate retryable:true so existing error-body contracts stay stable; absence means not retryable.
  if (retryable) payload.retryable = true;
  return payload;
};
export function positiveId(value) {
  const id = Number(value);
  if (!Number.isSafeInteger(id) || id < 1) throw hostError('host_project_invalid');
  return id;
}
export function boundedText(value, max = 65536) {
  if (typeof value !== 'string' || !value.trim() || Buffer.byteLength(value) > max || value.includes('\0')) throw hostError('host_text_invalid');
  return value.trim();
}
export const DEFAULT_CONVERSATION_TITLE = 'New conversation';
const PLACEHOLDER_CONVERSATION_TITLES = new Set([DEFAULT_CONVERSATION_TITLE, '新对话', '新對話']);
export function isPlaceholderConversationTitle(title) {
  const value = String(title ?? '').trim();
  return !value || PLACEHOLDER_CONVERSATION_TITLES.has(value);
}
/** Light one-liner from the first user message; no model call. */
export function deriveConversationTitle(text, maxChars = 48) {
  const cleaned = String(text ?? '').replace(/\s+/g, ' ').trim();
  if (!cleaned) return DEFAULT_CONVERSATION_TITLE;
  const chars = [...cleaned];
  if (chars.length <= maxChars) return cleaned;
  const joined = chars.slice(0, maxChars).join('');
  const space = joined.lastIndexOf(' ');
  if (space > Math.floor(maxChars / 3)) return `${joined.slice(0, space)}…`;
  return `${joined}…`;
}
const stamp = () => new Date().toISOString();

/** Core storage contains no application business tables or implicit legacy DB. */
export class HostStore {
  constructor(file) {
    if (file !== ':memory:') mkdirSync(dirname(file), { recursive: true, mode: 0o700 });
    this.db = new DatabaseSync(file);
    this.db.exec(`PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
      CREATE TABLE IF NOT EXISTS projects(id INTEGER PRIMARY KEY,name TEXT NOT NULL,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),title TEXT NOT NULL,session_id TEXT,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),role TEXT NOT NULL CHECK(role IN ('user','assistant')),content TEXT NOT NULL,status TEXT NOT NULL,run_id TEXT,created_at TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS messages_by_conversation ON messages(conversation_id);
      CREATE TABLE IF NOT EXISTS applications(id TEXT PRIMARY KEY,version TEXT NOT NULL,digest TEXT NOT NULL,path TEXT NOT NULL,manifest TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 0,revision INTEGER NOT NULL DEFAULT 1,status TEXT NOT NULL DEFAULT 'installed');
      CREATE TABLE IF NOT EXISTS application_grants(app_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,project_id INTEGER NOT NULL REFERENCES projects(id),permissions TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(app_id,project_id));
      CREATE TABLE IF NOT EXISTS host_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS runtime_metrics(run_id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL,metrics TEXT NOT NULL,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS provider_usage(request_id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,run_id TEXT NOT NULL,api TEXT NOT NULL,model TEXT NOT NULL,usage_json TEXT,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS conversation_memory(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,payload TEXT NOT NULL,updated_at TEXT NOT NULL);`);
    this.db.exec(`CREATE TABLE IF NOT EXISTS conversation_workspace_grants(conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,runtime_identity TEXT NOT NULL,permission TEXT NOT NULL CHECK(permission IN ('workspace.read','workspace.write')),PRIMARY KEY(conversation_id,runtime_identity,permission))`);
    this.db.exec(`CREATE TABLE IF NOT EXISTS conversation_tasks(id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,payload TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS conversation_tasks_conversation ON conversation_tasks(conversation_id);`);
    for (const row of this.db.prepare('SELECT id,payload FROM conversation_tasks').all()) {
      const task = JSON.parse(row.payload);
      if (task.status === 'running') this.saveTask(interruptTask(task));
    }
    // Reconcile the parent's durable child index after interrupted children have
    // been classified. This does not start runtimes or replay any request.
    for (const row of this.db.prepare('SELECT payload FROM conversation_tasks').all()) {
      const child = JSON.parse(row.payload);
      if (!child.parentTaskId) continue;
      const parent = this.task(child.conversationId, child.parentTaskId);
      const status = child.status;
      if (parent.children?.some(item => item.id === child.id && item.status !== status)) this.saveTask({ ...parent, acceptance: null, children: parent.children.map(item => item.id === child.id ? { ...item, status, ...(child.report ? { report: child.report } : {}) } : item) });
    }
    const columns = table => new Set(this.db.prepare(`PRAGMA table_info(${table})`).all().map(row => row.name));
    const messageColumns = columns('messages');
    for (const [name, declaration] of [['outcome_unknown', 'INTEGER NOT NULL DEFAULT 0'], ['failure_code', "TEXT NOT NULL DEFAULT ''"], ['stop_reason', "TEXT NOT NULL DEFAULT ''"], ['cancel_requested', 'INTEGER NOT NULL DEFAULT 0']]) {
      if (!messageColumns.has(name)) this.db.exec(`ALTER TABLE messages ADD COLUMN ${name} ${declaration}`);
    }
    this.db.exec("UPDATE messages SET status='interrupted',outcome_unknown=1,failure_code='host_interrupted_restart',stop_reason='' WHERE status='running' OR (status='interrupted' AND failure_code='')");
    this.db.exec("UPDATE messages SET stop_reason='steer_interrupted' WHERE role='user' AND stop_reason='steer_queued'");
    const appColumns = columns('applications');
    if (!appColumns.has('data_path')) this.db.exec('ALTER TABLE applications ADD COLUMN data_path TEXT');
    if (!appColumns.has('data_version')) this.db.exec("ALTER TABLE applications ADD COLUMN data_version TEXT NOT NULL DEFAULT 'legacy'");
    if (!appColumns.has('builtin')) this.db.exec('ALTER TABLE applications ADD COLUMN builtin INTEGER NOT NULL DEFAULT 0');
    if (!appColumns.has('uninstallable')) this.db.exec('ALTER TABLE applications ADD COLUMN uninstallable INTEGER NOT NULL DEFAULT 1');
    const grantColumns = columns('application_grants');
    if (!grantColumns.has('disclosure_runtime')) this.db.exec("ALTER TABLE application_grants ADD COLUMN disclosure_runtime TEXT NOT NULL DEFAULT ''");
    const projectColumns = columns('projects');
    if (!projectColumns.has('external_id')) this.db.exec('ALTER TABLE projects ADD COLUMN external_id TEXT');
    if (!projectColumns.has('kind')) this.db.exec("ALTER TABLE projects ADD COLUMN kind TEXT NOT NULL DEFAULT 'project'");
    this.db.exec('CREATE UNIQUE INDEX IF NOT EXISTS projects_external_id_unique ON projects(external_id) WHERE external_id IS NOT NULL');
    this.db.exec(`CREATE TABLE IF NOT EXISTS application_versions(sequence INTEGER PRIMARY KEY,app_id TEXT NOT NULL,version TEXT NOT NULL,digest TEXT NOT NULL,path TEXT NOT NULL,manifest TEXT NOT NULL,data_path TEXT,data_version TEXT NOT NULL,replaced_at TEXT NOT NULL,replaced_by_version TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS host_budget_limits(project_id INTEGER PRIMARY KEY REFERENCES projects(id), tool_calls INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS host_budget_entries(operation_id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, state TEXT NOT NULL, effect_class TEXT NOT NULL, updated_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS tool_records(id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, conversation_id TEXT NOT NULL, operation_id TEXT NOT NULL, sha256 TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL, visibility TEXT NOT NULL DEFAULT 'available');
      CREATE TABLE IF NOT EXISTS project_allocations(project_id INTEGER PRIMARY KEY, total INTEGER NOT NULL, host_cap INTEGER NOT NULL, native_cap INTEGER NOT NULL, host_spent INTEGER NOT NULL DEFAULT 0, native_spent INTEGER NOT NULL DEFAULT 0, soft_pause INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS task_approvals(id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, conversation_id TEXT NOT NULL, scope_hash TEXT NOT NULL, choice TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS summary_versions(id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, conversation_id TEXT NOT NULL, source_ids TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS human_reviews(id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, task_id TEXT NOT NULL, report_version INTEGER NOT NULL, note TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS report_versions(id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, task_id TEXT NOT NULL, version INTEGER NOT NULL, format TEXT NOT NULL, snapshot_hash TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL, UNIQUE(project_id, task_id, snapshot_hash, format));`);
    this.db.exec(`CREATE TABLE IF NOT EXISTS application_retained(app_id TEXT PRIMARY KEY,version TEXT NOT NULL,digest TEXT NOT NULL,data_path TEXT,data_version TEXT NOT NULL,data_schema_version INTEGER NOT NULL,retained_at TEXT NOT NULL);`);
    initializeNativeOwners(this.db);
    const recordColumns = columns('tool_records');
    if (recordColumns.size && !recordColumns.has('visibility')) this.db.exec("ALTER TABLE tool_records ADD COLUMN visibility TEXT NOT NULL DEFAULT 'available'");
    const allocationColumns = columns('project_allocations');
    if (allocationColumns.size && !allocationColumns.has('revision')) this.db.exec('ALTER TABLE project_allocations ADD COLUMN revision INTEGER NOT NULL DEFAULT 1');
    if (allocationColumns.size && !allocationColumns.has('model_cap')) this.db.exec('ALTER TABLE project_allocations ADD COLUMN model_cap INTEGER');
    if (allocationColumns.size && !allocationColumns.has('model_spent')) this.db.exec('ALTER TABLE project_allocations ADD COLUMN model_spent INTEGER NOT NULL DEFAULT 0');
    const approvalColumns = columns('task_approvals');
    if (approvalColumns.size && !approvalColumns.has('budget_revision')) this.db.exec('ALTER TABLE task_approvals ADD COLUMN budget_revision INTEGER NOT NULL DEFAULT 0');
  }
  close() { this.db.close(); }
  setToolCallBudget(projectId, toolCalls) {
    const id = positiveId(projectId);
    this.project(id);
    if (toolCalls == null) { this.db.prepare('DELETE FROM host_budget_limits WHERE project_id=?').run(id); return null; }
    if (!Number.isSafeInteger(toolCalls) || toolCalls < 0) throw hostError('host_budget_invalid', 400);
    this.db.prepare('INSERT INTO host_budget_limits(project_id,tool_calls) VALUES(?,?) ON CONFLICT(project_id) DO UPDATE SET tool_calls=excluded.tool_calls').run(id, toolCalls);
    return toolCalls;
  }
  admitToolCall(projectId, operationId, effectClass = 'read') {
    const id = positiveId(projectId);
    const limit = this.db.prepare('SELECT tool_calls FROM host_budget_limits WHERE project_id=?').get(id);
    const allocationPreview = this.db.prepare('SELECT 1 AS ok FROM project_allocations WHERE project_id=?').get(id);
    if (!limit && !allocationPreview) return { enforced: false };
    const op = String(operationId || '');
    if (!op) throw hostError('host_operation_invalid', 400);
    this.db.exec('BEGIN IMMEDIATE');
    let open = true;
    const rollback = () => { if (!open) return; open = false; this.db.exec('ROLLBACK'); };
    try {
      const existing = this.db.prepare('SELECT state FROM host_budget_entries WHERE operation_id=?').get(op);
      if (existing) { open = false; this.db.exec('COMMIT'); return { enforced: true, state: existing.state, replay: true }; }
      const used = this.db.prepare("SELECT COUNT(*) AS n FROM host_budget_entries WHERE project_id=? AND state IN ('reserved','consumed','indeterminate')").get(id).n;
      const allocation = this.db.prepare('SELECT host_cap, host_spent, soft_pause FROM project_allocations WHERE project_id=?').get(id);
      if (allocation && allocation.host_spent >= allocation.host_cap) { rollback(); throw hostError('host_budget_hard_stop', 409, { retryable: false }); }
      if (allocation && allocation.soft_pause > 0 && allocation.host_cap - allocation.host_spent <= allocation.soft_pause) { rollback(); throw hostError('host_budget_paused', 409, { retryable: false }); }
      if (limit && used >= limit.tool_calls) { rollback(); throw hostError('host_budget_hard_stop', 409, { retryable: false }); }
      this.db.prepare('INSERT INTO host_budget_entries(operation_id,project_id,state,effect_class,updated_at) VALUES(?,?,?,?,?)').run(op, id, 'reserved', effectClass === 'write' ? 'write' : 'read', stamp());
      if (allocation) this.db.prepare('UPDATE project_allocations SET host_spent=host_spent+1 WHERE project_id=?').run(id);
      open = false; this.db.exec('COMMIT');
      return { enforced: true, state: 'reserved' };
    } catch (error) {
      rollback();
      throw error;
    }
  }
  settleToolCall(operationId, state) {
    const op = String(operationId || '');
    const next = ['consumed', 'indeterminate', 'released'].includes(state) ? state : '';
    if (!op || !next) return null;
    const row = this.db.prepare('SELECT state FROM host_budget_entries WHERE operation_id=?').get(op);
    if (!row || row.state === 'consumed' || row.state === 'indeterminate') return row?.state ?? null;
    this.db.prepare('UPDATE host_budget_entries SET state=?, updated_at=? WHERE operation_id=?').run(next, stamp(), op);
    return next;
  }
  saveToolRecord({ id, projectId, conversationId, operationId, sha256, body }) {
    this.project(projectId);
    this.db.prepare('INSERT INTO tool_records(id,project_id,conversation_id,operation_id,sha256,body,created_at) VALUES(?,?,?,?,?,?,?)').run(id, positiveId(projectId), String(conversationId), String(operationId), sha256, body, stamp());
    return id;
  }
  saveReportVersion({ id, projectId, taskId, format, snapshotHash, body }) {
    this.project(projectId);
    const existing = this.db.prepare('SELECT id, version, body FROM report_versions WHERE project_id=? AND task_id=? AND snapshot_hash=? AND format=?').get(positiveId(projectId), String(taskId), snapshotHash, format);
    if (existing) return { id: existing.id, version: existing.version, body: existing.body, reused: true };
    const version = (this.db.prepare('SELECT COALESCE(MAX(version),0) AS version FROM report_versions WHERE project_id=? AND task_id=? AND format=?').get(positiveId(projectId), String(taskId), format)?.version ?? 0) + 1;
    this.db.prepare('INSERT INTO report_versions(id,project_id,task_id,version,format,snapshot_hash,body,created_at) VALUES(?,?,?,?,?,?,?,?)').run(id, positiveId(projectId), String(taskId), version, format, snapshotHash, body, stamp());
    return { id, version, body, reused: false };
  }
  saveAllocation(projectId, allocation) {
    const id = positiveId(projectId);
    this.project(id);
    // Omission preserves an existing model cap; explicit null removes it. Spent
    // attempts remain cumulative when an operator changes any allocation.
    const modelCap = allocation.modelCap === undefined ? this.allocation(id)?.modelCap ?? null : allocation.modelCap;
    this.db.prepare(`INSERT INTO project_allocations(project_id,total,host_cap,native_cap,host_spent,native_spent,soft_pause,revision,model_cap,model_spent) VALUES(?,?,?,?,0,0,?,1,?,0)
      ON CONFLICT(project_id) DO UPDATE SET total=excluded.total, host_cap=excluded.host_cap, native_cap=excluded.native_cap, soft_pause=excluded.soft_pause, model_cap=excluded.model_cap, revision=project_allocations.revision+1`).run(id, allocation.total, allocation.host, allocation.native, allocation.softPause ?? 0, modelCap);
    return this.allocation(id);
  }
  allocation(projectId) {
    const row = this.db.prepare('SELECT total, host_cap AS host, native_cap AS native, host_spent AS hostSpent, native_spent AS nativeSpent, soft_pause AS softPause, revision, model_cap AS modelCap, model_spent AS modelSpent FROM project_allocations WHERE project_id=?').get(positiveId(projectId));
    if (!row) return null;
    return { ...row, pending: row.total - row.host - row.native };
  }
  reserveNative(projectId, amount) {
    const id = positiveId(projectId);
    const row = this.allocation(id);
    if (!row) throw hostError('host_budget_invalid', 400);
    if (!Number.isSafeInteger(amount) || amount < 1 || row.nativeSpent + amount > row.native) throw hostError('host_budget_hard_stop', 409, { retryable: false });
    this.db.prepare('UPDATE project_allocations SET native_spent=native_spent+? WHERE project_id=?').run(amount, id);
    return this.allocation(id);
  }
  prepareApproval({ id, projectId, conversationId, scopeHash, choice, budgetRevision = 0 }) {
    this.project(projectId);
    this.db.prepare('INSERT INTO task_approvals(id,project_id,conversation_id,scope_hash,choice,status,created_at,budget_revision) VALUES(?,?,?,?,?,?,?,?)').run(id, positiveId(projectId), String(conversationId), scopeHash, choice, 'prepared', stamp(), budgetRevision);
    return { id, status: 'prepared', scopeHash, budgetRevision, startsScan: false };
  }
  confirmApproval({ projectId, approvalId, scopeHash }) {
    const row = this.db.prepare('SELECT id, project_id AS projectId, scope_hash AS scopeHash, status, budget_revision AS budgetRevision FROM task_approvals WHERE id=?').get(String(approvalId));
    const currentRevision = this.allocation(projectId)?.revision ?? 0;
    if (!row || row.projectId !== positiveId(projectId) || row.scopeHash !== scopeHash || row.status !== 'prepared') throw hostError('host_approval_invalid', 409);
    if (row.budgetRevision !== currentRevision) throw hostError('host_approval_stale', 409);
    this.db.prepare("UPDATE task_approvals SET status='confirmed' WHERE id=?").run(row.id);
    return { id: row.id, status: 'confirmed', scopeHash, startsScan: false };
  }
  admitModelRequest(projectId) {
    const id = positiveId(projectId);
    // Conditional increment is the admission gate, including across separate
    // database connections. Count attempted HTTP requests conservatively; an
    // interrupted response cannot prove that the provider did not bill it.
    const admitted = this.db.prepare('UPDATE project_allocations SET model_spent=model_spent+1 WHERE project_id=? AND model_cap IS NOT NULL AND model_spent<model_cap RETURNING model_spent').get(id);
    if (admitted) return { enforced: true };
    const row = this.allocation(id);
    if (!row || row.modelCap == null) return { enforced: false };
    throw hostError('host_budget_hard_stop', 409, { retryable: false });
  }
  querySummaries(projectId) {
    return this.db.prepare('SELECT id, conversation_id AS conversationId, source_ids AS sourceIds, body, created_at AS createdAt FROM summary_versions WHERE project_id=? ORDER BY created_at').all(positiveId(projectId)).map(row => ({ ...row, sourceIds: JSON.parse(row.sourceIds) }));
  }
  saveSummaryVersion({ id, projectId, conversationId, sourceIds, body }) {
    this.project(projectId);
    this.db.prepare('INSERT INTO summary_versions(id,project_id,conversation_id,source_ids,body,created_at) VALUES(?,?,?,?,?,?)').run(id, positiveId(projectId), String(conversationId), JSON.stringify(sourceIds), body, stamp());
    return id;
  }
  addHumanReview({ id, projectId, taskId, reportVersion, note }) {
    this.project(projectId);
    this.db.prepare('INSERT INTO human_reviews(id,project_id,task_id,report_version,note,created_at) VALUES(?,?,?,?,?,?)').run(id, positiveId(projectId), String(taskId), reportVersion, String(note).slice(0, 500), stamp());
    return { id, reportVersion, note };
  }
  purgeProjectRecords(projectId) {
    const changed = this.db.prepare("UPDATE tool_records SET visibility='source_purged' WHERE project_id=? AND visibility!='source_purged'").run(positiveId(projectId)).changes;
    return changed;
  }
  queryHistory(projectId, { includePurged = false } = {}) {
    const id = positiveId(projectId);
    const rows = this.db.prepare(`SELECT id, operation_id AS operationId, sha256, visibility, created_at AS createdAt FROM tool_records WHERE project_id=? ${includePurged ? '' : "AND visibility!='source_purged'"} ORDER BY created_at`).all(id);
    return rows;
  }
  toolRecord(id, projectId) {
    return this.db.prepare('SELECT id,project_id AS projectId,conversation_id AS conversationId,operation_id AS operationId,sha256,body,created_at AS createdAt FROM tool_records WHERE id=? AND project_id=?').get(String(id), positiveId(projectId)) ?? null;
  }
  projects() { return this.db.prepare('SELECT id,name,external_id AS externalId,kind,created_at AS createdAt FROM projects ORDER BY id').all(); }
  project(id) {
    const row = this.db.prepare('SELECT id,name,external_id AS externalId,kind FROM projects WHERE id=?').get(positiveId(id));
    if (!row) throw hostError('host_project_unavailable', 404);
    return row;
  }
  normalizeProjectKind(kind) {
    return kind === 'scratch' ? 'scratch' : 'project';
  }
  createProject(name, { externalId, kind } = {}) {
    const value = boundedText(name, 256);
    const external = externalId == null || externalId === '' ? null : String(externalId).slice(0, 128);
    const projectKind = this.normalizeProjectKind(kind);
    const result = this.db.prepare('INSERT INTO projects(name,created_at,external_id,kind) VALUES(?,?,?,?)').run(value, stamp(), external, projectKind);
    return this.project(Number(result.lastInsertRowid));
  }
  /** Ephemeral workspace for chat-first; still project-bound for ACP cwd / grants. */
  createScratchProject(label) {
    const day = new Date().toISOString().slice(0, 10);
    const name = typeof label === 'string' && label.trim() ? boundedText(label, 256) : `Scratch · ${day}`;
    return this.createProject(name, { kind: 'scratch' });
  }
  /** Rename a scratch workspace into a durable project. Non-scratch projects are rejected. */
  promoteProject(id, name) {
    const project = this.project(id);
    if (project.kind !== 'scratch') throw hostError('host_project_not_scratch');
    const value = boundedText(name, 256);
    this.db.prepare("UPDATE projects SET name=?, kind='project' WHERE id=?").run(value, positiveId(id));
    return this.project(id);
  }
  /** Map a desktop workspace into the host projects table by stable externalId. */
  ensureProjectByExternalId(externalId, name) {
    const external = String(externalId ?? '').slice(0, 128);
    if (!external) throw hostError('host_project_external_id_invalid');
    const existing = this.db.prepare('SELECT id FROM projects WHERE external_id=?').get(external);
    if (existing) {
      const nextName = boundedText(name || 'Workspace', 256);
      this.db.prepare('UPDATE projects SET name=? WHERE id=?').run(nextName, existing.id);
      return this.project(existing.id);
    }
    return this.createProject(name || `Workspace ${external}`, { externalId: external });
  }
  conversations(projectId) {
    this.project(projectId);
    return this.db.prepare('SELECT id,project_id AS projectId,title,created_at AS createdAt FROM conversations WHERE project_id=? ORDER BY created_at,id').all(Number(projectId));
  }
  conversation(id, projectId) {
    const row = this.db.prepare('SELECT id,project_id AS projectId,title,session_id AS sessionId FROM conversations WHERE id=?').get(String(id));
    if (!row || (projectId !== undefined && row.projectId !== positiveId(projectId))) throw hostError('host_conversation_unavailable', 404);
    return row;
  }
  createConversation(projectId, title = DEFAULT_CONVERSATION_TITLE) {
    this.project(projectId);
    const id = randomUUID();
    this.db.prepare('INSERT INTO conversations(id,project_id,title,created_at) VALUES(?,?,?,?)').run(id, Number(projectId), boundedText(title, 256), stamp());
    return this.conversation(id);
  }
  updateConversationTitle(id, title) {
    this.conversation(id);
    this.db.prepare('UPDATE conversations SET title=? WHERE id=?').run(boundedText(title, 256), String(id));
    return this.conversation(id);
  }
  deleteConversation(id, projectId) {
    this.conversation(id, projectId);
    const conversationId = String(id);
    this.db.exec('BEGIN');
    try {
      this.db.prepare('DELETE FROM messages WHERE conversation_id=?').run(conversationId);
      this.db.prepare('DELETE FROM conversation_memory WHERE conversation_id=?').run(conversationId);
      this.db.prepare('DELETE FROM conversations WHERE id=?').run(conversationId);
      this.db.exec('COMMIT');
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
  }
  session(id, sessionId) { this.conversation(id); this.db.prepare('UPDATE conversations SET session_id=? WHERE id=?').run(sessionId, id); }
  messages(id, projectId) {
    this.conversation(id, projectId);
    return this.db.prepare('SELECT m.id,m.role,m.content,m.status,m.run_id AS runId,m.created_at AS createdAt,m.outcome_unknown AS outcomeUnknown,m.failure_code AS failureCode,m.stop_reason AS stopReason,m.cancel_requested AS cancelRequested,rm.metrics AS runtimeMetrics FROM messages m LEFT JOIN runtime_metrics rm ON rm.run_id=m.run_id WHERE m.conversation_id=? ORDER BY m.rowid').all(id).map(({ runtimeMetrics, ...row }) => {
      let usage; try { usage = JSON.parse(runtimeMetrics ?? '{}').usage; } catch { /* older metrics */ }
      return { ...row, outcomeUnknown: Boolean(row.outcomeUnknown), cancelRequested: Boolean(row.cancelRequested), ...(usage?.requests > 0 ? { usage } : {}) };
    });
  }
  turnMessages(id, userMessageId, projectId) {
    this.conversation(id, projectId);
    const columns='id,role,content,status,run_id AS runId,stop_reason AS stopReason,outcome_unknown AS outcomeUnknown';
    const latest=this.db.prepare(`SELECT ${columns} FROM messages WHERE conversation_id=? ORDER BY rowid DESC LIMIT 1`).get(id);
    if(!latest)return [];
    const user=typeof userMessageId==='string'
      ?this.db.prepare(`SELECT ${columns} FROM messages WHERE conversation_id=? AND id=? AND role='user'`).get(id,userMessageId):null;
    return(user&&user.id!==latest.id?[user,latest]:[latest]).map(row=>({...row,outcomeUnknown:Boolean(row.outcomeUnknown)}));
  }
  addMessage(id, role, content, status = 'completed', runId = null) {
    this.conversation(id);
    const message = { id: randomUUID(), role, content, status, runId, createdAt: stamp() };
    this.db.prepare('INSERT INTO messages(id,conversation_id,role,content,status,run_id,created_at) VALUES(?,?,?,?,?,?,?)').run(message.id, id, role, content, status, runId, message.createdAt);
    return message;
  }
  acceptSteer(conversationId, runId, content) {
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const message = this.addMessage(conversationId, 'user', boundedText(content), 'completed', runId);
      this.finishMessage(message.id, message.content, 'completed', { stopReason: 'steer_queued' });
      this.db.exec('COMMIT');
      return { ...message, stopReason: 'steer_queued' };
    } catch (error) { this.db.exec('ROLLBACK'); throw error; }
  }
  deliverSteers(ids) {
    const write = this.db.prepare("UPDATE messages SET stop_reason='steer_delivered' WHERE id=? AND role='user' AND stop_reason IN ('steer_queued','steer_interrupted')");
    for (const id of ids) write.run(id);
  }
  interruptSteers(runId) { this.db.prepare("UPDATE messages SET stop_reason='steer_interrupted' WHERE run_id=? AND role='user' AND stop_reason='steer_queued'").run(runId); }
  pendingSteers(conversationId) {
    return this.db.prepare("SELECT id,content FROM messages WHERE conversation_id=? AND role='user' AND stop_reason='steer_interrupted' ORDER BY rowid").all(String(conversationId));
  }
  finishMessage(id, content, status, details = {}) { this.db.prepare('UPDATE messages SET content=?,status=?,outcome_unknown=?,failure_code=?,stop_reason=?,cancel_requested=? WHERE id=?').run(content, status, Number(Boolean(details.outcomeUnknown)), details.failureCode ?? '', details.stopReason ?? '', Number(Boolean(details.cancelRequested)), id); }
  metrics(runId, conversationId, metrics) { this.db.prepare('INSERT OR REPLACE INTO runtime_metrics VALUES(?,?,?,?)').run(runId, conversationId, JSON.stringify(metrics), stamp()); }
  beginProviderRequest({ requestId, conversationId, runId, api, model }) {
    const conversation = this.conversation(conversationId);
    this.db.prepare('INSERT INTO provider_usage(request_id,project_id,conversation_id,run_id,api,model,created_at) VALUES(?,?,?,?,?,?,?)').run(requestId, conversation.projectId, conversationId, runId, api, String(model).slice(0, 200), stamp());
  }
  recordProviderUsage(requestId, usage) {
    const row = this.db.prepare('SELECT usage_json FROM provider_usage WHERE request_id=?').get(requestId);
    if (!row) throw hostError('host_provider_usage_request_unavailable');
    const previous = row.usage_json ? JSON.parse(row.usage_json) : {};
    const next = { ...previous };
    for (const [key, value] of Object.entries(usage)) if (Number.isSafeInteger(value) && value >= 0) next[key] = value;
    this.db.prepare('UPDATE provider_usage SET usage_json=? WHERE request_id=?').run(JSON.stringify(next), requestId);
  }
  providerUsage(runId) {
    const rows = this.db.prepare('SELECT usage_json FROM provider_usage WHERE run_id=?').all(runId);
    const reported = rows.filter(row => row.usage_json).map(row => JSON.parse(row.usage_json));
    const totals = {};
    for (const field of ['inputTokens','outputTokens','totalTokens','cachedInputTokens','reasoningTokens']) {
      const values = reported.map(row => row[field]).filter(value => Number.isSafeInteger(value) && value >= 0);
      totals[field] = values.length ? values.reduce((a, b) => a + b, 0) : null;
    }
    return { requests: rows.length, reportedRequests: reported.length, unknownRequests: rows.length - reported.length, ...totals, source: 'provider_reported', price: null };
  }
  tasks(conversationId, projectId) {
    this.conversation(conversationId, projectId);
    return this.db.prepare('SELECT payload FROM conversation_tasks WHERE conversation_id=? ORDER BY rowid').all(String(conversationId)).map(row => JSON.parse(row.payload));
  }
  task(conversationId, taskId, projectId) {
    this.conversation(conversationId, projectId);
    const row = taskId === undefined
      ? this.db.prepare("SELECT payload FROM conversation_tasks WHERE conversation_id=? AND json_extract(payload,'$.parentTaskId') IS NULL ORDER BY rowid DESC LIMIT 1").get(String(conversationId))
      : this.db.prepare('SELECT payload FROM conversation_tasks WHERE conversation_id=? AND id=?').get(String(conversationId), String(taskId));
    if (!row && taskId !== undefined) throw hostError('host_task_unavailable', 404);
    return row ? JSON.parse(row.payload) : null;
  }
  createTask(conversationId, goal, metadata = {}) {
    const task = { ...createTask(this.conversation(conversationId), goal), ...metadata };
    return this.saveTask(task);
  }
  saveTask(task) {
    this.conversation(task.conversationId, task.projectId);
    const existing = this.db.prepare('SELECT conversation_id,payload FROM conversation_tasks WHERE id=?').get(task.id);
    if (existing && (existing.conversation_id !== task.conversationId || JSON.parse(existing.payload).revision !== task.revision)) throw hostError('host_task_revision_conflict', 409);
    const next = { ...task, revision: task.revision + 1 };
    this.db.prepare('INSERT INTO conversation_tasks(id,conversation_id,payload,created_at,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at WHERE conversation_tasks.conversation_id=excluded.conversation_id')
      .run(next.id, next.conversationId, JSON.stringify(next), next.createdAt, next.updatedAt);
    return next;
  }
  applications() { return this.db.prepare('SELECT * FROM applications ORDER BY id').all().map(row => ({ ...row, enabled: Boolean(row.enabled), builtin: Boolean(row.builtin), uninstallable: row.uninstallable === undefined || row.uninstallable === null ? true : Boolean(row.uninstallable), manifest: JSON.parse(row.manifest) })); }
  application(id) { const app = this.applications().find(row => row.id === id); if (!app) throw hostError('host_app_unavailable', 404); return app; }
  grant(appId, projectId) {
    this.project(projectId);
    const row = this.db.prepare('SELECT permissions,revision,disclosure_runtime AS disclosureRuntime FROM application_grants WHERE app_id=? AND project_id=?').get(appId, Number(projectId));
    return row ? { permissions: JSON.parse(row.permissions), revision: row.revision, disclosureRuntime: row.disclosureRuntime } : { permissions: [], revision: 0, disclosureRuntime: '' };
  }
  grants(appId) { return this.db.prepare('SELECT project_id AS projectId,permissions,revision,disclosure_runtime AS disclosureRuntime FROM application_grants WHERE app_id=?').all(appId).map(row => ({ ...row, permissions: JSON.parse(row.permissions) })); }
  conversationMemory(id) {
    const row = this.db.prepare('SELECT payload FROM conversation_memory WHERE conversation_id=?').get(String(id));
    if (!row?.payload) return { turns: [], facts: [] };
    try { return JSON.parse(row.payload); } catch { return { turns: [], facts: [] }; }
  }
  saveConversationMemory(id, payload) {
    this.conversation(id);
    this.db.prepare('INSERT INTO conversation_memory(conversation_id,payload,updated_at) VALUES(?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at')
      .run(String(id), JSON.stringify(payload ?? { turns: [], facts: [] }), stamp());
  }
}
