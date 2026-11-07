import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';

// The read-root a large cited file lives in must survive a follow-up "继续" turn that
// does not re-paste the path — otherwise workspace_run loses the root and the agent
// (even a strong model) reports "沙箱打不开原路径". This covers the store mechanism
// #persistAndLoadCitedRoots relies on: per-conversation scope, idempotency, cascade.
test('cited read-roots persist per conversation and survive a path-less continue turn', t => {
  const dir = mkdtempSync(join(tmpdir(), 'cited-roots-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = new HostStore(join(dir, 'host.sqlite'));
  const proj = store.createProject('P', { externalId: '1' });
  const conv = store.createConversation(proj.id, 'chat');
  const put = store.db.prepare('INSERT OR IGNORE INTO conversation_cited_roots(conversation_id,root,created_at) VALUES(?,?,?)');
  const list = id => store.db.prepare('SELECT root FROM conversation_cited_roots WHERE conversation_id=? ORDER BY created_at').all(id).map(r => r.root);

  // Turn 1: user cites a big external file (Chinese path) → its directory is persisted.
  const root = '/Users/swyiic/Desktop/项目/IDSS/交付/2026-11月（广州巡检）/Resources';
  put.run(conv.id, root, new Date().toISOString());

  // Turn 2 ("继续没做完的", no path): the root is still there for this conversation.
  assert.deepEqual(list(conv.id), [root]);

  // Idempotent: re-citing the same path does not duplicate.
  put.run(conv.id, root, new Date().toISOString());
  assert.equal(list(conv.id).length, 1);

  // Scoped: a different conversation never inherits another's cited roots.
  const other = store.createConversation(proj.id, 'other');
  assert.deepEqual(list(other.id), []);

  // FK cascade: deleting the conversation removes its roots (no orphan grant lingers).
  store.db.prepare('DELETE FROM conversations WHERE id=?').run(conv.id);
  assert.deepEqual(list(conv.id), []);
});
