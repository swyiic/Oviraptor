// Exercise the task-center SFC, including retry state and the rendered list.
const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, flush, deferred, scan, page, searchClock } = require('./harness.cjs');
const row = (id = 'safe') => ({ ...scan(id), status: 'draft', projectId: 1 });
const invalidPages = {
  null: null, object: {}, 'null row': [null], 'array row': [[]],
  'blank id': [row(' ')], 'oversized id': [row('a'.repeat(257))],
  'missing timestamp': [{ ...row(), updatedAt: undefined }],
  'invalid status': [{ ...row(), status: null }],
  'duplicate ids': [row(), row()],
  'too many rows': Array.from({ length: 101 }, (_, i) => row(`task-${i}`)),
  'foreign project': [{ ...row(), projectId: 2 }],
  'missing project': [{ ...row(), projectId: undefined }],
  'partially valid page': [row(), { ...row('bad'), projectId: 2 }],
};
for (const [label, invalid] of Object.entries(invalidPages)) {
  test(`task-center search rejects ${label} atomically`, async t => {
    const clock = searchClock(), calls = [];
    const h = await mount(async () => page(), { searchSentinelScanPage: async (...args) => {
      calls.push(args); return invalid;
    } }, clock); t.after(h.unmount);
    h.props.projectId = 1; h.b.search.value = 'task'; await flush();
    clock.fire(); await flush();
    assert.equal(calls.length, 1);
    assert.deepEqual(h.b.searchResults.value, []);
    assert.equal(h.b.searchCursor.value, undefined);
    assert.equal(h.b.searchHasMore.value, false);
    assert.equal(h.b.searchLoading.value, false);
    assert.equal(h.b.searchError.value, '全库搜索失败，请重试');
    const html = await h.render();
    assert.ok(html.includes('全库搜索失败，请重试'));
    assert.ok(!html.includes('全库没有匹配的任务'), 'an invalid page is not a valid empty search');
  });
}

test('invalid older search page preserves rows and retry cursor, then recovers without partial publication', async t => {
  const clock = searchClock(), calls = [];
  const rows = Array.from({ length: 100 }, (_, i) => row(`task-${i}`));
  const replies = [rows, [row('partial'), { ...row('bad'), projectId: 2 }], [row('older')]];
  const h = await mount(async () => page(), { searchSentinelScanPage: async (...args) => {
    calls.push(args); return replies.shift();
  } }, clock); t.after(h.unmount);
  h.props.projectId = 1; h.b.search.value = 'task'; await flush(); clock.fire(); await flush();
  const cursor = { ...h.b.searchCursor.value };
  await h.b.loadSearchPage(true);
  assert.deepEqual(h.b.searchResults.value, rows);
  assert.deepEqual(h.b.searchCursor.value, cursor);
  assert.equal(h.b.searchHasMore.value, true);
  assert.equal(h.b.searchError.value, '全库搜索失败，请重试');
  await h.b.loadSearchPage(true);
  assert.deepEqual(calls[1][4], cursor); assert.deepEqual(calls[2][4], cursor);
  assert.equal(h.b.searchResults.value.length, 101);
  assert.equal(h.b.searchResults.value.at(-1).id, 'older');
  assert.equal(h.b.searchHasMore.value, false); assert.equal(h.b.searchError.value, '');
});

test('same-tick query round trip invalidates the original search generation', async t => {
  const clock = searchClock(), old = deferred();
  const h = await mount(async () => page(), { searchSentinelScanPage: () => old.promise }, clock);
  t.after(h.unmount);
  h.b.search.value = 'original'; await flush(); clock.fire();
  h.b.search.value = 'temporary'; h.b.search.value = 'original';
  old.resolve([row('obsolete')]); await flush();
  assert.deepEqual(h.b.searchResults.value, []);
  assert.equal(clock.size, 1, 'the current generation retains one debounced request');
});

test('global search preserves legacy empty fields and mixed project rows but rejects invalid project IDs', async t => {
  const clock = searchClock();
  let response = [{ ...row('legacy'), projectId: null, updatedAt: '', status: '' },
    { ...row('another'), projectId: 2, status: 'future_state' }];
  const h = await mount(async () => page(), { searchSentinelScanPage: async () => response }, clock);
  t.after(h.unmount); h.b.search.value = 'legacy'; await flush(); clock.fire(); await flush();
  assert.deepEqual(h.b.searchResults.value, response); assert.equal(h.b.searchError.value, '');
  response = [{ ...row(), projectId: 1.5 }];
  h.b.search.value = 'invalid project'; await flush(); clock.fire(); await flush();
  assert.deepEqual(h.b.searchResults.value, []);
  assert.equal(h.b.searchError.value, '全库搜索失败，请重试');
});
