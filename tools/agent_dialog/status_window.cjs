// Status-window contract regressions, loaded by the existing status suite.
const { assert, test, mount, status, longTimeline, flush, eventRefreshClock } = require('../agent_dialog_harness.cjs');

const initial = () => ({ ...status('A'), latestSequence: 2,
  timelineBeforeSequence: 1, timeline: longTimeline(2) });
const next = (incremental) => ({ ...status('A'), latestSequence: 4, isIncremental: incremental,
  timelineBeforeSequence: 3, timeline: longTimeline(4).slice(2) });

const invalidWindows = {
  'row beyond watermark': page => ({ ...page, timeline: longTimeline(5).slice(3) }),
  'row below window start': page => ({ ...page, timelineBeforeSequence: 4 }),
  'nonempty window at zero': page => ({ ...page, timelineBeforeSequence: 0 }),
  'more than 100 projected rows': page => ({ ...page, latestSequence: 103,
    timeline: longTimeline(103).slice(2) }),
};
for (const incremental of [false, true]) {
  for (const [label, corrupt] of Object.entries(invalidWindows)) {
    test(`${incremental ? 'delta' : 'full'} window rejects ${label} without advancing reading or cache`, async (t) => {
      let writes = 0;
      const clock = eventRefreshClock();
      const h = await mount({ getNativeScanStatus: async () => initial(),
        saveAgentDialogView: async () => { writes++; throw Error('unexpected read write'); },
      }, { ...clock, listen: async () => () => {} }); t.after(h.unmount);
      const previous = h.b.state.value;
      const reading = h.b.dialogView.value;
      h.api.getNativeScanStatus = async () => corrupt(next(incremental));
      await h.b.loadStatus(incremental ? 2 : undefined); await flush();
      assert.equal(h.b.statusError.value, '任务状态读取失败，请重试。');
      assert.equal(h.b.state.value, previous);
      assert.equal(h.b.statusSync.latestSequence.value, 2);
      assert.equal(h.b.dialogView.value, reading);
      assert.deepEqual(h.b.timelinePage.value.map(row => row.sequence), [1, 2]);
      assert.equal(writes, 0);
      assert.ok([...clock.timers.values()].some(timer => timer.delay === 3000));
      h.api.getNativeScanStatus = async () => next(incremental);
      await h.b.loadStatus(incremental ? 2 : undefined);
      assert.equal(h.b.statusError.value, '');
      assert.equal(h.b.statusSync.latestSequence.value, 4);
      assert.deepEqual(h.b.timelinePage.value.map(row => row.sequence), incremental ? [1, 2, 3, 4] : [3, 4]);
    });
  }
}

for (const sequence of [1, 2]) {
  test(`delta cannot replay sequence ${sequence} at or before its requested cursor, even without optional window metadata`, async (t) => {
    const h = await mount({ getNativeScanStatus: async () => initial() }); t.after(h.unmount);
    const previous = h.b.state.value;
    h.api.getNativeScanStatus = async () => ({ ...next(true), timelineBeforeSequence: undefined,
      timeline: [{ ...longTimeline(2)[sequence - 1], summary: 'replayed entity' }] });
    await h.b.loadStatus(2);
    assert.equal(h.b.statusError.value, '任务状态读取失败，请重试。');
    assert.equal(h.b.state.value, previous);
    assert.equal(h.b.statusSync.latestSequence.value, 2);
  });
}

test('an empty delta cannot move its window start behind the request cursor', async (t) => {
  const h = await mount({ getNativeScanStatus: async () => initial() }); t.after(h.unmount);
  const previous = h.b.state.value;
  h.api.getNativeScanStatus = async () => ({ ...next(true), timelineBeforeSequence: 2, timeline: [] });
  await h.b.loadStatus(2);
  assert.equal(h.b.statusError.value, '任务状态读取失败，请重试。');
  assert.equal(h.b.state.value, previous);
});

for (const incremental of [false, true]) {
  for (const empty of [false, true]) {
    test(`${incremental ? 'delta' : 'full'} window accepts ${empty ? 'empty' : 'sparse timestamp-ordered'} projections`, async (t) => {
      const h = await mount({ getNativeScanStatus: async () => initial() }); t.after(h.unmount);
      h.api.getNativeScanStatus = async () => ({ ...next(incremental), latestSequence: 10,
        hasEarlierTimeline: true, timeline: empty ? [] : [
          { ...longTimeline(8)[7], timestamp: 'a' }, { ...longTimeline(5)[4], timestamp: 'b' },
        ] });
      await h.b.loadStatus(incremental ? 2 : undefined);
      assert.equal(h.b.statusError.value, '');
      assert.equal(h.b.statusSync.latestSequence.value, 10);
      assert.deepEqual(h.b.state.value.timeline.filter(row => row.sequence > 2).map(row => row.sequence), empty ? [] : [8, 5]);
      assert.equal(h.b.state.value.timelineBeforeSequence, incremental ? 1 : 3);
    });
  }
}

test('attempt reset uses the full response bounds, not the old delta cursor', async (t) => {
  const h = await mount({ getNativeScanStatus: async () => initial() }); t.after(h.unmount);
  h.api.getNativeScanStatus = async () => ({ ...status('A'), attemptNumber: 2, latestSequence: 1,
    timelineBeforeSequence: 1, timeline: longTimeline(1) });
  await h.b.loadStatus(2);
  assert.equal(h.b.statusError.value, '');
  assert.equal(h.b.state.value.attemptNumber, 2);
  assert.equal(h.b.statusSync.latestSequence.value, 1);
  assert.deepEqual(h.b.state.value.timeline.map(row => row.sequence), [1]);
});

test('valid 100-row deltas accumulate only the latest 300 rows and an empty poll preserves them', async (t) => {
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 100,
    timelineBeforeSequence: 1, timeline: longTimeline(100) }) }); t.after(h.unmount);
  for (const latest of [200, 300, 400]) {
    h.api.getNativeScanStatus = async () => ({ ...status('A'), isIncremental: true, latestSequence: latest,
      timelineBeforeSequence: latest - 99, hasEarlierTimeline: true, timeline: longTimeline(latest).slice(-100) });
    await h.b.loadStatus(latest - 100);
    assert.equal(h.b.statusError.value, '');
    assert.equal(h.b.state.value.timeline.length, Math.min(latest, 300));
  }
  assert.equal(h.b.state.value.timelineBeforeSequence, 101);
  assert.equal(h.b.state.value.hasEarlierTimeline, true);
  h.api.getNativeScanStatus = async () => ({ ...status('A'), isIncremental: true, latestSequence: 400 });
  await h.b.loadStatus(400);
  assert.equal(h.b.statusError.value, '');
  assert.equal(h.b.state.value.timeline.length, 300);
  assert.equal(h.b.state.value.timelineBeforeSequence, 101);
});

test('an invalid initial window cannot restore reading state before a corrected retry', async (t) => {
  let reads = 0;
  const h = await mount({ getNativeScanStatus: async () => ({ ...initial(), latestSequence: 1 }),
    getAgentDialogView: async (scanId, attemptNumber) => {
      reads++; return { scanId, attemptNumber, revision: 0, selectedThread: '', allReadSequence: 0, threadReadSequences: {} };
    },
  }); t.after(h.unmount);
  assert.equal(h.b.state.value, undefined);
  assert.equal(h.b.statusSync.latestSequence.value, 0);
  assert.equal(reads, 0);
  h.api.getNativeScanStatus = async () => initial();
  await h.b.loadStatus(); await flush();
  assert.equal(h.b.statusError.value, '');
  assert.equal(reads, 1);
});

for (const incremental of [false, true]) {
  test(`${incremental ? 'delta' : 'full'} compatible window may omit the optional start cursor`, async (t) => {
    const h = await mount({ getNativeScanStatus: async () => initial() }); t.after(h.unmount);
    h.api.getNativeScanStatus = async () => ({ ...next(incremental), timelineBeforeSequence: undefined });
    await h.b.loadStatus(incremental ? 2 : undefined);
    assert.equal(h.b.statusError.value, '');
    assert.equal(h.b.statusSync.latestSequence.value, 4);
  });
}
