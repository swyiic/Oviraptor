// Real AgentDialog setup/render regressions: followup.
const { assert, test, renderDialog, deferred, flush, eventRefreshClock, status,
  directive, mount, pendingReceipt, event } = require('./agent_dialog_harness.cjs');

test('follow-up preview emits authoritative source only once and makes no task or approval call', async (t) => {
  const pending = deferred(); const calls = [];
  const { b, events, unmount } = await mount({ previewAgentGapFollowup: (...args) => { calls.push(args); return pending.promise; } });
  t.after(unmount);
  const item = { id: 'assessment', gapAssessment: { newAttemptRequired: true } };
  const request = b.prepareFollowup(item);
  await b.prepareFollowup(item);
  const preview = { sourceScanId: 'A', sourceHash: 'server-verified' };
  pending.resolve(preview); await request;
  assert.deepEqual(calls, [['A', 'assessment']]);
  assert.deepEqual(events, [['followup', preview]]);
});

test('follow-up preview drops stale A-B-A success and errors', async (t) => {
  for (const fails of [false, true]) {
    const pending = deferred();
    const { b, events, unmount } = await mount({ previewAgentGapFollowup: () => pending.promise });
    t.after(unmount);
    const operation = b.prepareFollowup({ id: 'assessment', gapAssessment: { newAttemptRequired: true } });
    b.scanId.value = 'B'; await flush(); b.scanId.value = 'A'; await flush();
    if (fails) pending.reject(new Error('stale error')); else pending.resolve({ sourceScanId: 'A' });
    await operation;
    assert.deepEqual(events, []);
    assert.equal(b.actionError.value, '');
  }
});

test('follow-up preview from an old attempt cannot open a draft or replace current errors', async (t) => {
  for (const fails of [false, true]) {
    const pending = deferred();
    const { b, events, unmount } = await mount({ previewAgentGapFollowup: () => pending.promise });
    t.after(unmount);
    const operation = b.prepareFollowup({ id: 'old-assessment', gapAssessment: { newAttemptRequired: true } });
    b.state.value = { ...status('A'), attemptNumber: 2 };
    b.actionError.value = 'current attempt error';
    if (fails) pending.reject(new Error('old attempt error'));
    else pending.resolve({ sourceScanId: 'A', sourceAttemptNumber: 1 });
    await operation;
    assert.deepEqual(events, []);
    assert.equal(b.actionError.value, 'current attempt error');
    assert.equal(b.preparingFollowup.value, false);
  }
});

test('follow-up preview busy state belongs to the current task and attempt, not an old request', async (t) => {
  for (const change of ['task', 'attempt', 'project', 'invalidated_snapshot']) {
    const oldRequest = deferred(); const newRequest = deferred(); const calls = [];
    const { b, rootProps, events, unmount } = await mount({ previewAgentGapFollowup: (...args) => {
      calls.push(args);
      return calls.length === 1 ? oldRequest.promise : newRequest.promise;
    } });
    t.after(unmount);
    const item = { id: 'assessment', gapAssessment: { newAttemptRequired: true } };
    const oldOperation = b.prepareFollowup(item);
    if (change === 'task') b.scanId.value = 'B';
    else if (change === 'attempt') b.state.value = { ...status('A'), attemptNumber: 2 };
    else if (change === 'project') rootProps.projectId = 2;
    else {
      b.state.value = undefined;
      b.state.value = { ...status('A'), attemptNumber: 2 };
    }
    await flush();
    assert.equal(b.preparingFollowup.value, false, change);
    const newOperation = b.prepareFollowup(item);
    assert.equal(calls.length, 2, change);
    assert.equal(b.preparingFollowup.value, true, change);
    oldRequest.resolve({ sourceScanId: 'A', obsolete: true });
    await oldOperation;
    assert.deepEqual(events, [], change);
    assert.equal(b.preparingFollowup.value, true, `${change}: old finally must not unlock new request`);
    await b.prepareFollowup(item);
    assert.equal(calls.length, 2, `${change}: current request remains single-flight`);
    const preview = { sourceScanId: b.scanId.value, sourceHash: `new-${change}` };
    newRequest.resolve(preview); await newOperation;
    assert.deepEqual(events, [['followup', preview]], change);
    assert.equal(b.preparingFollowup.value, false, change);
  }
});

test('same-attempt status polling does not invalidate an in-flight follow-up preview', async (t) => {
  const pending = deferred();
  const { b, events, unmount } = await mount({ previewAgentGapFollowup: () => pending.promise });
  t.after(unmount);
  const operation = b.prepareFollowup({ id: 'assessment', gapAssessment: { newAttemptRequired: true } });
  b.state.value = { ...status('A'), latestSequence: 19 };
  assert.equal(b.preparingFollowup.value, true);
  const preview = { sourceScanId: 'A', sourceHash: 'current' };
  pending.resolve(preview); await operation;
  assert.deepEqual(events, [['followup', preview]]);
  assert.equal(b.preparingFollowup.value, false);
});

test('follow-up preview exposes current rejection but drops response after unmount', async (t) => {
  const pending = deferred();
  const { b, events, api, unmount } = await mount({ previewAgentGapFollowup: async () => { throw new Error('evidence missing'); } });
  t.after(unmount);
  const item = { id: 'assessment', gapAssessment: { newAttemptRequired: true } };
  await b.prepareFollowup(item);
  assert.equal(b.actionError.value, '无法准备补充任务预览；请刷新核对证据是否缺失或变化。本操作未创建新任务。');
  assert.deepEqual(events, []);
  api.previewAgentGapFollowup = () => pending.promise;
  const operation = b.prepareFollowup(item);
  unmount(); pending.resolve({ sourceScanId: 'A' }); await operation;
  assert.deepEqual(events, []);
});

test('older saved receipts surface as read-only debt without a current-attempt action', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), attemptNumber: 2, historicalPendingReceipts: 1, timeline: [],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /旧尝试仍有待核对记录/);
  assert.match(html, /不代表旧响应已经核验/);
  assert.match(html, /逐条选择后可尝试独立的历史本地对账/);
  assert.doesNotMatch(html, /补齐本地回执（不重试模型）/);
  assert.doesNotMatch(html, /确认本地对账/);
});

test('historical receipt requires exact identity and a separate confirmation', async (t) => {
  const pending = deferred(), calls = [];
  const record = { directiveId: 'old-directive', attemptNumber: 1,
    createdAt: '2026-09-26 10:00:00', verified: false };
  const { b, unmount } = await mount({
    getNativeScanStatus: async (id) => ({ ...status(id), attemptNumber: 2,
      historicalPendingReceipts: 1, historicalReceiptItems: [record] }),
    reconcileHistoricalScanDirectiveReceipt: (...args) => { calls.push(args); return pending.promise; },
  });
  t.after(unmount);
  let html = await renderDialog(b);
  assert.match(html, /old-directive/);
  assert.doesNotMatch(html, /确认本地对账/);
  await b.reconcileHistoricalReceipt(record);
  assert.equal(calls.length, 0);
  b.selectedHistoricalReceipt.value = '1:old-directive';
  html = await renderDialog(b);
  assert.match(html, /只结算此扫描、此尝试、此建议/);
  assert.match(html, /确认本地对账/);
  const operation = b.reconcileHistoricalReceipt(record);
  await b.reconcileHistoricalReceipt(record);
  assert.deepEqual(calls, [['A', 1, 'old-directive']]);
  pending.resolve({ status: 'completed' });
  await operation;
  assert.equal(b.selectedHistoricalReceipt.value, '');
  assert.equal(b.sending.value, false);
});

test('historical reconciliation cannot publish a late result into another attempt', async (t) => {
  const pending = deferred();
  const record = { directiveId: 'old-directive', attemptNumber: 1,
    createdAt: '2026-09-26 10:00:00', verified: false };
  const { b, unmount } = await mount({
    getNativeScanStatus: async (id) => ({ ...status(id), attemptNumber: 2,
      historicalPendingReceipts: 1, historicalReceiptItems: [record] }),
    reconcileHistoricalScanDirectiveReceipt: () => pending.promise,
  });
  t.after(unmount);
  b.selectedHistoricalReceipt.value = '1:old-directive';
  const operation = b.reconcileHistoricalReceipt(record);
  b.state.value = { ...b.state.value, attemptNumber: 3 };
  await flush();
  assert.equal(b.selectedHistoricalReceipt.value, '');
  pending.reject(new Error('obsolete historical response'));
  await operation;
  assert.equal(b.error.value, '');
  assert.equal(b.sending.value, false);
});
