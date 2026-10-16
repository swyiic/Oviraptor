import { DatabaseSync } from 'node:sqlite';
import { randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

export const hostError = (code, status = 400) => Object.assign(new Error(code), { code, status });
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
    this.db.exec(`PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
      CREATE TABLE IF NOT EXISTS projects(id INTEGER PRIMARY KEY,name TEXT NOT NULL,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),title TEXT NOT NULL,session_id TEXT,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),role TEXT NOT NULL CHECK(role IN ('user','assistant')),content TEXT NOT NULL,status TEXT NOT NULL,run_id TEXT,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS applications(id TEXT PRIMARY KEY,version TEXT NOT NULL,digest TEXT NOT NULL,path TEXT NOT NULL,manifest TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 0,revision INTEGER NOT NULL DEFAULT 1,status TEXT NOT NULL DEFAULT 'installed');
      CREATE TABLE IF NOT EXISTS application_grants(app_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,project_id INTEGER NOT NULL REFERENCES projects(id),permissions TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(app_id,project_id));
      CREATE TABLE IF NOT EXISTS host_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS runtime_metrics(run_id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL,metrics TEXT NOT NULL,created_at TEXT NOT NULL);
      UPDATE messages SET status='interrupted' WHERE status='running';`);
  }
  close() { this.db.close(); }
  projects() { return this.db.prepare('SELECT id,name,created_at AS createdAt FROM projects ORDER BY id').all(); }
  project(id) {
    const row = this.db.prepare('SELECT id,name FROM projects WHERE id=?').get(positiveId(id));
    if (!row) throw hostError('host_project_unavailable', 404);
    return row;
  }
  createProject(name) {
    const value = boundedText(name, 256);
    const result = this.db.prepare('INSERT INTO projects(name,created_at) VALUES(?,?)').run(value, stamp());
    return this.project(Number(result.lastInsertRowid));
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
    return this.db.prepare('SELECT id,role,content,status,run_id AS runId,created_at AS createdAt FROM messages WHERE conversation_id=? ORDER BY rowid').all(id);
  }
  addMessage(id, role, content, status = 'completed', runId = null) {
    this.conversation(id);
    const message = { id: randomUUID(), role, content, status, runId, createdAt: stamp() };
    this.db.prepare('INSERT INTO messages(id,conversation_id,role,content,status,run_id,created_at) VALUES(?,?,?,?,?,?,?)').run(message.id, id, role, content, status, runId, message.createdAt);
    return message;
  }
  finishMessage(id, content, status) { this.db.prepare('UPDATE messages SET content=?,status=? WHERE id=?').run(content, status, id); }
  metrics(runId, conversationId, metrics) { this.db.prepare('INSERT OR REPLACE INTO runtime_metrics VALUES(?,?,?,?)').run(runId, conversationId, JSON.stringify(metrics), stamp()); }
  applications() { return this.db.prepare('SELECT * FROM applications ORDER BY id').all().map(row => ({ ...row, enabled: Boolean(row.enabled), manifest: JSON.parse(row.manifest) })); }
  application(id) { const app = this.applications().find(row => row.id === id); if (!app) throw hostError('host_app_unavailable', 404); return app; }
  grant(appId, projectId) {
    this.project(projectId);
    const row = this.db.prepare('SELECT permissions,revision FROM application_grants WHERE app_id=? AND project_id=?').get(appId, Number(projectId));
    return row ? { permissions: JSON.parse(row.permissions), revision: row.revision } : { permissions: [], revision: 0 };
  }
  grants(appId) { return this.db.prepare('SELECT project_id AS projectId,permissions,revision FROM application_grants WHERE app_id=?').all(appId).map(row => ({ ...row, permissions: JSON.parse(row.permissions) })); }
}
