const { assert, test, mount, flush, deferred, status, longTimeline } = require('./agent_dialog_harness.cjs');

const recent = () => ({ ...status('A'), latestSequence: 300,
  timeline: longTimeline(300).slice(200), timelineBeforeSequence: 201, hasEarlierTimeline: true });
const history = (before) => ({ scanId: 'A', attemptNumber: 1, beforeSequence: before,
  timelineBeforeSequence: before - 100, hasEarlierTimeline: before > 101,
  timeline: longTimeline(300).slice(before - 101, before - 1) });

for (const added of [150, 200]) {
  test(`anchored local page survives ${added} incoming rows and cache eviction without read writes`, async (t) => {
    let writes = 0;
    const calls = [];
    const snapshot = (count) => ({ ...status('A'), latestSequence: count,
      timeline: longTimeline(count).slice(-300), timelineBeforeSequence: count - 299, hasEarlierTimeline: true });
    const h = await mount({ getNativeScanStatus: async () => ({ ...snapshot(401),
      timeline: longTimeline(401).slice(-100), timelineBeforeSequence: 302 }),
      getNativeScanTimelinePage: async (...args) => { calls.push(args); return history(args[2]); },
      saveAgentDialogView: async () => { writes++; throw new Error('unexpected read write'); },
    }); t.after(h.unmount);
    // Seed the local merged cache explicitly; it is not one backend page.
    h.b.state.value = snapshot(401);
    h.b.showEarlierMessages();
    assert.equal(h.b.timelinePage.value[0].sequence, 202);
    // Two updates exercise partial and complete eviction, not unbounded append.
    h.b.state.value = snapshot(401 + added);
    assert.equal(h.b.timelinePage.value[0].sequence, 202);
    h.b.state.value = snapshot(601 + added);
    await flush();
    assert.deepEqual(h.b.timelinePage.value.map((row) => row.sequence),
      Array.from({ length: 100 }, (_, index) => index + 202));
    assert.equal(h.b.state.value.timeline.length, 300);
    assert.equal(h.b.historyPage.value.timeline.length, 100);
    h.b.dialogView.value = { ...h.b.dialogView.value, allReadSequence: 201 };
    assert.equal(h.b.canMarkPageRead.value, false);
    await h.b.saveDialogView(true);
    assert.equal(writes, 0);
    assert.deepEqual(calls, []);
    h.b.showEarlierMessages(); await flush();
    assert.deepEqual(calls[0], ['A', 1, 202]);
    assert.equal(h.b.timelinePage.value[0].sequence, 102);
    h.b.showLatestMessages();
    assert.equal(h.b.historyPage.value, undefined);
    assert.equal(h.b.timelinePage.value.at(-1).sequence, 601 + added);
  });
}

test('evicted local page is cleared when the attempt changes', async (t) => {
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 401,
    timeline: longTimeline(401).slice(-100), timelineBeforeSequence: 302, hasEarlierTimeline: true }) });
  t.after(h.unmount);
  h.b.state.value = { ...h.b.state.value, timeline: longTimeline(401).slice(-300), timelineBeforeSequence: 102 };
  h.b.showEarlierMessages();
  h.b.state.value = { ...h.b.state.value, latestSequence: 601,
    timeline: longTimeline(601).slice(-300), timelineBeforeSequence: 302 };
  assert.equal(h.b.historyPage.value.timeline.length, 100);
  h.b.state.value = { ...status('A'), attemptNumber: 2, timeline: longTimeline(1),
    latestSequence: 1, timelineBeforeSequence: 1, hasEarlierTimeline: false };
  await flush();
  assert.equal(h.b.historyPage.value, undefined);
  assert.deepEqual(h.b.timelinePage.value.map((row) => row.sequence), [1]);
});

test('cache eviction fences a pending history reply and retains only the page being read', async (t) => {
  const pending = deferred();
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 401,
    timeline: longTimeline(401).slice(-100), timelineBeforeSequence: 302, hasEarlierTimeline: true }),
    getNativeScanTimelinePage: () => pending.promise });
  t.after(h.unmount);
  h.b.state.value = { ...h.b.state.value, timeline: longTimeline(401).slice(-300), timelineBeforeSequence: 102 };
  h.b.showEarlierMessages(); h.b.showEarlierMessages();
  assert.equal(h.b.timelinePage.value[0].sequence, 102);
  h.b.showEarlierMessages();
  assert.equal(h.b.historyBusy.value, true);
  h.b.state.value = { ...h.b.state.value, latestSequence: 601,
    timeline: longTimeline(601).slice(-300), timelineBeforeSequence: 302 };
  assert.equal(h.b.historyBusy.value, false);
  pending.resolve(history(102)); await flush();
  assert.equal(h.b.timelinePage.value[0].sequence, 102);
  assert.equal(h.b.historyRetainedLocalPage.value, true);
  h.b.showLaterMessages();
  assert.equal(h.b.historyRetainedLocalPage.value, false);
  assert.equal(h.b.timelinePage.value[0].sequence, 502);
});

test('bounded history reads backward and forward without caching old page payloads or writing an ack', async (t) => {
  const calls = []; let writes = 0;
  const h = await mount({ getNativeScanStatus: async () => recent(),
    getNativeScanTimelinePage: async (...args) => { calls.push(args); return history(args[2]); },
    saveAgentDialogView: async (input) => { writes++; return { scanId: 'A', attemptNumber: 1,
      revision: input.expectedRevision + 1, selectedThread: input.selectedThread,
      allReadSequence: input.markReadThrough ?? 0, threadReadSequences: {} }; },
  }); t.after(h.unmount);
  assert.equal(h.b.timelinePage.value[0].sequence, 201);
  h.b.showEarlierMessages(); await flush();
  assert.deepEqual(calls[0], ['A', 1, 201]);
  assert.equal(h.b.timelinePage.value[0].sequence, 101);
  assert.equal(h.b.state.value.timeline.length, 100);
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.timelinePage.value[0].sequence, 1);
  h.b.showLaterMessages(); await flush();
  assert.deepEqual(calls.at(-1), ['A', 1, 201]);
  assert.equal(h.b.timelinePage.value[0].sequence, 101);
  h.b.showLaterMessages();
  assert.equal(h.b.timelinePage.value[0].sequence, 201);
  assert.equal(writes, 0);
});

test('prefix reads require the oldest page first and never consume unseen history', async (t) => {
  const writes = [];
  const h = await mount({ getNativeScanStatus: async () => recent(),
    getNativeScanTimelinePage: async (_scan, _attempt, before) => history(before),
    saveAgentDialogView: async (input) => { writes.push(input); return { scanId: 'A', attemptNumber: 1,
      revision: input.expectedRevision + 1, selectedThread: input.selectedThread,
      allReadSequence: input.markReadThrough, threadReadSequences: {} }; },
  }); t.after(h.unmount);
  assert.equal(h.b.canMarkPageRead.value, false);
  await h.b.saveDialogView(true);
  assert.equal(writes.length, 0);
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.visibleUnreadCount.value, 100);
  assert.equal(h.b.canMarkPageRead.value, false);
  await h.b.saveDialogView(true);
  assert.equal(writes.length, 0);
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.timelinePage.value[0].sequence, 1);
  assert.equal(h.b.canMarkPageRead.value, true);
  await h.b.saveDialogView(true);
  assert.equal(writes[0].markReadThrough, 100);
  h.b.showLaterMessages(); await flush();
  assert.equal(h.b.canMarkPageRead.value, true);
  await h.b.saveDialogView(true);
  assert.equal(writes[1].markReadThrough, 200);
  h.b.showLatestMessages();
  assert.equal(h.b.visibleUnreadCount.value, 100);
  assert.equal(h.b.canMarkPageRead.value, true);
  await h.b.saveDialogView(true);
  assert.equal(writes[2].markReadThrough, 300);
});

test('a filtered thread cannot skip its unloaded prefix unless already read', async (t) => {
  const writes = [];
  const h = await mount({ getNativeScanStatus: async () => recent(),
    saveAgentDialogView: async (input) => { writes.push(input); return { scanId: 'A', attemptNumber: 1,
      revision: input.expectedRevision + 1, selectedThread: input.selectedThread,
      allReadSequence: 0, threadReadSequences: { team: input.markReadThrough } }; },
  }); t.after(h.unmount);
  h.b.threadFilter.value = 'team';
  assert.equal(h.b.canMarkPageRead.value, false);
  await h.b.saveDialogView(true);
  assert.equal(writes.length, 0);
  h.b.dialogView.value = { ...h.b.dialogView.value, threadReadSequences: { team: 200 } };
  assert.equal(h.b.canMarkPageRead.value, true);
  await h.b.saveDialogView(true);
  assert.equal(writes[0].markReadThrough, 300);
  assert.equal(writes[0].selectedThread, 'team');
});

test('a history-only thread selection sends a bounded event witness without marking read', async (t) => {
  const writes = [];
  const h = await mount({ getNativeScanStatus: async () => recent(),
    getNativeScanTimelinePage: async (_scan, _attempt, before) => {
      const page = history(before);
      page.timeline[0] = { ...page.timeline[0], threadKey: 'historic-assignment' };
      return page;
    },
    saveAgentDialogView: async (input) => { writes.push(input); return { scanId: 'A', attemptNumber: 1,
      revision: input.expectedRevision + 1, selectedThread: input.selectedThread,
      allReadSequence: 0, threadReadSequences: {} }; },
  }); t.after(h.unmount);
  h.b.showEarlierMessages(); await flush();
  h.b.threadFilter.value = 'historic-assignment';
  await h.b.saveDialogView();
  assert.equal(writes.length, 1);
  assert.equal(writes[0].selectedThreadSequence, 101);
  assert.equal(Object.hasOwn(writes[0], 'markReadThrough'), false);
});

test('timestamp order cannot hide a lower-sequence unread item from a prefix commit', async (t) => {
  const writes = [];
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'),
    latestSequence: 3, timelineBeforeSequence: 1,
    timeline: [1, 3, 2].map((sequence) => ({ ...longTimeline(3)[sequence - 1] })) }),
    saveAgentDialogView: async (input) => { writes.push(input); throw new Error('unexpected write'); },
  }); t.after(h.unmount);
  h.b.timelinePageAnchor.value = h.b.timelineIdentity(longTimeline(1)[0]);
  // The normal 100-row slice contains everything; no omitted event exists.
  assert.equal(h.b.canMarkPageRead.value, true);
  h.b.state.value = { ...h.b.state.value, timeline: longTimeline(102).sort((a, b) =>
    (a.sequence === 2 ? 103 : a.sequence) - (b.sequence === 2 ? 103 : b.sequence)),
    latestSequence: 102, timelineBeforeSequence: 1 };
  h.b.timelinePageAnchor.value = h.b.timelineIdentity(longTimeline(3)[2]);
  assert.equal(h.b.timelinePage.value.some((item) => item.sequence === 2), false);
  assert.equal(h.b.canMarkPageRead.value, false);
  await h.b.saveDialogView(true);
  assert.equal(writes.length, 0);
});

test('late and malformed history responses cannot cross attempts or replace the current chat', async (t) => {
  const old = deferred();
  const h = await mount({ getNativeScanStatus: async () => recent(),
    getNativeScanTimelinePage: () => old.promise }); t.after(h.unmount);
  h.b.showEarlierMessages();
  h.b.state.value = { ...recent(), attemptNumber: 2 };
  old.resolve(history(201)); await flush();
  assert.equal(h.b.historyPage.value, undefined);
  assert.equal(h.b.timelinePage.value[0].sequence, 201);
  h.api.getNativeScanTimelinePage = async () => ({ ...history(201), attemptNumber: 1 });
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.historyPage.value, undefined);
  assert.equal(h.b.historyError.value, '历史消息读取失败，请重试。');
});

const invalidHistoryPages = {
  'duplicate message identity': (page) => { page.timeline[1].id = page.timeline[0].id; },
  'duplicate sequence': (page) => { page.timeline[1].sequence = page.timeline[0].sequence; },
  'empty message identity': (page) => { page.timeline[0].id = ''; },
  'empty event type': (page) => { page.timeline[0].eventType = ''; },
  'object timestamp': (page) => { page.timeline[0].timestamp = {}; },
  'object summary': (page) => { page.timeline[0].summary = { secret: 'not-displayable' }; },
  'object thread key': (page) => { page.timeline[0].threadKey = {}; },
  'non-array reasons': (page) => { page.timeline[0].reasonCodes = 'proposal_invalid'; },
  'non-string reason': (page) => { page.timeline[0].reasonCodes = [42]; },
  'zero cursor with visible messages': (page) => {
    page.timelineBeforeSequence = 0; page.hasEarlierTimeline = false;
  },
};

for (const [label, corrupt] of Object.entries(invalidHistoryPages)) {
  test(`history rejects ${label} without replacing a verified page or writing read state`, async (t) => {
    let writes = 0;
    const h = await mount({ getNativeScanStatus: async () => recent(),
      getNativeScanTimelinePage: async (_scan, _attempt, before) => history(before),
      saveAgentDialogView: async () => { writes++; throw new Error('unexpected write'); },
    }); t.after(h.unmount);
    h.b.showEarlierMessages(); await flush();
    assert.equal(h.b.timelinePage.value[0].sequence, 101);
    const verified = h.b.historyPage.value;
    h.api.getNativeScanTimelinePage = async (_scan, _attempt, before) => {
      const page = history(before); corrupt(page); return page;
    };
    h.b.showEarlierMessages(); await flush();
    assert.equal(h.b.historyError.value, '历史消息读取失败，请重试。');
    assert.equal(h.b.historyPage.value, verified);
    assert.equal(h.b.timelinePage.value[0].sequence, 101);
    assert.equal(h.b.canMarkPageRead.value, false);
    await h.b.saveDialogView(true);
    assert.equal(writes, 0);
    h.api.getNativeScanTimelinePage = async (_scan, _attempt, before) => history(before);
    h.b.showEarlierMessages(); await flush();
    assert.equal(h.b.timelinePage.value[0].sequence, 1, 'retry uses the unchanged cursor stack');
    assert.equal(h.b.historyError.value, '');
  });
}

test('superseded history rows may leave sparse or empty pages with a valid earlier cursor', async (t) => {
  const calls = [];
  const h = await mount({ getNativeScanStatus: async () => recent(),
    getNativeScanTimelinePage: async (_scan, _attempt, before) => {
      calls.push(before);
      return { ...history(before), timeline: before === 201 ? [] : [longTimeline(100)[49]] };
    },
  }); t.after(h.unmount);
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.historyError.value, '');
  assert.equal(h.b.timelinePage.value.length, 0);
  assert.equal(h.b.canMarkPageRead.value, false);
  h.b.showEarlierMessages(); await flush();
  assert.deepEqual(calls, [201, 101]);
  assert.equal(h.b.historyError.value, '');
  assert.equal(h.b.timelinePage.value[0].sequence, 50);
});

test('history preserves timestamp order and event-scoped identity, including an empty oldest page', async (t) => {
  const page = history(201);
  page.timeline = [150, 120].map((sequence, index) => ({ ...longTimeline(200)[sequence - 1],
    id: 'same-entity', eventType: index ? 'review_gate' : 'mailbox_message' }));
  const h = await mount({ getNativeScanStatus: async () => recent(),
    getNativeScanTimelinePage: async (_scan, _attempt, before) => before === 201 ? page
      : { ...history(before), timelineBeforeSequence: 0, hasEarlierTimeline: false, timeline: [] },
  }); t.after(h.unmount);
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.historyError.value, '');
  assert.deepEqual(h.b.timelinePage.value.map((item) => item.sequence), [150, 120]);
  h.b.showEarlierMessages(); await flush();
  assert.equal(h.b.historyError.value, '');
  assert.equal(h.b.timelinePage.value.length, 0);
  assert.equal(h.b.canMarkPageRead.value, false);
  h.b.showLaterMessages(); await flush();
  assert.deepEqual(h.b.timelinePage.value.map((item) => item.sequence), [150, 120]);
});
