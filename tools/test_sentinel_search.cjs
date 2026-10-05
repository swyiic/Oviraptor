const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan, flush } = require('./sentinel_board/harness.cjs');

async function fixture(t) {
  const requests = [];
  const m = await mount(undefined, { searchSentinelScanIds: query => new Promise((resolve, reject) => {
    requests.push({ query, resolve, reject });
  }) });
  t.after(() => { m.unmount(); requests.forEach(request => request.resolve([])); });
  async function search(query) { m.props.search = query; await flush(); return requests.at(-1); }
  return { m, requests, search };
}

test('older search response cannot overwrite newer matching task ids', async (t) => {
  const { m, search } = await fixture(t);
  const old = await search('first'), current = await search('second');
  current.resolve(['current']); await flush();
  old.resolve(['obsolete']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, ['current']);
  assert.deepEqual(m.events, []);
});

test('clearing search invalidates an outstanding response and does not reselect a task', async (t) => {
  const { m, requests, search } = await fixture(t);
  const old = await search('first');
  await search('  '); old.resolve(['obsolete']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, []);
  assert.equal(requests.length, 1);
  assert.equal(m.b.selected.value, undefined);
});

test('new query clears previous matched ids immediately while awaiting new results', async (t) => {
  const { m, search } = await fixture(t);
  (await search('first')).resolve(['old']); await flush();
  m.props.search = 'second';
  await flush();
  assert.deepEqual(m.b.matchedScanIds.value, []);
});

test('repeated query text cannot revive an earlier generation of the same query', async (t) => {
  const { m, search } = await fixture(t);
  const first = await search('same');
  const middle = await search('other');
  const last = await search('same');
  last.resolve(['new']); await flush();
  first.resolve(['old']); middle.resolve(['other']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, ['new']);
});

test('obsolete search errors are silent and cannot erase current results', async (t) => {
  const { m, search } = await fixture(t);
  const old = await search('first'), current = await search('second');
  current.resolve(['current']); await flush();
  old.reject(new Error('/private/token=secret')); await flush();
  assert.deepEqual(m.events, []);
  assert.deepEqual(m.b.matchedScanIds.value, ['current']);
});

test('current search failure clears old matches and uses a fixed non-sensitive error', async (t) => {
  const { m, search } = await fixture(t);
  (await search('first')).resolve(['old']); await flush();
  (await search('second')).reject(new Error('/private/token=secret')); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, []);
  assert.deepEqual(m.events, [['error', '任务查询失败，请重试']]);
});

test('changing project invalidates old search results and refreshes the current query', async (t) => {
  const { m, requests, search } = await fixture(t);
  const old = await search('first');
  m.props.projectId = 2; await flush();
  old.resolve(['old-project']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, []);
  assert.equal(requests.length, 2);
  requests[1].resolve(['new-project']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, ['new-project']);
  assert.equal(m.b.projectFilter.value, 2);
});

test('returning to an earlier project still rejects its previous query generation', async (t) => {
  const { m, requests, search } = await fixture(t);
  const first = await search('same');
  m.props.projectId = 2; await flush();
  m.props.projectId = undefined; await flush();
  assert.equal(requests.length, 3);
  requests[2].resolve(['current']); await flush();
  first.resolve(['obsolete']); requests[1].resolve(['other-project']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, ['current']);
});

test('late response cannot navigate to an old task matching only the previous query', async (t) => {
  const { m, search } = await fixture(t);
  const old = await search('first'), current = await search('second');
  m.b.scans.value = [scan('obsolete')];
  current.resolve([]); await flush();
  old.resolve(['obsolete']); await flush();
  assert.equal(m.b.selected.value, undefined);
  assert.deepEqual(m.events, []);
});

test('task search is read-only and supports valid empty and populated responses', async (t) => {
  const { m, requests, search } = await fixture(t);
  (await search('first')).resolve(['one', 'two']); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, ['one', 'two']);
  (await search('second')).resolve([]); await flush();
  assert.deepEqual(m.b.matchedScanIds.value, []);
  assert.equal(m.b.tab.value, 'results');
  assert.deepEqual(requests.map(request => request.query), ['first', 'second']);
  assert.deepEqual(m.events, []);
});

for (const [name, response] of [['null', null], ['object', {}], ['mixed ids', ['ok', 2]], ['blank id', [' ']]]) {
  test(`malformed ${name} search response cannot become rendered task ids`, async (t) => {
    const { m, search } = await fixture(t);
    (await search('first')).resolve(response); await flush();
    assert.deepEqual(m.b.matchedScanIds.value, []);
    assert.deepEqual(m.events, [['error', '任务查询失败，请重试']]);
  });
}
