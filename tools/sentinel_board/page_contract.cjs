// Real Board callers and paging state; only desktop reads/timers are replaced.
const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan, flush } = require('./harness.cjs');
const row = (id = 'safe') => scan(id, 'scanning');
const head = () => Array.from({ length: 300 }, (_, i) => row(`task-${i}`));
const invalidPages = {
  null: null, object: {}, 'null row': [null], 'array row': [[]],
  'blank id': [row(' ')], 'oversized id': [row('a'.repeat(257))],
  'missing timestamp': [{ ...row(), updatedAt: undefined }],
  'invalid status': [{ ...row(), status: null }],
  'duplicate ids': [row(), row()],
  'too many rows': Array.from({ length: 301 }, (_, i) => row(`task-${i}`)),
  'foreign project': [{ ...row(), projectId: 2 }],
  'missing project': [{ ...row(), projectId: undefined }],
  'partially valid page': [row('partial'), { ...row('bad'), projectId: 2 }],
};

async function fixture(t) {
  let reply = head();
  const calls = [], errors = [];
  const m = await mount(undefined, { listSentinelScans: async (...args) => {
    calls.push(args); return reply;
  } }, { props: { projectId: 1 }, onError: error => errors.push(error) });
  t.after(m.unmount);
  m.props.active = true; await flush();
  return { m, calls, errors, respond: value => { reply = value; } };
}

for (const mode of ['head', 'live', 'history']) {
  for (const [label, invalid] of Object.entries(invalidPages)) {
    test(`Board ${mode} rejects ${label} without publishing rows or cursor`, async t => {
      const { m, calls, errors, respond } = await fixture(t);
      const before = m.b.scans.value, cursor = m.b.scanPages.scanOlderCursor.value;
      const count = calls.length;
      respond(invalid);
      if (mode === 'head') await m.b.load();
      else if (mode === 'live') await m.b.liveSync();
      else await m.b.loadMoreScanHistory();
      assert.equal(calls.length, count + 1, 'exercise a real read, not a skipped callback');
      assert.equal(m.b.scans.value, before);
      assert.equal(m.b.scanPages.scanOlderCursor.value, cursor);
      assert.equal(m.b.scanHasMore.value, true);
      assert.equal(m.b.scanLoadingMore.value, false);
      assert.equal(m.b.loading.value, false);
      assert.deepEqual(errors, []);
      assert.deepEqual(m.events, mode === 'live' ? [] : [['error', mode === 'history'
        ? '历史任务读取失败，请重试' : 'Error: 任务列表数据无效，请重试']]);
      // Retry uses the last accepted cursor, never a partially valid row.
      respond([row('recovered')]);
      await m.b.loadMoreScanHistory();
      assert.deepEqual(calls.at(-1)[2], cursor);
      assert.equal(m.b.scans.value.length, 301);
      assert.equal(m.b.scans.value.at(-1).id, 'recovered');
      assert.equal(m.b.scanHasMore.value, false);
    });
  }
}

test('Board global pages accept legacy fields and mixed projects but reject invalid project IDs', async t => {
  const { m, respond } = await fixture(t);
  m.props.projectId = undefined; await flush();
  const legacy = [{ ...row('legacy'), projectId: null, status: '', updatedAt: '' },
    { ...row('future'), projectId: 2, status: 'future_state' },
    { ...row('unassigned'), projectId: undefined }];
  respond(legacy); await m.b.load();
  assert.deepEqual(m.b.scans.value, legacy);
  for (const projectId of [0, -1, 1.5, '1', Number.MAX_SAFE_INTEGER + 1]) {
    respond([{ ...row(), projectId }]); await m.b.load();
    assert.equal(m.b.scans.value, legacy);
  }
  respond([]); await m.b.load();
  assert.deepEqual(m.b.scans.value, []);
  assert.equal(m.b.scanPages.scanOlderCursor.value, undefined);
  assert.equal(m.b.scanHasMore.value, false);
});
