// Real Vue setup/render; IPC is simulated. Original provenance is covered by Rust.
const { assert, test, renderDialog, status, event, mount } = require('./agent_dialog_harness.cjs');
const target = 'https://authorized.example.test';
function obligation(thread = 'team', extra = {}) {
  return { rootRunId: 'root-A', callId: 'a'.repeat(64), round: 4, targetKey: target,
    humanDirective: { schemaVersion: 1, directiveId: 'directive-A', draftId: 'draft-A', revision: 2,
      draftHash: 'b'.repeat(64), confirmationReceiptId: 'review-A', threadKey: thread, targetKey: target },
    state: 'cost_unconfirmed', reportedUsage: { inputTokens: 100000, cachedInputTokens: 0, outputTokens: 0, totalTokens: 100000, modelRequests: 1 },
    createdAt: '2026-10-05 08:00:00', ...extra };
}
const envelope = (items = [obligation()], extra = {}) => ({ schemaVersion: 1, executionAllowed: false, automaticResumeAllowed: false, items, truncated: false, ...extra });
const snapshot = (value, extra = {}) => ({ ...status('A'), humanAssessmentObligations: value, ...extra });
const panel = html => html.match(/<section[^>]*class="human-assessment-obligations"[\s\S]*?<\/section>/)?.[0] || '';
test('original unpublished Human assessment is visible only in its original selected thread without current recipient authority', async t => {
  for (const thread of ['team', target, 'assignment-A', 'coordinator:root-A']) {
    const m = await mount({ getNativeScanStatus: async () => snapshot(envelope([obligation(thread)]), { directiveRecipients: [] }) }); t.after(m.unmount);
    assert.ok(m.b.threadOptions.value.some(row => row.key === thread && row.count === 0));
    m.b.threadFilter.value = thread; const html = panel(await renderDialog(m.b));
    assert.match(html, /原评估待核对/); assert.match(html, /第 2 版/); assert.match(html, /100000/); assert.match(html, /未对账/);
    assert.doesNotMatch(html, /<button|已支付|已结算|已完成|正在思考|humanDirective|cost_unconfirmed/);
    m.b.threadFilter.value = 'foreign-thread'; assert.equal(panel(await renderDialog(m.b)), '');
  }
});
test('unknown unsent and pending receipts display separate readonly states without invented zero usage or automatic resume', async t => {
  for (const [state, label] of [['dispatch_unconfirmed', '派发结果尚未确认'], ['not_sent', '请求未发送'], ['awaiting_receipt', '等待原调用回执'], ['assessment_unpublished', '评估尚未发布']]) {
    const m = await mount({ getNativeScanStatus: async () => snapshot(envelope([obligation('team', { state, reportedUsage: null })])) }); t.after(m.unmount);
    const html = panel(await renderDialog(m.b)); assert.match(html, new RegExp(label)); assert.match(html, /用量尚未确认/); assert.match(html, /不会自动续跑/);
    assert.doesNotMatch(html, /0 tokens|输入 0|已结算|<button|正在思考/);
  }
});
test('malformed obligations cannot replace a verified view or consume its message cursor', async t => {
  let writes = 0;
  const safe = { ...status('A'), latestSequence: 20, timelineBeforeSequence: 20, timeline: [event('safe-before', 'mailbox_message', 20)] };
  const m = await mount({ getNativeScanStatus: async () => safe, saveAgentDialogView: async () => { writes++; throw Error('unexpected save'); } }); t.after(m.unmount);
  const row = obligation(); const human = row.humanDirective;
  const invalid = [null, [], undefined, envelope([], { schemaVersion: 2 }), envelope([], { executionAllowed: true }), envelope([], { automaticResumeAllowed: true }),
    envelope([], { rawBody: 'PRIVATE_BODY' }), envelope([], { truncated: 'yes' }), envelope([row, row]), envelope(Array.from({ length: 51 }, (_, n) => ({ ...row, callId: n.toString(16).padStart(64, '0') })))];
  for (const change of [{ state: 'paid' }, { rootRunId: '' }, { callId: 'bad' }, { round: 0 }, { round: 1.5 }, { targetKey: 'https://foreign.test' },
    { createdAt: '2026-13-05 08:00:00' }, { createdAt: '2026-02-30 08:00:00' }, { rawBody: 'PRIVATE_BODY' }, { reportedUsage: { ...row.reportedUsage, inputTokens: -1 } },
    { reportedUsage: { ...row.reportedUsage, outputTokens: NaN } }, { reportedUsage: { ...row.reportedUsage, modelRequests: Number.MAX_SAFE_INTEGER + 1 } },
    { reportedUsage: { ...row.reportedUsage, secret: 'PRIVATE_BODY' } }]) invalid.push(envelope([{ ...row, ...change }]));
  for (const change of [{ schemaVersion: 2 }, { directiveId: '' }, { draftHash: 'bad' }, { revision: 0 }, { threadKey: 'team\nforeign' },
    { threadKey: 'coordinator:foreign-root' }, { targetKey: 'https://foreign.test' }, { confirmationReceiptId: '' }, { rawBody: 'PRIVATE_BODY' }]) invalid.push(envelope([{ ...row, humanDirective: { ...human, ...change } }]));
  for (const value of invalid) {
    m.api.getNativeScanStatus = async () => snapshot(value, { latestSequence: 21, isIncremental: true });
    await m.b.loadStatus(20); assert.equal(m.b.statusSync.latestSequence.value, 20, JSON.stringify(value));
    assert.equal(m.b.state.value.timeline[0].id, 'safe-before'); assert.ok(m.b.statusError.value); assert.equal(panel(await renderDialog(m.b)), '');
  }
  assert.equal(writes, 0);
});
test('same-cursor obligation snapshots update clear and retain original thread without creating timeline messages or read acknowledgements', async t => {
  let writes = 0;
  const pending = obligation('team', { state: 'awaiting_receipt', reportedUsage: null });
  const m = await mount({ getNativeScanStatus: async () => snapshot(envelope([pending]), { latestSequence: 20 }), saveAgentDialogView: async () => { writes++; throw Error('unexpected save'); } }); t.after(m.unmount);
  m.b.threadFilter.value = 'team'; assert.match(panel(await renderDialog(m.b)), /等待原调用回执/);
  m.api.getNativeScanStatus = async () => snapshot(envelope([{ ...pending, state: 'dispatch_unconfirmed' }]), { latestSequence: 20, isIncremental: true });
  await m.b.loadStatus(20); await m.b.loadStatus(20);
  const html = panel(await renderDialog(m.b)); assert.match(html, /派发结果尚未确认/); assert.equal(html.match(/第 2 版/g).length, 1);
  assert.equal(m.b.state.value.timeline.length, 0); assert.equal(m.b.statusSync.latestSequence.value, 20); assert.equal(m.b.threadFilter.value, 'team');
  m.api.getNativeScanStatus = async () => snapshot(envelope([]), { latestSequence: 20, isIncremental: true });
  await m.b.loadStatus(20); assert.equal(panel(await renderDialog(m.b)), ''); assert.equal(m.b.threadFilter.value, 'team'); assert.equal(writes, 0);
});
test('supplier reported inconsistent usage stays labelled unsettled and missing optional Native snapshot metadata remains accepted', async t => {
  const row = obligation(); row.reportedUsage.totalTokens = 7;
  const m = await mount({ getNativeScanStatus: async () => snapshot(envelope([row])) }); t.after(m.unmount);
  const html = panel(await renderDialog(m.b)); assert.match(html, /100000/); assert.match(html, /未对账/); assert.ok(!m.b.statusError.value);
  m.api.getNativeScanStatus = async () => status('A'); await m.b.loadStatus();
  assert.ok(!m.b.statusError.value); assert.equal(panel(await renderDialog(m.b)), '');
});
