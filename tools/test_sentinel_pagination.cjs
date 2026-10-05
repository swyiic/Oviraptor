const assert = require('node:assert/strict');
const test = require('node:test');
require('./sentinel_board/page_contract.cjs');
const { mount, scan, flush } = require('./sentinel_board/harness.cjs');

const row = (id, projectId = 1, updatedAt = '2026-09-29 10:00:00') => ({ ...scan(id), projectId, updatedAt });
const head = (projectId = 1) => Array.from({ length: 300 }, (_, i) => row(`p${projectId}-${String(999 - i)}`, projectId));
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
async function fixture(t) {
  const requests = [], heads = [];
  let nextHead;
  const m = await mount(undefined, { listSentinelScans: (project, limit, cursor) => {
    if (!cursor) {
      heads.push(project);
      const response = nextHead; nextHead = undefined;
      return response ? response.promise : Promise.resolve(head(project));
    }
    const request = { project, limit, cursor, ...deferred() };
    requests.push(request); return request.promise;
  } });
  t.after(() => { m.unmount(); requests.forEach(request => request.resolve([])); });
  return { m, requests, heads, delayHead() { nextHead = deferred(); return nextHead; } };
}

test('history pagination keeps its cursor, prevents duplicate requests and finishes on a short page', async (t) => {
  const { m, requests } = await fixture(t);
  assert.equal(m.b.scans.value.length, 300); assert.equal(m.b.scanHasMore.value, true);
  const work = m.b.loadMoreScanHistory(); await m.b.loadMoreScanHistory();
  assert.equal(requests.length, 1); assert.equal(requests[0].limit, 300);
  assert.equal(requests[0].cursor.id, 'p1-700');
  requests[0].resolve([row('older', 1, '2026-09-28 00:00:00')]); await work;
  assert.equal(m.b.scans.value.length, 301); assert.equal(m.b.scanHasMore.value, false);
  assert.equal(m.b.scanLoadingMore.value, false);
  await m.b.loadMoreScanHistory(); assert.equal(requests.length, 1);
});

test('history page success after unmount cannot publish rows or cursor state', async (t) => {
  const { m, requests } = await fixture(t);
  const work = m.b.loadMoreScanHistory(); const before = m.b.scans.value;
  const cursor = m.b.scanPages.scanOlderCursor.value;
  m.unmount(); requests[0].resolve([row('late')]); await work;
  assert.equal(m.b.scans.value, before); assert.equal(m.b.scanPages.scanOlderCursor.value, cursor);
  assert.deepEqual(m.events, []);
});

test('unmounted history failure stays silent and a stale callback cannot start another read', async (t) => {
  const { m, requests } = await fixture(t);
  const work = m.b.loadMoreScanHistory(); m.unmount();
  requests[0].reject(new Error('/private/token=secret')); await work;
  assert.deepEqual(m.events, []);
  const retry = m.b.loadMoreScanHistory();
  assert.equal(requests.length, 1); await retry;
});

test('project switch releases old paging busy state before the old request settles', async (t) => {
  const { m, requests } = await fixture(t);
  const old = m.b.loadMoreScanHistory();
  m.props.projectId = 2; await flush();
  assert.equal(m.b.scanLoadingMore.value, false);
  const current = m.b.loadMoreScanHistory();
  assert.equal(requests.length, 2); assert.equal(requests[1].project, 2);
  requests[0].resolve([row('wrong-project')]); await old;
  assert.equal(m.b.scanLoadingMore.value, true);
  assert.ok(m.b.scans.value.every(item => item.projectId === 2));
  requests[1].resolve([row('current', 2, '2026-09-28 00:00:00')]); await current;
  assert.equal(m.b.scanLoadingMore.value, false);
  assert.equal(m.b.scans.value.length, 301);
});

test('old-project pagination failure cannot notify in the new project', async (t) => {
  const { m, requests } = await fixture(t);
  const old = m.b.loadMoreScanHistory(); m.props.projectId = 2; await flush();
  requests[0].reject(new Error('/private/token=secret')); await old;
  assert.deepEqual(m.events, []);
});

test('head reload invalidates pending history before the refreshed head arrives', async (t) => {
  const { m, requests, delayHead } = await fixture(t);
  const old = m.b.loadMoreScanHistory(), pendingHead = delayHead();
  const reload = m.b.load();
  requests[0].resolve([row('obsolete')]); await old;
  assert.equal(m.b.scans.value.some(item => item.id === 'obsolete'), false);
  const retry = m.b.loadMoreScanHistory(); assert.equal(requests.length, 1); await retry;
  pendingHead.resolve(head()); await reload;
});

test('new head can paginate while invalidated history is still in flight', async (t) => {
  const { m, requests } = await fixture(t);
  const old = m.b.loadMoreScanHistory(); await m.b.load();
  assert.equal(m.b.scanLoadingMore.value, false);
  const current = m.b.loadMoreScanHistory(); assert.equal(requests.length, 2);
  requests[0].reject(new Error('old-private-error')); await old;
  assert.equal(m.b.scanLoadingMore.value, true); assert.deepEqual(m.events, []);
  requests[1].resolve([]); await current;
  assert.equal(m.b.scanLoadingMore.value, false); assert.equal(m.b.scanHasMore.value, false);
});

test('current pagination failure preserves existing rows and cursor and allows retry', async (t) => {
  const { m, requests } = await fixture(t);
  const before = m.b.scans.value, cursor = m.b.scanPages.scanOlderCursor.value;
  const work = m.b.loadMoreScanHistory(); requests[0].reject(new Error('/private/token=secret')); await work;
  assert.equal(m.b.scans.value, before); assert.equal(m.b.scanPages.scanOlderCursor.value, cursor);
  assert.equal(m.b.scanHasMore.value, true); assert.equal(m.b.scanLoadingMore.value, false);
  assert.deepEqual(m.events, [['error', '历史任务读取失败，请重试']]);
  const retry = m.b.loadMoreScanHistory(); assert.equal(requests.length, 2);
  requests[1].resolve([]); await retry;
  assert.equal(m.b.scanHasMore.value, false);
});

test('older history page cannot roll back a newer row already merged from live DB data', async (t) => {
  const { m, requests } = await fixture(t);
  const work = m.b.loadMoreScanHistory();
  m.b.mergeScanPage([{ ...row('updated', 1, '2026-09-29 11:00:00'), status: 'completed' }]);
  requests[0].resolve([{ ...row('updated', 1, '2026-09-28 00:00:00'), status: 'scanning' }]); await work;
  const actual = m.b.scans.value.filter(item => item.id === 'updated');
  assert.equal(actual.length, 1); assert.equal(actual[0].status, 'completed');
  assert.equal(actual[0].updatedAt, '2026-09-29 11:00:00');
});

test('same-timestamp history cannot overwrite live state but a current live merge can update it', async (t) => {
  const { m, requests } = await fixture(t);
  const work = m.b.loadMoreScanHistory();
  const original = row('same-time', 1, '2026-09-28 00:00:00');
  m.b.mergeScanPage([{ ...original, status: 'completed' }]);
  requests[0].resolve([{ ...original, status: 'scanning' }]); await work;
  assert.equal(m.b.scans.value.find(item => item.id === original.id).status, 'completed');
  m.b.mergeScanPage([{ ...original, status: 'completed', archivedAt: '2026-09-29 11:00:00' }]);
  assert.equal(m.b.scans.value.find(item => item.id === original.id).archivedAt, '2026-09-29 11:00:00');
});

test('full older page advances its historical cursor independently of new live rows', async (t) => {
  const { m, requests } = await fixture(t);
  const page = Array.from({ length: 300 }, (_, i) => row(`older-${999 - i}`, 1, '2026-09-28 00:00:00'));
  const work = m.b.loadMoreScanHistory(); requests[0].resolve(page); await work;
  assert.equal(m.b.scans.value.length, 600); assert.equal(m.b.scanHasMore.value, true);
  m.b.mergeScanPage([row('newest', 1, '2026-09-29 11:00:00')]);
  const next = m.b.loadMoreScanHistory();
  assert.deepEqual(requests[1].cursor, { id: 'older-700', updatedAt: '2026-09-28 00:00:00' });
  requests[1].resolve([]); await next;
  assert.equal(m.b.scans.value.length, 601); assert.equal(m.b.scanHasMore.value, false);
});

test('project A to B to A does not revive the old A history page', async (t) => {
  const { m, requests } = await fixture(t);
  const old = m.b.loadMoreScanHistory();
  m.props.projectId = 2; await flush(); m.props.projectId = undefined; await flush();
  const current = m.b.loadMoreScanHistory(); assert.equal(requests.length, 2);
  requests[0].resolve([row('obsolete')]); await old;
  assert.equal(m.b.scanLoadingMore.value, true);
  assert.equal(m.b.scans.value.some(item => item.id === 'obsolete'), false);
  requests[1].resolve([]); await current;
  assert.equal(m.b.scanLoadingMore.value, false);
});

test('failed head refresh leaves the previous history cursor retryable without accepting its old request', async (t) => {
  const { m, requests, delayHead } = await fixture(t);
  const old = m.b.loadMoreScanHistory(), headRead = delayHead();
  const reload = m.b.load(); headRead.reject(new Error('head unavailable')); await reload;
  const retry = m.b.loadMoreScanHistory(); assert.equal(requests.length, 2);
  assert.deepEqual(requests[1].cursor, requests[0].cursor);
  requests[0].resolve([row('obsolete')]); await old;
  assert.equal(m.b.scanLoadingMore.value, true);
  assert.equal(m.b.scans.value.some(item => item.id === 'obsolete'), false);
  requests[1].resolve([]); await retry;
});
