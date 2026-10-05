// Vue components with simulated IPC. Rust tests supply the real paid provenance.
const { assert, test, renderDialog, flush, event, status, mount } = require('./agent_dialog_harness.cjs');
function humanItem(thread = 'team', extra = {}, recordExtra = {}) {
  const target = 'https://authorized.example.test';
  return { ...event('["root-A",7]', 'root_decision', 21), fromRole: 'coordinator', fromRunId: 'root-A', toRole: 'operator', toRunId: '',
    threadKey: thread, targetKey: target, status: 'recorded', deliveryState: 'persisted', ackState: 'n/a', messageKind: 'root_decision_summary',
    correlationId: 'a'.repeat(64), assignmentId: '', evidenceRevision: 0,
    decisionRecord: { schemaVersion: 1, advisoryOnly: true, callId: 'a'.repeat(64), round: 4, modelEventSequence: 7,
      summary: { schemaVersion: 1, observed: ['原确认指令的已记录评估'], missing: ['仍待实际工具执行'], suggestions: ['assess:human_directive'], costNotes: [], risks: ['尚待独立验证'] },
      usage: { inputTokens: 80, cachedInputTokens: 20, outputTokens: 20, totalTokens: 100, modelRequests: 1 },
      humanDirective: { schemaVersion: 1, directiveId: 'directive-A', draftId: 'draft-A', revision: 2, draftHash: 'b'.repeat(64), confirmationReceiptId: 'review-A', threadKey: thread, targetKey: target },
      ...recordExtra }, ...extra };
}
const snapshot = (row = humanItem(), extra = {}) => ({ ...status('A'), latestSequence: row.sequence, timelineBeforeSequence: row.sequence, timeline: [row], ...extra });
test('confirmed directive summary remains in its original selected thread with readable advisory and version', async t => {
  for (const thread of ['team', 'https://authorized.example.test', 'assignment-A', 'coordinator:root-A']) {
    const m = await mount({ getNativeScanStatus: async () => snapshot(humanItem(thread)) }); t.after(m.unmount);
    m.b.threadFilter.value = thread; const html = await renderDialog(m.b);
    assert.match(html, /原确认指令的已记录评估/); assert.match(html, /已确认指令/); assert.match(html, /第 2 版/);
    assert.match(html, /建议评估已确认的指令/); assert.match(html, /100 tokens/); assert.match(html, /建议不授予执行权限/);
    assert.doesNotMatch(html, /assess:human_directive|humanDirective|confirmationReceiptId|原指令已完成|正在思考/);
    m.b.threadFilter.value = 'foreign-thread'; assert.doesNotMatch(await renderDialog(m.b), /原确认指令的已记录评估|第 2 版/);
  }
});
test('wrong thread target confirmation revision or private context cannot consume a chat cursor', async t => {
  const m = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 20, timelineBeforeSequence: 20, timeline: [event('safe-before', 'mailbox_message', 20)] }) }); t.after(m.unmount);
  const base = humanItem().decisionRecord.humanDirective;
  for (const change of [{ threadKey: 'foreign-thread' }, { targetKey: 'https://foreign.test' }, { revision: 0 }, { revision: 1.5 },
    { schemaVersion: 2 }, { draftHash: 'bad' }, { directiveId: '' }, { confirmationReceiptId: '' }, { threadKey: 'team\nforeign' }, { rawBody: 'PRIVATE_BODY' }]) {
    m.api.getNativeScanStatus = async () => snapshot(humanItem('team', {}, { humanDirective: { ...base, ...change } }), { isIncremental: true });
    await m.b.loadStatus(20); assert.equal(m.b.statusSync.latestSequence.value, 20, JSON.stringify(change)); assert.equal(m.b.state.value.timeline[0].id, 'safe-before'); assert.ok(m.b.statusError.value);
    assert.doesNotMatch(await renderDialog(m.b), /PRIVATE_BODY|原确认指令的已记录评估/);
  }
  for (const humanDirective of [null, [], 'team', { ...base, revision: NaN }]) {
    m.api.getNativeScanStatus = async () => snapshot(humanItem('team', {}, { humanDirective }), { isIncremental: true });
    await m.b.loadStatus(20); assert.equal(m.b.statusSync.latestSequence.value, 20); assert.equal(m.b.state.value.timeline[0].id, 'safe-before');
  }
});
test('original human decision replay deduplicates its committed event and preserves selected thread', async t => {
  const m = await mount({ getNativeScanStatus: async () => snapshot() }); t.after(m.unmount);
  m.b.threadFilter.value = 'team'; m.api.getNativeScanStatus = async () => snapshot(humanItem(), { isIncremental: true });
  await m.b.loadStatus(20); await m.b.loadStatus(20); assert.equal(m.b.state.value.timeline.length, 1); assert.equal(m.b.threadFilter.value, 'team');
  assert.equal((await renderDialog(m.b)).match(/原确认指令的已记录评估/g).length, 1);
});
test('malformed historical human context keeps current paid view without acknowledging or saving', async t => {
  let writes = 0; const old = humanItem('team', { sequence: 9, id: '["root-A",5]', correlationId: 'c'.repeat(64) }, { modelEventSequence: 5, callId: 'c'.repeat(64) }); old.decisionRecord.humanDirective.threadKey = 'foreign-thread';
  const m = await mount({ getNativeScanStatus: async () => snapshot(humanItem(), { hasEarlierTimeline: true }),
    getNativeScanTimelinePage: async () => ({ scanId: 'A', attemptNumber: 1, beforeSequence: 21, timelineBeforeSequence: 9, hasEarlierTimeline: false, timeline: [old] }),
    saveAgentDialogView: async () => { writes++; throw Error('unexpected write'); } }); t.after(m.unmount);
  m.b.threadFilter.value = 'team'; m.b.showEarlierMessages(); await flush(); assert.ok(m.b.historyError.value); assert.equal(m.b.historyPage.value, undefined); assert.equal(writes, 0);
  assert.match(await renderDialog(m.b), /原确认指令的已记录评估/);
});
test('human context can accompany original bounded local tools without exposing arguments or result bodies', async t => {
  const m = await mount({ getNativeScanStatus: async () => snapshot(humanItem('team', {}, { localTools: ['snapshot.read', 'plan.propose'] })) }); t.after(m.unmount);
  const html = await renderDialog(m.b); assert.match(html, /原确认指令的已记录评估/); assert.match(html, /已确认指令/); assert.match(html, /读取任务快照/); assert.match(html, /提出计划建议/);
  assert.doesNotMatch(html, /snapshot.read|plan.propose|humanDirective|正在思考/);
});
test('budget-deferred confirmed directive has a readable reason and never appears executed', async t => {
  const row = { ...event('directive-A', 'user_directive', 22, 'prioritize authorization'), fromRole: 'operator', toRole: 'coordinator', status: 'deferred', deliveryState: 'persisted', ackState: 'deferred', reasonCodes: ['root_human_assessment_budget_unavailable'] };
  const m = await mount({ getNativeScanStatus: async () => snapshot(row) }); t.after(m.unmount);
  const html = await renderDialog(m.b); assert.match(html, /原预算不足以评估此指令，已延后处理/); assert.doesNotMatch(html, /root_human_assessment_budget_unavailable|已落实队列优先级|指令已执行/);
});
