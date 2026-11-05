import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import {
  writeProjectMemoryEntry,
  readProjectMemory,
  forgetProjectMemory,
  formatProjectMemoryPromptBlock,
  PROJECT_MEMORY_LIMITS,
} from '../src/memory.mjs';

function tempStore() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-project-memory-'));
  const store = new HostStore(join(dir, 'host.sqlite'));
  return { store, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

test('remember writes a project entry that recall returns and formats with its id + non-authority disclaimer', () => {
  const { store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '1' });
    const written = writeProjectMemoryEntry(store, project.id, { note: '该站点登录走 /api/v2/auth,返回 JWT', kind: 'fact', sourceConversationId: 'conv-1' });
    assert.equal(written.ok, true);
    assert.ok(written.entryId);

    const entries = readProjectMemory(store, project.id);
    assert.equal(entries.length, 1);
    assert.equal(entries[0].note, '该站点登录走 /api/v2/auth,返回 JWT');
    assert.equal(entries[0].sourceConversationId, 'conv-1');

    const block = formatProjectMemoryPromptBlock(entries);
    assert.match(block, /该站点登录走/);
    assert.match(block, new RegExp(written.entryId));
    assert.match(block, /不是授权/); // recall is reference, never authority
  } finally { cleanup(); }
});

test('remember dedups an identical note (touch, not duplicate)', () => {
  const { store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '1' });
    const first = writeProjectMemoryEntry(store, project.id, { note: '同一条笔记' });
    const second = writeProjectMemoryEntry(store, project.id, { note: '  同一条笔记  ' }); // whitespace-normalized dup
    assert.equal(first.ok, true);
    assert.equal(second.ok, true);
    assert.equal(second.deduped, true);
    assert.equal(second.entryId, first.entryId);
    assert.equal(readProjectMemory(store, project.id).length, 1);
  } finally { cleanup(); }
});

test('entries are bounded to the max; oldest are pruned', () => {
  const { store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '1' });
    const max = PROJECT_MEMORY_LIMITS.maxEntries;
    for (let i = 0; i < max + 5; i++) writeProjectMemoryEntry(store, project.id, { note: `note number ${i}` });
    const entries = readProjectMemory(store, project.id);
    assert.equal(entries.length, max);
    // The newest notes survive; the very first ones are pruned.
    const notes = entries.map(entry => entry.note);
    assert.ok(notes.includes(`note number ${max + 4}`));
    assert.ok(!notes.includes('note number 0'));
  } finally { cleanup(); }
});

test('an obvious secret block is refused, empty note is refused', () => {
  const { store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '1' });
    const secret = writeProjectMemoryEntry(store, project.id, { note: '-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----' });
    assert.equal(secret.ok, false);
    assert.equal(secret.reason, 'looks_like_secret');
    const empty = writeProjectMemoryEntry(store, project.id, { note: '   ' });
    assert.equal(empty.ok, false);
    assert.equal(readProjectMemory(store, project.id).length, 0);
  } finally { cleanup(); }
});

test('memory is isolated per project and forget removes one entry', () => {
  const { store, cleanup } = tempStore();
  try {
    const a = store.createProject('a', { externalId: '1' });
    const b = store.createProject('b', { externalId: '2' });
    const written = writeProjectMemoryEntry(store, a.id, { note: '只属于 A 的笔记' });
    assert.equal(readProjectMemory(store, b.id).length, 0); // not visible in B
    assert.equal(readProjectMemory(store, a.id).length, 1);

    assert.equal(forgetProjectMemory(store, a.id, written.entryId), true);
    assert.equal(readProjectMemory(store, a.id).length, 0);
    assert.equal(forgetProjectMemory(store, a.id, 'no-such-id'), false);
  } finally { cleanup(); }
});

test('recall of empty memory is an empty string (no block injected)', () => {
  const { store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '1' });
    assert.equal(formatProjectMemoryPromptBlock(readProjectMemory(store, project.id)), '');
  } finally { cleanup(); }
});
