import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';

// The structured "what next" options must survive a DB round-trip (migration,
// finishMessage write, and both message-list read paths) — not just the live
// event — so a reopened conversation still shows the buttons.
test('suggestedNext persists through finishMessage and both message-read paths', t => {
  const dir = mkdtempSync(join(tmpdir(), 'suggested-next-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = new HostStore(join(dir, 'host.sqlite'));
  const proj = store.createProject('P', { externalId: '1' });
  const conv = store.createConversation(proj.id, 'chat');
  const message = store.addMessage(conv.id, 'assistant', '', 'running');
  const suggestedNext = [{ id: 'continue_current', prompt: '继续：下一步' }, { id: 'end_topic' }];

  store.finishMessage(message.id, '完成。', 'completed', { stopReason: 'end_turn', suggestedNext });

  const viaMessages = store.messages(conv.id).find(row => row.id === message.id);
  assert.deepEqual(viaMessages.suggestedNext, suggestedNext);

  const page = store.messagesPage(conv.id, undefined, { limit: 10 });
  const viaPage = page.items.find(row => row.id === message.id);
  assert.deepEqual(viaPage.suggestedNext, suggestedNext);

  // A message finished without suggestedNext (the common case) round-trips to [].
  const plain = store.addMessage(conv.id, 'assistant', '', 'running');
  store.finishMessage(plain.id, '好的。', 'completed', { stopReason: 'end_turn' });
  assert.deepEqual(store.messages(conv.id).find(row => row.id === plain.id).suggestedNext, []);
});
