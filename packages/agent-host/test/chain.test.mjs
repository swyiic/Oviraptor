import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { addHop, fileHops, hopsFromTool, listHops } from '../src/chain.mjs';

test('a fetched page and a file excerpt become one chain', () => {
  const db = new DatabaseSync(':memory:');
  const fetched = hopsFromTool('workspace_fetch', { url: 'https://example.test/login' }, { url: 'https://example.test/login', status: 200, path: 'cited-web/example.test/fetched/page.html' });
  const file = fileHops([{ source: '/tmp/app.mec', bytes: 20 }]);
  const excerpt = hopsFromTool('workspace_source_search', {}, { matches: [{ path: '/tmp/app.mec', offset: 12, text: 'AES.decrypt' }] });
  for (const hop of [...fetched, ...file, ...excerpt]) addHop(db, 'conv', hop);
  const hops = listHops(db, 'conv');
  assert.deepEqual(hops.map(hop => hop.factType), ['request_captured', 'file_identified', 'file_excerpt']);
  assert.equal(hops[1].parentId, hops[0].id);
  assert.equal(hops[2].parentId, hops[1].id);
  assert.match(hops[2].source, /@12/);
});

test('only the six fact kinds plus file facts are accepted', () => {
  const db = new DatabaseSync(':memory:');
  assert.throws(() => addHop(db, 'conv', { factType: 'guess', factValue: 'maybe', source: 'note' }), /chain_fact_invalid/);
});
