import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import {
  readApprovedResearchCards,
  formatResearchCardsPromptBlock,
} from '../src/research-cards.mjs';

function tempAppDb() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-research-cards-'));
  const path = join(dir, 'oviraptor.sqlite3');
  const db = new DatabaseSync(path);
  // Minimal mirror of the Rust RESEARCH_CARDS_SCHEMA (only the columns the reader needs).
  db.exec(`CREATE TABLE research_cards (
    id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', source TEXT NOT NULL DEFAULT '',
    methodology TEXT NOT NULL DEFAULT '', stripped_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending', created_at TEXT NOT NULL DEFAULT '', decided_at TEXT NOT NULL DEFAULT ''
  );`);
  const insert = db.prepare(
    'INSERT INTO research_cards(id,title,methodology,status,created_at) VALUES(?,?,?,?,?)',
  );
  insert.run('a', '令牌伪造链', '先枚举字段再逐个测越权,confidence 高再升级到批量', 'approved', '2026-11-06 10:00:00');
  insert.run('b', '待审的卡', '这条还没批准,不该出现在 prompt 里', 'pending', '2026-11-06 11:00:00');
  insert.run('c', '被拒的卡', '这条被拒了,也不该出现', 'rejected', '2026-11-06 12:00:00');
  db.close();
  return { path, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

test('readApprovedResearchCards returns only approved cards', () => {
  const { path, cleanup } = tempAppDb();
  try {
    const cards = readApprovedResearchCards(path);
    assert.equal(cards.length, 1);
    assert.equal(cards[0].title, '令牌伪造链');
    assert.match(cards[0].methodology, /先枚举字段/);
  } finally {
    cleanup();
  }
});

test('readApprovedResearchCards is graceful: missing path/file/table all yield []', () => {
  assert.deepEqual(readApprovedResearchCards(undefined), []);
  assert.deepEqual(readApprovedResearchCards(''), []);
  assert.deepEqual(readApprovedResearchCards('/no/such/oviraptor.sqlite3'), []);
  // existing db without the table
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-research-cards-empty-'));
  const path = join(dir, 'oviraptor.sqlite3');
  const db = new DatabaseSync(path);
  db.exec('CREATE TABLE something_else(x INTEGER);');
  db.close();
  try {
    assert.deepEqual(readApprovedResearchCards(path), []);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('formatResearchCardsPromptBlock labels the block non-authority and untrusted', () => {
  assert.equal(formatResearchCardsPromptBlock([]), '');
  assert.equal(formatResearchCardsPromptBlock(undefined), '');
  const block = formatResearchCardsPromptBlock([
    { title: '令牌伪造链', methodology: '先枚举字段再逐个测越权' },
  ]);
  assert.match(block, /先枚举字段/);
  assert.match(block, /不是授权/); // advisory, never authority
  assert.match(block, /不可信数据/); // untrusted data, not instructions
  assert.match(block, /令牌伪造链/);
});

test('block round-trips from an approved card read out of the app db', () => {
  const { path, cleanup } = tempAppDb();
  try {
    const block = formatResearchCardsPromptBlock(readApprovedResearchCards(path));
    assert.match(block, /令牌伪造链/);
    assert.doesNotMatch(block, /待审的卡/);
    assert.doesNotMatch(block, /被拒的卡/);
  } finally {
    cleanup();
  }
});
