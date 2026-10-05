// Real AgentDialog setup/render regressions: receipts.
const { assert, test, renderDialog, deferred, flush, eventRefreshClock, status,
  directive, mount, pendingReceipt, event } = require('./agent_dialog_harness.cjs');

test('follow-up review renders source gap disposition separately from task status and escapes evidence', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), stopDiagnostic: { code: 'in_progress', obligations: [] },
    followup: { source: { sourceScanId: 'old', candidateId: 'old-candidate', candidateRevision: 2 }, tasks: [], gapResolved: false,
      review: { status: 'pending_review', gapResolved: false } },
  }) });
  t.after(unmount);
  let html = await renderDialog(b);
  assert.match(html, /原证据缺口尚未收口/);
  assert.doesNotMatch(html, /原缺证项已由新证据/);
  b.state.value.followup.review = { status: 'resolved', gapResolved: true, verdict: 'confirmed', hypothesisVerdict: 'rejected',
    summary: '<script>not executable</script>', assessment: { items: [{ index: 0, missingEvidence: 'control', status: 'addressed', factRefs: ['new-fact'], reason: 'Fresh control disproves the original claim' }] } };
  html = await renderDialog(b);
  assert.match(html, /原缺证项已由新证据/);
  assert.match(html, /原假设：rejected/);
  assert.match(html, /new-fact/);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>/);
  b.state.value.followup.review = { status: 'unverified', gapResolved: false, reasonCode: 'gap_review_context_changed' };
  html = await renderDialog(b);
  assert.match(html, /原证据缺口尚未收口/);
  assert.doesNotMatch(html, /原缺证项已由新证据/);
});

test('source task shows independent per-follow-up results without treating completed as resolved', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), stopDiagnostic: { code: 'in_progress', obligations: [] },
    followup: { source: null, gapResolved: false, tasks: [
      { scanId: 'new-1', taskName: 'first', status: 'completed', assessmentMessageId: 'gap-1', review: { status: 'pending_review', gapResolved: false } },
      { scanId: 'new-2', taskName: 'second', status: 'completed_with_gaps', assessmentMessageId: 'gap-2', review: { status: 'resolved', gapResolved: true, hypothesisVerdict: 'rejected' } },
    ] },
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /pending_review/);
  assert.match(html, /resolved/);
  assert.match(html, /不代表缺口已解决/);
  assert.match(html, /原假设：rejected/);
});

test('linked task events refresh terminal source disposition without mixing independent chat cursors', async (t) => {
  const clock = eventRefreshClock();
  let notify, reads = 0;
  const { b, unmount } = await mount({ getNativeScanStatus: async (id, after) => {
    reads++;
    return { ...status(id), latestSequence: 90, followup: { source: null, gapResolved: false,
      tasks: [{ scanId: 'child', status: 'completed', review: { status: reads > 1 ? 'resolved' : 'pending_review', gapResolved: reads > 1 } }] } };
  } }, { ...clock, listen: async (_, handler) => { notify = handler; return () => {}; } });
  t.after(unmount);
  const initial = reads;
  notify({ payload: { scanId: 'unrelated', sequence: 100, attemptNumber: 1 } }); await flush();
  assert.equal(reads, initial);
  for (let i = 0; i < 100; i++) notify({ payload: { scanId: 'child', sequence: 1, attemptNumber: 3 } });
  await clock.tick();
  assert.equal(reads, initial + 1, 'child cursor must not be compared with source cursor');
  assert.equal(b.state.value.followup.tasks[0].review.gapResolved, true);
  assert.equal(b.state.value.scanId, 'A');
  assert.equal(b.statusSync.latestSequence.value, 90);
  assert.deepEqual(b.state.value.timeline, []);
});

test('saved receipt UI distinguishes reconciliation from execution and escapes assessment text', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [pendingReceipt()], stopDiagnostic: { code: 'in_progress', obligations: [] },
  }) });
  t.after(unmount);
  assert.match(await renderDialog(b), /补齐本地回执（不重试模型）/);
  const item = b.state.value.timeline[0];
  item.taskClosure.disposition = 'outcome_unknown';
  assert.doesNotMatch(await renderDialog(b), /补齐本地回执（不重试模型）/);
  item.taskClosure.disposition = 'receipt_pending';
  item.localReconciliation = { status: 'completed', completedAt: '2026-09-26' };
  let html = await renderDialog(b);
  assert.match(html, /已接收保存的只读评估/);
  assert.match(html, /建议未执行，不代表漏洞验证或 Reviewer 通过/);
  assert.match(html, /本次补齐未调用模型、未请求目标/);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>|完成回执尚未提交|补齐本地回执（不重试模型）/);
  item.localReconciliation.status = 'failed';
  html = await renderDialog(b);
  assert.match(html, /保存的模型响应不是有效评估/);
  assert.doesNotMatch(html, /已接收保存的只读评估/);
});

test('reconcileReceipt uses the current persisted card and prevents duplicate submissions', async (t) => {
  const pending = deferred(), calls = [];
  const { b, unmount } = await mount({
    getNativeScanStatus: async (id) => ({ ...status(id), latestSequence: 1, timelineBeforeSequence: 1, timeline: [pendingReceipt()] }),
    reconcileScanDirectiveReceipt: (...args) => { calls.push(args); return pending.promise; },
  });
  t.after(unmount);
  await b.reconcileReceipt(pendingReceipt());
  assert.equal(calls.length, 0, 'detached or stale cards cannot submit');
  const item = b.state.value.timeline[0];
  const operation = b.reconcileReceipt(item);
  await b.reconcileReceipt(item);
  assert.deepEqual(calls, [['A', 1, 'saved-receipt']]);
  assert.equal(b.sending.value, true);
  pending.resolve({ status: 'completed' });
  await operation;
  assert.equal(b.sending.value, false);
  assert.equal(b.error.value, '');
});

for (const switchKind of ['task', 'project', 'round-trip', 'attempt']) {
  test(`reconcileReceipt ignores stale completions after ${switchKind} changes`, async (t) => {
    const old = deferred(), next = deferred();
    const { b, rootProps, unmount } = await mount({
      getNativeScanStatus: async (id) => ({ ...status(id), latestSequence: 1, timelineBeforeSequence: 1, timeline: [pendingReceipt()] }),
      reconcileScanDirectiveReceipt: () => old.promise,
      draftScanDirective: () => next.promise,
    });
    t.after(unmount);
    const operation = b.reconcileReceipt(b.state.value.timeline[0]);
    if (switchKind === 'project') rootProps.projectId = 2;
    else if (switchKind === 'attempt') b.state.value = { ...b.state.value, attemptNumber: 2 };
    else b.scanId.value = 'B';
    await flush();
    if (switchKind === 'round-trip') { b.scanId.value = 'A'; await flush(); }
    b.draft.value = '新的想法';
    const newer = b.send();
    assert.equal(b.sending.value, true);
    old.reject(new Error('stale reconciliation error'));
    await operation;
    assert.equal(b.error.value, '');
    assert.equal(b.sending.value, true, 'old completion must not unlock the new operation');
    next.resolve(directive(b.scanId.value));
    await newer;
    assert.equal(b.sending.value, false);
  });
}

test('the real template previews scheduling actions and distinguishes receipts from verification', async (t) => {
  const fixture = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    stopDiagnostic: { code: 'in_progress', obligations: [] }, findingCandidateCount: 0,
    directiveDrafts: [{ ...directive(id, 'drafted'), text: '请优先检查权限控制', threadKey: 'team',
      coordinatorDecision: 'accept', validationResult: 'valid', intent: 'priority_adjustment',
      requestedRoles: ['coordinator'], priorityChanges: ['prioritize_family:authorization'],
      sideEffectClass: 'read_only', estimatedTokens: 0, estimatedRequests: 0,
      requiredApprovals: ['directive_confirmation'], reasonCodes: [],
    }],
    timeline: [{ ...event('priority-receipt', 'user_directive', 1), fromRole: 'operator',
      toRole: 'coordinator', threadKey: 'team', summary: '请优先检查权限控制', status: 'completed',
      deliveryState: 'persisted', ackState: 'completed',
      queueAction: { kind: 'prioritize_family', family: 'authorization', matchedItems: 2,
        changedOrder: true, coverageVerified: false, targetRequests: 0, modelRequests: 0 },
    }],
  }) });
  t.after(fixture.unmount);
  let html = await renderDialog(fixture.b);
  assert.match(html, /调度动作/);
  assert.match(html, /prioritize_family:authorization/);
  assert.match(html, /无匹配待办时不应用/);
  assert.match(html, /已落实队列优先级/);
  assert.match(html, /2 项匹配待办/);
  assert.match(html, /不代表扫描、漏洞验证或复核完成/);
  assert.doesNotMatch(html, /请求已送达模型/);
  fixture.b.state.value.timeline[0].queueAction.changedOrder = false;
  html = await renderDialog(fixture.b);
  assert.match(html, /匹配项原已在前，优先级已保存/);
  fixture.b.state.value.timeline[0].deliveryState = 'receipt_unverified';
  fixture.b.state.value.timeline[0].reasonCodes = ['queue_action_receipt_unverified'];
  html = await renderDialog(fixture.b);
  assert.match(html, /队列优先级回执无法核验/);
  assert.match(html, /恢复时不会静默重放/);
  assert.doesNotMatch(html, /已落实队列优先级/);
  fixture.b.state.value.timeline[0].queueAction = null;
  html = await renderDialog(fixture.b);
  assert.match(html, /队列优先级回执无法核验/);
  fixture.b.state.value.timeline[0].reasonCodes = [];
  fixture.b.state.value.timeline[0] = { ...fixture.b.state.value.timeline[0],
    status: 'accepted', deliveryState: 'model_received', queueAction: null };
  html = await renderDialog(fixture.b);
  assert.match(html, /请求已送达模型，但尚无队列调整/);
  assert.doesNotMatch(html, /已落实队列优先级/);
  fixture.b.state.value.timeline[0] = { ...fixture.b.state.value.timeline[0],
    status: 'deferred', deliveryState: 'persisted', reasonCodes: ['priority_no_matching_pending_work'] };
  html = await renderDialog(fixture.b);
  assert.match(html, /当前没有匹配的待办，优先级未应用/);
  assert.match(html, /不会在以后静默生效/);
  assert.doesNotMatch(html, /已落实队列优先级/);
});

test('the real template distinguishes proposal completion, failure and unknown outcomes', async (t) => {
  const fixture = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    stopDiagnostic: { code: 'in_progress', obligations: [] }, findingCandidateCount: 0,
    timeline: [{ id: 'proposal-receipt', eventType: 'user_directive', sequence: 1, timestamp: 'stamp', fromRole: 'operator',
      toRole: 'coordinator', threadKey: 'team', summary: '@mapper 请评估已有证据', status: 'completed',
      deliveryState: 'persisted', ackState: 'completed', proposalAction: {
        state: 'completed', assignmentId: 'assignment-1', childRunId: 'child-1',
        summary: '已评估现有证据 <script>unsafe()</script>', advisoryOnly: true, coverageVerified: false,
      },
    }],
  }) });
  t.after(fixture.unmount);
  let html = await renderDialog(fixture.b);
  assert.match(html, /独立 Agent 已提交只读评估/);
  assert.match(html, /建议尚未执行，不代表漏洞验证或 Reviewer 复核完成/);
  assert.doesNotMatch(html, /<script>unsafe/);
  fixture.b.state.value.timeline[0].proposalAction.state = 'uncertain';
  fixture.b.state.value.timeline[0].status = 'assigned';
  html = await renderDialog(fixture.b);
  assert.match(html, /模型调用结果未知，未自动重试/);
  assert.doesNotMatch(html, /独立 Agent 已提交只读评估/);
  fixture.b.state.value.timeline[0].proposalAction.state = 'failed';
  fixture.b.state.value.timeline[0].status = 'failed';
  html = await renderDialog(fixture.b);
  assert.match(html, /子任务未返回有效评估/);
  assert.doesNotMatch(html, /独立 Agent 已提交只读评估/);
  const event = fixture.b.state.value.timeline[0];
  event.status = 'completed';
  event.proposalAction.state = 'completed';
  event.reasonCodes = ['proposal_receipt_unverified'];
  event.deliveryState = 'receipt_unverified';
  for (const action of [event.proposalAction, null]) {
    event.proposalAction = action;
    html = await renderDialog(fixture.b);
    assert.match(html, /评估交付回执无法核验/);
    assert.doesNotMatch(html, /独立 Agent 已提交只读评估/);
    assert.doesNotMatch(html, /评估请求暂未派发/);
    assert.doesNotMatch(html, /已评估现有证据/);
  }
});

test('terminal task receipts distinguish cancellation, unknown outcome and stored response', async (t) => {
  const fixture = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    stopDiagnostic: { code: 'in_progress', obligations: [] }, findingCandidateCount: 0,
    timeline: [{ ...event('closed', 'user_directive', 1), fromRole: 'operator', toRole: 'coordinator',
      threadKey: 'team', summary: '<script>unsafe()</script>', status: 'deferred', ackState: 'deferred',
      proposalAction: { state: 'failed', summary: 'stale proposal' },
      taskClosure: { disposition: 'not_started', automaticRetry: false, requiresReconciliation: false },
    }],
  }) });
  t.after(fixture.unmount);
  for (const [disposition, text] of [
    ['not_started', /尚未启动，已取消并释放其未用预算/],
    ['not_applied', /指令未取得动作落实回执/],
    ['outcome_unknown', /模型调用结果未知，预算预留保留/],
    ['receipt_pending', /模型响应已保存，但完成回执尚未提交/],
    ['reconciliation_required', /执行回执或租约归属需核对/],
  ]) {
    fixture.b.state.value.timeline[0].taskClosure.disposition = disposition;
    const html = await renderDialog(fixture.b);
    assert.match(html, text);
    assert.match(html, /本目标任务已结束/);
    assert.match(html, /不会自动重试或在后续任务静默生效/);
    assert.doesNotMatch(html, /子任务未返回有效评估|其他已授权 Web 工作可继续|stale proposal|<script>unsafe/);
  }
});
