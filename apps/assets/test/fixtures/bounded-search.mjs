import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { createApplication } from '../../application.mjs';

// Local metadata only. No URL is requested. The 32 MiB V8 old-space budget is
// deliberately smaller than retaining this complete ~60 MiB observation set.
const directory = process.argv[2];
const count = 30000;
const project = 91;
const config = JSON.parse(await readFile(new URL('../../app.package.json', import.meta.url), 'utf8'));
const app = createApplication(config.manifest);
await app.onStart({ dataDir: directory, context: { appId: config.manifest.id, hostApiVersion: '1.0.0' } });
const db = new DatabaseSync(join(directory, 'inventory.sqlite3'));
const rowHash = () => {
  const hash = createHash('sha256');
  for (const row of db.prepare('SELECT id,project_id,source,external_id,observation_json,received_at FROM inventory ORDER BY id').iterate()) hash.update(JSON.stringify(row));
  return hash.digest('hex');
};
try {
  const insert = db.prepare('INSERT INTO inventory(project_id,source,external_id,observation_json,received_at) VALUES(?,?,?,?,?)');
  const host = Array.from({ length: 4 }, () => 'h'.repeat(60)).join('.');
  db.exec('BEGIN IMMEDIATE');
  for (let index = 0; index < count; index++) {
    const value = { externalId: String(index), host, link: `https://asset.example.test/${'p'.repeat(940)}?record=${index}`,
      ip: '192.0.2.1', port: '443', protocol: 'https', domain: host,
      title: `${index % 1000 === 0 ? 'Needle ' : 'Plain '}${'t'.repeat(500)}`, statusCode: '200', probeOutcome: 'web_alive', observedAt: '2026-10-16T00:00:00Z' };
    insert.run(project, 'local-memory-fixture', String(index), JSON.stringify(value), '2026-10-16T00:00:01Z');
  }
  insert.run(92, 'different-project', 'other', JSON.stringify({ externalId: 'other', host: 'other.example.test', title: 'Needle' }), '2026-10-16T00:00:01Z');
  db.exec('COMMIT');
  const before = rowHash();
  const context = { hostBoundProjectId: project, grantRevision: 1, operationId: 'bounded-search' };
  const search = async args => (await app.tools.inventory_search(args, context)).structuredContent;
  const first = await search({ limit: 20 });
  assert.equal(first.total, count); assert.ok(first.returned > 0 && first.returned <= 20); assert.equal(first.hasMore, true);
  assert.equal(first.items[0].id, 1); assert.ok(Buffer.byteLength(JSON.stringify(first)) <= 24 * 1024);
  const second = await search({ limit: 20, cursor: first.nextCursor });
  assert.equal(second.total, count); assert.equal(second.items[0].id, first.items.at(-1).id + 1);
  assert.deepEqual(first.items.map(row => row.id), Array.from({ length: first.returned }, (_, i) => i + 1));
  const filtered = await search({ query: 'nEeDlE', limit: 7 });
  assert.equal(filtered.total, 30); assert.equal(filtered.returned, 7);
  assert.deepEqual(filtered.items.map(row => row.id), [1, 1001, 2001, 3001, 4001, 5001, 6001]);
  const filteredNext = await search({ query: 'nEeDlE', limit: 7, cursor: filtered.nextCursor });
  assert.equal(filteredNext.total, 30); assert.equal(filteredNext.items[0].id, 7001);
  assert.equal(rowHash(), before, 'Search must preserve the complete original inventory rowset');
  process.stdout.write(`${JSON.stringify({ passed: true, seeded: count, matchingProjectTotal: first.total, filteredTotal: filtered.total,
    firstReturned: first.returned, secondFirstId: second.items[0].id, databaseRowsUnchanged: true, oldSpaceMb: 32 })}\n`);
} finally { db.close(); await app.onStop(); }
