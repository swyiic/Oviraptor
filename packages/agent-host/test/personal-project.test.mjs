import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';

function tempStore() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-personal-'));
  const store = new HostStore(join(dir, 'host.sqlite'));
  return { store, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

test('ensurePersonalProject is idempotent and kind=personal', () => {
  const { store, cleanup } = tempStore();
  try {
    const a = store.ensurePersonalProject();
    const b = store.ensurePersonalProject();
    assert.equal(a.id, b.id);           // same reserved row, not a duplicate
    assert.equal(a.kind, 'personal');
    assert.equal(a.externalId, '__personal__');
    // it is the only personal project
    assert.equal(store.projects().filter(p => p.kind === 'personal').length, 1);
  } finally { cleanup(); }
});

test('a direct-chat conversation can be created in the personal space', () => {
  const { store, cleanup } = tempStore();
  try {
    const personal = store.ensurePersonalProject();
    const conv = store.createConversation(personal.id, 'direct chat');
    assert.equal(conv.projectId, personal.id);
    assert.equal(store.conversations(personal.id).length, 1);
  } finally { cleanup(); }
});
