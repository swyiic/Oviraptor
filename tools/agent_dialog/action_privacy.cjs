// Failed transport fixtures only: no model, target, task creation or execution.
const { assert, test, mount, status, directive, pendingReceipt, flush, renderDialog } = require('../agent_dialog_harness.cjs');
const secret = '/private/fixture.sqlite?token=fixture-secret Authorization: Bearer fixture-key';
const historical = { directiveId: 'old-directive', attemptNumber: 1, createdAt: 'old', verified: false };
const cases = [
  ['draftScanDirective', '草案提交结果无法核实，输入已保留；请刷新核对后再操作，不要直接重复提交。', b => b.send()],
  ['confirmScanDirective', '确认结果无法核实；请刷新核对草案和任务状态，不要直接重复确认。',
    b => b.confirmDirective(b.state.value.directiveDrafts[0])],
  ['cancelScanDirective', '取消结果无法核实；请刷新核对草案状态后再操作。',
    b => b.cancelDirective(b.state.value.directiveDrafts[0])],
  ['reconcileScanDirectiveReceipt', '本地回执核对结果无法确认；请刷新后核对记录。本操作不会重新执行任务。',
    b => b.reconcileReceipt(b.state.value.timeline[0])],
  ['reconcileHistoricalScanDirectiveReceipt', '历史回执核对结果无法确认；请重新读取旧记录。本操作不会重新执行任务。',
    b => { b.selectedHistoricalReceipt.value = '1:old-directive'; return b.reconcileHistoricalReceipt(historical); }],
  ['previewAgentGapFollowup', '无法准备补充任务预览；请刷新核对证据是否缺失或变化。本操作未创建新任务。',
    b => b.prepareFollowup({ id: 'assessment', gapAssessment: { newAttemptRequired: true } })],
];

for (const kind of ['Error', 'string']) for (const [api, message, action] of cases) {
  test(`${api} hides ${kind} details and preserves the last verified view`, async t => {
    const h = await mount({ getNativeScanStatus: async id => ({ ...status(id), attemptNumber: 2,
      latestSequence: 1, timelineBeforeSequence: 1, timeline: [pendingReceipt()],
      directiveDrafts: [{ ...directive(id), attemptNumber: 2 }],
      historicalPendingReceipts: 1, historicalReceiptItems: [historical] }) });
    t.after(h.unmount);
    h.b.draft.value = 'keep this input';
    const snapshot = h.b.state.value;
    let calls = 0;
    h.api[api] = async () => { calls++; throw kind === 'Error' ? new Error(secret) : secret; };
    await action(h.b); await flush();
    assert.equal(calls, 1, 'no automatic replay');
    assert.equal(h.b.actionError.value, message);
    assert.equal(h.b.state.value, snapshot, 'transport failure is not a success receipt');
    assert.equal(h.b.draft.value, 'keep this input');
    assert.equal(h.b.sending.value, false);
    assert.equal(h.b.preparingFollowup.value, false);
    assert.deepEqual(h.events, []);
    const html = await renderDialog(h.b);
    assert.ok(html.includes(message));
    for (const text of ['/private/', 'fixture-secret', 'fixture-key', 'Authorization:']) {
      assert.ok(!html.includes(text));
      assert.ok(!h.b.actionError.value.includes(text));
    }
  });
}

for (const [code, hint] of [
  ['directive_thread_coordinator_ambiguous', '多个目标'],
  ['directive_thread_has_no_coordinator', '没有可接收消息'],
  ['directive_draft_recipient_not_bound', '未绑定有效接收方'],
  ['directive_draft_stale_fencing_token', '执行租约已变化'],
]) {
  test(`known ${code} retains safe guidance without attached transport details`, async t => {
    const h = await mount({ draftScanDirective: async () => { throw new Error(`${code}: ${secret}`); } });
    t.after(h.unmount);
    h.b.draft.value = 'preserve'; await h.b.send();
    assert.ok(h.b.actionError.value.includes(hint));
    assert.ok(!h.b.actionError.value.includes('fixture-secret'));
    assert.equal(h.b.draft.value, 'preserve');
  });
}
