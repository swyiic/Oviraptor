import { DatabaseSync } from 'node:sqlite';
import { randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { initializeNativeOwners } from './native-owners.mjs';

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
      CREATE TABLE IF NOT EXISTS applications(id TEXT PRIMARY KEY,version TEXT NOT NULL,digest TEXT NOT NULL,path TEXT NOT NULL,manifest TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 0,revision INTEGER NOT NULL DEFAULT 1,status TEXT NOT NULL DEFAULT 'installed');
      CREATE TABLE IF NOT EXISTS application_grants(app_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,project_id INTEGER NOT NULL REFERENCES projects(id),permissions TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(app_id,project_id));
      CREATE TABLE IF NOT EXISTS host_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS runtime_metrics(run_id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL,metrics TEXT NOT NULL,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS conversation_memory(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,payload TEXT NOT NULL,updated_at TEXT NOT NULL);`);
    const columns = table => new Set(this.db.prepare(`PRAGMA table_info(${table})`).all().map(row => row.name));
    const messageColumns = columns('messages');
    for (const [name, declaration] of [['outcome_unknown', 'INTEGER NOT NULL DEFAULT 0'], ['failure_code', "TEXT NOT NULL DEFAULT ''"], ['stop_reason', "TEXT NOT NULL DEFAULT ''"], ['cancel_requested', 'INTEGER NOT NULL DEFAULT 0']]) {
      if (!messageColumns.has(name)) this.db.exec(`ALTER TABLE messages ADD COLUMN ${name} ${declaration}`);
    }
    this.db.exec("UPDATE messages SET status='interrupted',outcome_unknown=1,failure_code='host_interrupted_restart',stop_reason='' WHERE status='running' OR (status='interrupted' AND failure_code='')");
    const appColumns = columns('applications');
    if (!appColumns.has('data_path')) this.db.exec('ALTER TABLE applications ADD COLUMN data_path TEXT');
    if (!appColumns.has('data_version')) this.db.exec("ALTER TABLE applications ADD COLUMN data_version TEXT NOT NULL DEFAULT 'legacy'");
    if (!appColumns.has('builtin')) this.db.exec('ALTER TABLE applications ADD COLUMN builtin INTEGER NOT NULL DEFAULT 0');
    if (!appColumns.has('uninstallable')) this.db.exec('ALTER TABLE applications ADD COLUMN uninstallable INTEGER NOT NULL DEFAULT 1');
    const grantColumns = columns('application_grants');
    if (!grantColumns.has('disclosure_runtime')) this.db.exec("ALTER TABLE application_grants ADD COLUMN disclosure_runtime TEXT NOT NULL DEFAULT ''");
    const projectColumns = columns('projects');
    if (!projectColumns.has('external_id')) this.db.exec('ALTER TABLE projects ADD COLUMN external_id TEXT');
    this.db.exec('CREATE UNIQUE INDEX IF NOT EXISTS projects_external_id_unique ON projects(external_id) WHERE external_id IS NOT NULL');
    this.db.exec(`CREATE TABLE IF NOT EXISTS application_versions(sequence INTEGER PRIMARY KEY,app_id TEXT NOT NULL,version TEXT NOT NULL,digest TEXT NOT NULL,path TEXT NOT NULL,manifest TEXT NOT NULL,data_path TEXT,data_version TEXT NOT NULL,replaced_at TEXT NOT NULL,replaced_by_version TEXT NOT NULL);`);
    this.db.exec(`CREATE TABLE IF NOT EXISTS application_retained(app_id TEXT PRIMARY KEY,version TEXT NOT NULL,digest TEXT NOT NULL,data_path TEXT,data_version TEXT NOT NULL,data_schema_version INTEGER NOT NULL,retained_at TEXT NOT NULL);`);
    initializeNativeOwners(this.db);
  }
  close() { this.db.close(); }
  projects() { return this.db.prepare('SELECT id,name,external_id AS externalId,created_at AS createdAt FROM projects ORDER BY id').all(); }
  project(id) {
    const row = this.db.prepare('SELECT id,name,external_id AS externalId FROM projects WHERE id=?').get(positiveId(id));
    if (!row) throw hostError('host_project_unavailable', 404);
    return row;
  }
  createProject(name, { externalId } = {}) {
    const value = boundedText(name, 256);
    const external = externalId == null || externalId === '' ? null : String(externalId).slice(0, 128);
    const result = this.db.prepare('INSERT INTO projects(name,created_at,external_id) VALUES(?,?,?)').run(value, stamp(), external);
    return this.project(Number(result.lastInsertRowid));
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
  createConversation(projectId, title = 'New conversation') {
    this.project(projectId);
    const id = randomUUID();
    this.db.prepare('INSERT INTO conversations(id,project_id,title,created_at) VALUES(?,?,?,?)').run(id, Number(projectId), boundedText(title, 256), stamp());
    return this.conversation(id);
  }
  session(id, sessionId) { this.conversation(id); this.db.prepare('UPDATE conversations SET session_id=? WHERE id=?').run(sessionId, id); }
  messages(id, projectId) {
    this.conversation(id, projectId);
    return this.db.prepare('SELECT id,role,content,status,run_id AS runId,created_at AS createdAt,outcome_unknown AS outcomeUnknown,failure_code AS failureCode,stop_reason AS stopReason,cancel_requested AS cancelRequested FROM messages WHERE conversation_id=? ORDER BY rowid').all(id).map(row => ({ ...row, outcomeUnknown: Boolean(row.outcomeUnknown), cancelRequested: Boolean(row.cancelRequested) }));
  }
  addMessage(id, role, content, status = 'completed', runId = null) {
    this.conversation(id);
    const message = { id: randomUUID(), role, content, status, runId, createdAt: stamp() };
    this.db.prepare('INSERT INTO messages(id,conversation_id,role,content,status,run_id,created_at) VALUES(?,?,?,?,?,?,?)').run(message.id, id, role, content, status, runId, message.createdAt);
    return message;
  }
  finishMessage(id, content, status, details = {}) { this.db.prepare('UPDATE messages SET content=?,status=?,outcome_unknown=?,failure_code=?,stop_reason=?,cancel_requested=? WHERE id=?').run(content, status, Number(Boolean(details.outcomeUnknown)), details.failureCode ?? '', details.stopReason ?? '', Number(Boolean(details.cancelRequested)), id); }
  metrics(runId, conversationId, metrics) { this.db.prepare('INSERT OR REPLACE INTO runtime_metrics VALUES(?,?,?,?)').run(runId, conversationId, JSON.stringify(metrics), stamp()); }
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
