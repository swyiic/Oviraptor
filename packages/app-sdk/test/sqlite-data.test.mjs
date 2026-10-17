import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { realpathSync } from 'node:fs';
import {
  check, text, context, exact, open, operation, result, scope, object, legacyRead,
} from '../src/index.mjs';

test('sqlite-data validates context text and exact keys', () => {
  assert.equal(object({ a: 1 }), true);
  assert.equal(object([1]), false);
  assert.equal(text('ok', 8), 'ok');
  assert.throws(() => text('', 8), /app_arguments_invalid/);
  assert.throws(() => text('x\u0000y', 8), /app_arguments_invalid/);
  assert.equal(context({ hostBoundProjectId: 3, grantRevision: 0, operationId: 'op' }), 3);
  assert.throws(() => context({ hostBoundProjectId: 0, grantRevision: 0, operationId: 'op' }), /app_context_invalid/);
  exact({ a: 1 }, ['a']);
  assert.throws(() => exact({ a: 1, b: 2 }, ['a']), /app_arguments_invalid/);
});

test('sqlite-data open and operation are project-scoped and idempotent', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'oviraptor-sdk-sqlite-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const dataDir = realpathSync(dir);
  const db = open(dataDir, 'demo.sqlite3');
  const ctx = { hostBoundProjectId: 11, grantRevision: 1, operationId: 'once' };
  const args = { title: 'A' };
  let writes = 0;
  const first = operation(db, ctx, 'demo_write', args, project => {
    writes += 1;
    return { ...scope(project), ok: true };
  });
  assert.equal(first.ok, true);
  assert.equal(first.hostBoundProjectId, 11);
  assert.equal(writes, 1);
  const again = operation(db, ctx, 'demo_write', args, () => { writes += 1; return { ok: false }; });
  assert.deepEqual(again, first);
  assert.equal(writes, 1);
  assert.throws(() => operation(db, ctx, 'demo_write', { title: 'B' }, () => ({})), /app_operation_conflict/);
  const payload = result({ total: 2 });
  assert.equal(payload.structuredContent.total, 2);
  assert.equal(JSON.parse(payload.content[0].text).total, 2);
  db.close();
});

test('legacy-bridge-stub reports unavailable without adapter and forwards with one', async () => {
  assert.deepEqual(await legacyRead(null, 'app', 'stats', {}, { hostBoundProjectId: 1 }), {
    available: false, code: 'legacy_managed_bridge_unavailable', executionAvailable: false,
  });
  const seen = [];
  const reply = await legacyRead(async input => { seen.push(input); return { available: true, total: 4 }; },
    'org.oviraptor.assets', 'snapshot_stats', { q: 1 }, { hostBoundProjectId: 9, grantRevision: 0, operationId: 'x' });
  assert.equal(reply.total, 4);
  assert.equal(seen[0].appId, 'org.oviraptor.assets');
  assert.equal(seen[0].name, 'snapshot_stats');
  assert.deepEqual(seen[0].arguments, { q: 1 });
  assert.equal(seen[0].context.hostBoundProjectId, 9);
});
