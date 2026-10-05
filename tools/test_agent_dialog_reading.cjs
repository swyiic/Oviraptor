// Real AgentDialog setup/render regressions: reading.
const { assert, test, renderDialog, deferred, flush, status, longTimeline,
  mount, readingView, readingStatus } = require('./agent_dialog_harness.cjs');

test('reading state restores selected thread and unread without writing on load or polling', async (t) => {
  let writes = 0; let reads = 0;
  const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
    getAgentDialogView: async (id, attempt) => { reads++; return readingView(id, attempt,
      { revision: 1, selectedThread: 'team', threadReadSequences: { team: 1 } }); },
    saveAgentDialogView: async () => { writes++; throw new Error('unexpected write'); },
  });
  t.after(h.unmount);
  assert.equal(h.b.threadFilter.value, 'team');
  assert.equal(h.b.visibleUnreadCount.value, 1);
  await h.b.loadStatus(); await flush();
  assert.equal(reads, 1); assert.equal(writes, 0);
  const html = await renderDialog(h.b);
  assert.ok(html.includes('标记当前页消息为已读'));
});

test('read acknowledgements commit before counts change and preserve later messages', async (t) => {
  const pending = deferred(); const calls = [];
  const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
    saveAgentDialogView: (input) => { calls.push(input); return pending.promise; },
  }); t.after(h.unmount);
  h.b.threadFilter.value = 'team';
  const operation = h.b.saveDialogView(true);
  await h.b.saveDialogView(true);
  assert.equal(calls.length, 1); assert.equal(h.b.visibleUnreadCount.value, 2);
  assert.deepEqual(calls[0], { scanId: 'A', attemptNumber: 1, expectedRevision: 0,
    selectedThread: 'team', markReadThrough: 3 });
  h.b.state.value = { ...readingStatus(), latestSequence: 4, timeline: [
    ...readingStatus().timeline, { id: 'four', threadKey: 'team', sequence: 4 },
  ] };
  pending.resolve(readingView('A', 1, { revision: 1, selectedThread: 'team', threadReadSequences: { team: 3 } }));
  await operation;
  assert.equal(h.b.visibleUnreadCount.value, 1);
  h.b.threadFilter.value = 'assignment-A';
  assert.equal(h.b.visibleUnreadCount.value, 1);
});

test('reading save conflict keeps prior cursors and requires explicit reload without auto retry', async (t) => {
  let writes = 0; let revision = 0;
  const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
    getAgentDialogView: async (id, attempt) => readingView(id, attempt, { revision }),
    saveAgentDialogView: async () => { writes++; throw new Error('dialog_view_revision_conflict'); },
  }); t.after(h.unmount);
  await h.b.saveDialogView(true);
  assert.equal(h.b.visibleUnreadCount.value, 3);
  assert.equal(h.b.dialogViewError.value, '阅读偏好保存失败，请重新读取后再操作。');
  assert.equal(h.b.error.value, '');
  await h.b.saveDialogView(true); assert.equal(writes, 1);
  revision = 2; await h.b.loadDialogView();
  assert.equal(h.b.dialogView.value.revision, 2);
  assert.equal(h.b.dialogViewError.value, '');
});

test('stale reading load and save callbacks cannot cross task project attempt or snapshot boundaries', async (t) => {
  for (const action of ['load', 'save']) for (const change of ['task', 'project', 'attempt', 'snapshot']) for (const failure of [false, true]) {
    const pending = deferred();
    const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id) });
    t.after(h.unmount);
    const apiName = action === 'load' ? 'getAgentDialogView' : 'saveAgentDialogView';
    const original = h.api[apiName]; h.api[apiName] = () => pending.promise;
    const operation = action === 'load' ? h.b.loadDialogView() : h.b.saveDialogView(true);
    h.api[apiName] = original;
    if (change === 'task') { h.b.scanId.value = 'B'; await flush(); h.b.scanId.value = 'A'; }
    else if (change === 'project') h.rootProps.projectId = 2;
    else if (change === 'attempt') h.b.state.value = readingStatus('A', 2);
    else { h.b.state.value = undefined; h.b.state.value = readingStatus(); }
    await flush();
    const before = JSON.stringify(h.b.dialogView.value);
    if (failure) pending.reject(new Error('old-window'));
    else pending.resolve(readingView('A', 1, { revision: 99, selectedThread: 'obsolete' }));
    await operation;
    assert.equal(JSON.stringify(h.b.dialogView.value), before, `${action}/${change}/${failure}`);
    assert.equal(h.b.dialogViewError.value, '');
    assert.equal(h.b.dialogViewBusy.value, false);
  }
});

test('unavailable saved threads remain visible without destructive reset', async (t) => {
  const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
    getAgentDialogView: async (id, attempt) => readingView(id, attempt, { revision: 1, selectedThread: 'old-thread' }),
  }); t.after(h.unmount);
  assert.equal(h.b.threadFilter.value, 'old-thread');
  assert.equal(h.b.selectedThreadUnavailable.value, true);
  assert.equal(h.b.visibleTimeline.value.length, 0);
  assert.ok((await renderDialog(h.b)).includes('当前没有可显示消息'));
});

test('old reading callbacks cannot unlock a newer request and unmount suppresses late failures', async (t) => {
  const old = deferred(); const fresh = deferred();
  const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id) });
  t.after(h.unmount);
  h.api.saveAgentDialogView = () => old.promise;
  const operation = h.b.saveDialogView(true);
  h.api.getAgentDialogView = () => fresh.promise;
  h.b.state.value = readingStatus('A', 2);
  assert.equal(h.b.dialogViewBusy.value, true);
  old.reject(new Error('obsolete-save')); await operation;
  assert.equal(h.b.dialogViewBusy.value, true);
  assert.equal(h.b.dialogViewError.value, '');
  h.unmount(); fresh.reject(new Error('closed-window')); await flush();
  assert.equal(h.b.dialogViewError.value, '');
});

test('selection save does not silently mark messages read', async (t) => {
  const calls = [];
  const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
    saveAgentDialogView: async (input) => { calls.push(input); return readingView(input.scanId, input.attemptNumber,
      { revision: 1, selectedThread: input.selectedThread }); },
  }); t.after(h.unmount);
  h.b.threadFilter.value = 'team'; await h.b.saveDialogView();
  assert.equal(Object.hasOwn(calls[0], 'markReadThrough'), false);
  assert.equal(h.b.visibleUnreadCount.value, 2);
  assert.equal(h.b.dialogView.value.selectedThread, 'team');
});

test('malformed reading responses never imply read and cannot hide task timeline', async (t) => {
  for (const extra of [{ scanId: 'B' }, { revision: Number.MAX_SAFE_INTEGER + 1 },
    { allReadSequence: 4 }, { threadReadSequences: { team: -1 } }, { threadReadSequences: [] }]) {
    const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
      getAgentDialogView: async () => readingView('A', 1, extra),
    }); t.after(h.unmount);
    assert.equal(h.b.dialogView.value, undefined);
    assert.equal(h.b.visibleUnreadCount.value, undefined);
    assert.equal(h.b.visibleTimeline.value.length, 3);
    assert.equal(h.b.dialogViewError.value, '阅读偏好读取失败，请重新读取。');
  }
});

for (const [name, markRead, changes] of [
  ['selection advances the global cursor', false, { allReadSequence: 1 }],
  ['selection drops saved thread cursors', false, { threadReadSequences: {} }],
  ['selection invents another read thread', false, { threadReadSequences: { team: 1, 'assignment-A': 2, extra: 3 } }],
  ['read receipt does not acknowledge the requested cursor', true, {}],
  ['thread read also marks other threads globally', true, { allReadSequence: 3, threadReadSequences: { team: 3, 'assignment-A': 2 } }],
  ['thread read drops another saved cursor', true, { threadReadSequences: { team: 3 } }],
]) {
  test(`reading save rejects inconsistent receipt: ${name}`, async (t) => {
    const original = readingView('A', 1, { revision: 4, selectedThread: 'assignment-A',
      threadReadSequences: { team: 1, 'assignment-A': 2 } });
    let writes = 0;
    const h = await mount({ getNativeScanStatus: async (id) => readingStatus(id),
      getAgentDialogView: async () => original,
      saveAgentDialogView: async () => { writes++; return { ...original, revision: 5, selectedThread: 'team', ...changes }; },
    }); t.after(h.unmount);
    h.b.threadFilter.value = 'team';
    assert.equal(h.b.visibleUnreadCount.value, 1);
    await h.b.saveDialogView(markRead);
    assert.equal(h.b.dialogViewError.value, '阅读偏好保存失败，请重新读取后再操作。');
    assert.deepEqual(h.b.dialogView.value, original, 'keep the last verified reading snapshot');
    assert.equal(h.b.visibleUnreadCount.value, 1);
    assert.equal(h.b.dialogViewBusy.value, false);
    assert.equal(h.b.state.value.timeline.length, 3);
    await h.b.saveDialogView(markRead);
    assert.equal(writes, 1, 'an ambiguous receipt requires reload, never automatic replay');
    assert.match(await renderDialog(h.b), /阅读偏好保存失败，请重新读取后再操作。/);
    await h.b.loadDialogView();
    assert.equal(h.b.dialogViewError.value, '');
  });
}

test('global read receipt must prune covered cursors and preserve newer thread progress', async (t) => {
  for (const invalid of [true, false]) {
    const calls = [];
    const original = readingView('A', 1, { revision: 2, allReadSequence: 5,
      threadReadSequences: { team: 150, 'old-thread': 50 } });
    const h = await mount({ getNativeScanStatus: async (id) => ({ ...readingStatus(id),
      latestSequence: 200, timelineBeforeSequence: 101, timeline: longTimeline(200).slice(-100) }),
      getAgentDialogView: async () => original,
      saveAgentDialogView: async (input) => { calls.push(input); return readingView('A', 1,
        { revision: 3, allReadSequence: 100, threadReadSequences: invalid ? {} : { team: 150 } }); },
    }); t.after(h.unmount);
    h.b.state.value = { ...h.b.state.value, timelineBeforeSequence: 1, timeline: longTimeline(200) };
    h.b.showEarlierMessages();
    // Another thread on this page is unread, while team already has a later cursor.
    h.b.state.value.timeline[99].threadKey = 'other-thread';
    assert.equal(h.b.canMarkPageRead.value, true);
    await h.b.saveDialogView(true);
    assert.equal(calls[0].markReadThrough, 100);
    if (invalid) {
      assert.equal(h.b.dialogViewError.value, '阅读偏好保存失败，请重新读取后再操作。');
      assert.deepEqual(h.b.dialogView.value, original);
    } else {
      assert.equal(h.b.dialogViewError.value, '');
      assert.equal(h.b.dialogView.value.allReadSequence, 100);
      assert.deepEqual(h.b.dialogView.value.threadReadSequences, { team: 150 });
      h.b.showLatestMessages();
      assert.equal(h.b.visibleUnreadCount.value, 50);
    }
  }
});

test('valid read receipts preserve unrelated keys regardless of entry order or prototype-like names', async (t) => {
  const cursors = JSON.parse('{"__proto__":1,"constructor":2}');
  const h = await mount({ getNativeScanStatus: async (id) => ({ ...readingStatus(id),
    timeline: readingStatus(id).timeline.map(item => ({ ...item, threadKey: '__proto__' })) }),
    getAgentDialogView: async () => readingView('A', 1, { selectedThread: '__proto__', threadReadSequences: cursors }),
    saveAgentDialogView: async () => readingView('A', 1, { revision: 1, selectedThread: '__proto__',
      threadReadSequences: JSON.parse('{"constructor":2,"__proto__":3}') }),
  }); t.after(h.unmount);
  await h.b.saveDialogView(true);
  assert.equal(h.b.dialogViewError.value, '');
  assert.equal(h.b.visibleUnreadCount.value, 0);
  assert.equal(h.b.dialogView.value.threadReadSequences.constructor, 2);
});

test('zero finding candidates never claims coverage review exemption', async (t) => {
  const h = await mount({ getNativeScanStatus: async (id) => ({ ...status(id), findingCandidateCount: 0, multiAgentReady: true }) });
  t.after(h.unmount);
  const html = await renderDialog(h.b);
  assert.ok(html.includes('不代表覆盖审查完成'));
  assert.ok(!html.includes('免审'));
});
