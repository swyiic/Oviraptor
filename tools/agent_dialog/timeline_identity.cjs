// Message identity across cache merging, reading anchors and rendering.
const { assert, test, mount, status, longTimeline, flush, renderDialogTree } = require('../agent_dialog_harness.cjs');

test('delimiter-bearing message identities survive deltas and true identity updates', async (t) => {
  const rows = longTimeline(2);
  rows[0] = { ...rows[0], eventType: 'type:a', id: 'b' };
  rows[1] = { ...rows[1], eventType: 'type', id: 'a:b' };
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'),
    latestSequence: 1, timelineBeforeSequence: 1, timeline: [rows[0]] }) });
  t.after(h.unmount);
  h.api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 2,
    isIncremental: true, timelineBeforeSequence: 2, timeline: [rows[1]] });
  await h.b.loadStatus(1);
  assert.equal(h.b.statusError.value, '');
  assert.deepEqual(h.b.state.value.timeline.map(row => row.sequence), [1, 2]);
  h.api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 3,
    isIncremental: true, timelineBeforeSequence: 3,
    timeline: [{ ...rows[0], sequence: 3, summary: 'updated original' }] });
  await h.b.loadStatus(2);
  assert.deepEqual(h.b.state.value.timeline.map(row => row.sequence), [2, 3]);
  assert.equal(h.b.state.value.timeline[1].summary, 'updated original');
});

test('delimiter-bearing identities cannot redirect the reading anchor', async (t) => {
  const h = await mount(); t.after(h.unmount);
  const rows = longTimeline(250);
  rows[0] = { ...rows[0], eventType: 'type:a', id: 'b' };
  rows[50] = { ...rows[50], eventType: 'type', id: 'a:b' };
  // Populate the local cache, independently of the 100-row transport limit.
  h.b.state.value = { ...status('A'), latestSequence: 250,
    timelineBeforeSequence: 1, timeline: rows };
  await flush();
  h.b.showEarlierMessages();
  assert.equal(h.b.timelinePage.value[0].sequence, 51);
  h.b.state.value = { ...h.b.state.value, timeline: rows.filter(row => row.sequence !== 51) };
  await flush();
  assert.equal(h.b.timelinePage.value[0].sequence, 51, 'evicted page stays locally readable');
  assert.equal(h.b.canMarkPageRead.value, false, 'retained page cannot advance server read state');
});

test('the actual chat template keys distinct event-scoped identities losslessly', async (t) => {
  const rows = longTimeline(4).map((row, index) => ({ ...row, ...[
    { eventType: 'type-a', id: 'b' }, { eventType: 'type', id: 'a-b' },
    { eventType: 'type:a', id: 'b' }, { eventType: 'type', id: 'a:b' },
  ][index] }));
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'),
    latestSequence: 4, timelineBeforeSequence: 1, timeline: rows }) });
  t.after(h.unmount);
  const keys = [];
  function visit(node) {
    if (Array.isArray(node)) return node.forEach(visit);
    if (!node || typeof node !== 'object') return;
    if (typeof node.key === 'string') keys.push(node.key);
    if (Array.isArray(node.children)) visit(node.children);
  }
  visit(renderDialogTree(h.b));
  for (const row of rows) {
    const expected = JSON.stringify([row.eventType, row.id]);
    assert.equal(keys.filter(key => key === expected).length, 1);
  }
});
