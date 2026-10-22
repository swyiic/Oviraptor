import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

async function fixture(t, name = 'ovi-chat-metadata-') {
  const dir = await mkdtemp(join(tmpdir(), name));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
}

test('metadata upgrades an old Host DB without changing titles, sessions, message timestamps or content', async t => {
  const dir = await fixture(t), file = join(dir, 'host.sqlite');
  const old = new DatabaseSync(file);
  old.exec(`CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT NOT NULL,created_at TEXT NOT NULL);
    CREATE TABLE conversations(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),title TEXT NOT NULL,session_id TEXT,created_at TEXT NOT NULL);
    CREATE TABLE messages(id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),role TEXT NOT NULL,content TEXT NOT NULL,status TEXT NOT NULL,run_id TEXT,created_at TEXT NOT NULL);`);
  old.prepare('INSERT INTO projects VALUES(?,?,?)').run(1, 'Preserved project', '2020-01-01T00:00:00.000Z');
  old.prepare('INSERT INTO conversations VALUES(?,?,?,?,?)').run('original-chat', 1, '原始标题', 'original-session', '2020-01-02T00:00:00.000Z');
  old.prepare('INSERT INTO conversations VALUES(?,?,?,?,?)').run('empty-chat', 1, '空会话', null, '2020-01-04T00:00:00.000Z');
  old.prepare('INSERT INTO messages VALUES(?,?,?,?,?,?,?)').run('original-message', 'original-chat', 'user', '不可修改的原文', 'completed', null, '2020-01-03T00:00:00.000Z');
  old.close();
  let store = new HostStore(file);
  assert.deepEqual(store.conversations(1).map(({ id, createdAt, lastActivityAt, pinned }) => ({ id, createdAt, lastActivityAt, pinned })), [
    { id: 'original-chat', createdAt: '2020-01-02T00:00:00.000Z', lastActivityAt: '2020-01-03T00:00:00.000Z', pinned: false },
    { id: 'empty-chat', createdAt: '2020-01-04T00:00:00.000Z', lastActivityAt: '2020-01-04T00:00:00.000Z', pinned: false },
  ]);
  const before = store.messages('original-chat');
  const saved = store.updateConversationMetadata('original-chat', { projectId: 1, title: '新对话', pinned: true });
  assert.equal(saved.titleManuallySet, true);
  assert.equal(saved.createdAt, '2020-01-02T00:00:00.000Z');
  assert.equal(saved.lastActivityAt, '2020-01-03T00:00:00.000Z');
  assert.equal(saved.sessionId, 'original-session');
  assert.deepEqual(store.messages('original-chat'), before);
  store.close();
  store = new HostStore(file);
  try {
    assert.equal(store.conversation('original-chat', 1).pinned, true);
    assert.equal(store.conversation('original-chat', 1).title, '新对话');
    assert.equal(store.conversation('original-chat', 1).titleManuallySet, true);
    assert.equal(store.conversation('original-chat', 1).sessionId, 'original-session');
    assert.deepEqual(store.messages('original-chat'), before);
    assert.deepEqual(store.conversations(1).map(row => row.id), ['original-chat', 'empty-chat'], 'pin does not silently reorder the list API');
    assert.equal(store.updateConversationMetadata('original-chat', { projectId: 1, pinned: false }).pinned, false);
  } finally { store.close(); }
});

test('invalid metadata and wrong-project requests never mutate title, pin, messages or grants', () => {
  const store = new HostStore(':memory:');
  try {
    const p1 = store.createProject('One'), p2 = store.createProject('Two');
    const c = store.createConversation(p1.id, 'Original');
    store.addMessage(c.id, 'user', 'Retain this');
    const before = store.conversation(c.id), messages = store.messages(c.id);
    const changes = store.db.prepare('SELECT total_changes() AS n').get().n;
    for (const input of [null, [], {}, { title: 'Missing project' }, { projectId: p1.id },
      { projectId: p1.id, title: '' }, { projectId: p1.id, title: '  ' }, { projectId: p1.id, title: 'a'.repeat(257) },
      { projectId: p1.id, title: 123 }, { projectId: p1.id, pinned: 1 }, { projectId: p1.id, pinned: 'true' },
      { projectId: p1.id, pinned: null }, { projectId: p1.id, title: 'Okay', sessionId: 'injected' },
      { projectId: p2.id, pinned: true }, { projectId: p2.id, title: 'Other project' }, { projectId: null, pinned: true }]) {
      assert.throws(() => store.updateConversationMetadata(c.id, input));
    }
    assert.equal(store.db.prepare('SELECT total_changes() AS n').get().n, changes);
    assert.deepEqual(store.conversation(c.id), before);
    assert.deepEqual(store.messages(c.id), messages);
    assert.deepEqual(store.db.prepare('SELECT * FROM conversation_workspace_grants').all(), []);
    assert.deepEqual(store.db.prepare('SELECT * FROM application_grants').all(), []);
  } finally { store.close(); }
});

test('renaming to a placeholder remains manual on subsequent turns and emits scoped metadata', async t => {
  const dir = await fixture(t), events = [];
  const host = new AgentHost({ dataDir: dir, runtimeSelection: 'none', onEvent: event => events.push(event) });
  t.after(() => host.close());
  const p = host.store.createProject('Manual title');
  const c = host.store.createConversation(p.id);
  const saved = host.updateConversation(c.id, { projectId: p.id, title: '新对话', pinned: true });
  assert.equal(saved.pinned, true);
  assert.equal(events.at(-1).type, 'conversation.updated');
  assert.equal(events.at(-1).conversationId, c.id);
  assert.equal(events.at(-1).projectId, p.id);
  assert.equal(events.at(-1).pinned, true);
  await host.send(c.id, { projectId: p.id, text: '这条消息不应该替换手动标题' });
  assert.equal(host.store.conversation(c.id).title, '新对话');
  assert.equal(events.filter(row => row.type === 'conversation.updated').length, 1);
  assert.equal(host.store.conversation(c.id).lastActivityAt, host.store.messages(c.id).at(-1).createdAt);
});

test('PATCH is desktop-CORS accessible, session-authenticated, project bound and survives restart', async t => {
  const dir = await fixture(t);
  let host = new AgentHost({ dataDir: dir, runtimeSelection: 'none' });
  let web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); });
  const p1 = host.store.createProject('One'), p2 = host.store.createProject('Two');
  const c = host.store.createConversation(p1.id);
  const patch = (input, auth = true) => fetch(`${web.origin}/api/conversations/${c.id}`, { method: 'PATCH',
    headers: { Origin: 'tauri://localhost', 'Content-Type': 'application/json', ...(auth ? { 'X-Oviraptor-Session': web.token } : {}) }, body: JSON.stringify(input) });
  const options = await fetch(`${web.origin}/api/conversations/${c.id}`, { method: 'OPTIONS', headers: { Origin: 'tauri://localhost', 'Access-Control-Request-Method': 'PATCH' } });
  assert.equal(options.status, 204);
  assert.match(options.headers.get('Access-Control-Allow-Methods'), /\bPATCH\b/);
  assert.equal((await patch({ projectId: p1.id, title: 'Unauthorized' }, false)).status, 403);
  assert.equal((await patch({ projectId: p2.id, title: 'Wrong project', pinned: true })).status, 404);
  assert.equal((await patch({ projectId: p1.id, pinned: 'true' })).status, 400);
  const response = await patch({ projectId: p1.id, title: '命名后的会话', pinned: true });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('Access-Control-Allow-Origin'), 'tauri://localhost');
  const row = await response.json();
  assert.equal(row.id, c.id); assert.equal(row.projectId, p1.id); assert.equal(row.title, '命名后的会话'); assert.equal(row.pinned, true);
  assert.equal(row.createdAt, row.lastActivityAt);
  await web.close(); await host.close();
  host = new AgentHost({ dataDir: dir, runtimeSelection: 'none' }); web = await serveHost(host);
  const listed = await (await fetch(`${web.origin}/api/conversations?projectId=${p1.id}`, { headers: { 'X-Oviraptor-Session': web.token } })).json();
  assert.equal(listed.items[0].pinned, true);
  assert.equal(listed.items[0].title, '命名后的会话');
  assert.equal(listed.items[0].createdAt, row.createdAt);
});
