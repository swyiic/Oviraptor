const { assert, test, renderDialog, deferred, flush, event, status, mount } = require('./agent_dialog_harness.cjs');
function rootItem(overrides = {}, recordOverrides = {}) {
  return { ...event('["root-A",7]', 'root_decision', 21, 'Root saved advisory record'),
    fromRole: 'coordinator', fromRunId: 'root-A', toRole: 'operator', toRunId: '',
    threadKey: 'coordinator:root-A', targetKey: 'https://authorized.example.test',
    status: 'recorded', deliveryState: 'persisted', ackState: 'n/a', messageKind: 'root_decision_summary',
    correlationId: 'a'.repeat(64), assignmentId: '', evidenceRevision: 0,
    decisionRecord: { schemaVersion: 1, advisoryOnly: true, callId: 'a'.repeat(64), round: 2, modelEventSequence: 7,
      summary: { schemaVersion: 1, observed: ['原始映射回执已保存'], missing: ['仍缺身份B证据'],
        suggestions: ['dispatch:web_executor'], costNotes: ['模型建议预估费用 999 美元'], risks: ['尚待独立验证'] },
      usage: { inputTokens: 80, cachedInputTokens: 20, outputTokens: 20, totalTokens: 100, modelRequests: 1 },
      ...recordOverrides }, ...overrides };
}
const snapshot = (row = rootItem(), id = 'A', extra = {}) => ({ ...status(id),
  latestSequence: row.sequence, timelineBeforeSequence: row.sequence, timeline: [row], ...extra });

test('actual chat status IPC renders bounded original paid summary with separate facts, advice and cost', async t => {
  const m = await mount({ getNativeScanStatus: async () => snapshot() }); t.after(m.unmount);
  const html = await renderDialog(m.b);
  for (const text of ['已记录决策摘要', '观察', '缺口', '下一步建议', '风险与局限', '本轮账本用量', '费用说明建议']) assert.ok(html.includes(text), text);
  assert.match(html, /原始映射回执已保存/); assert.match(html, /100 tokens/); assert.match(html, /1 次模型请求/);
  assert.match(html, /未核验单价，未换算金额/); assert.match(html, /建议不授予执行权限/);
  assert.doesNotMatch(html, /decisionRecord|modelEventSequence|dispatch:web_executor|实际费用.{0,20}999|正在思考/);
});

test('actual incremental replay deduplicates committed Root row and notification payload cannot become a summary', async t => {
  let notify; const m = await mount({ getNativeScanStatus: async () => snapshot() }, {
    listen: async (_name, callback) => { notify = callback; return () => {}; } }); t.after(m.unmount);
  m.api.getNativeScanStatus = async () => snapshot(rootItem(), 'A', { isIncremental: true });
  await m.b.loadStatus(20); await m.b.loadStatus(20);
  assert.equal(m.b.state.value.timeline.length, 1);
  notify({ payload: { scanId: 'foreign', attemptNumber: 1, sequence: 22, decisionRecord: { summary: { observed: ['FORGED_FACT'] } } } });
  await flush(); const html = await renderDialog(m.b);
  assert.match(html, /已记录决策摘要/); assert.doesNotMatch(html, /FORGED_FACT/);
});

test('actual history IPC consumes Root structured receipt and keeps selected coordinator thread', async t => {
  const old = rootItem({ sequence: 9, id: '["root-A",5]', correlationId: 'b'.repeat(64) }, { modelEventSequence: 5, round: 1, callId: 'b'.repeat(64) }); const calls = [];
  const m = await mount({ getNativeScanStatus: async () => snapshot(rootItem(), 'A', { hasEarlierTimeline: true }),
    getNativeScanTimelinePage: async (...args) => { calls.push(args); return { scanId: 'A', attemptNumber: 1,
      beforeSequence: 21, timelineBeforeSequence: 9, hasEarlierTimeline: false, timeline: [old] }; } }); t.after(m.unmount);
  m.b.threadFilter.value = 'coordinator:root-A'; m.b.showEarlierMessages(); await flush();
  assert.deepEqual(calls, [['A', 1, 21]]); assert.equal(m.b.timelinePage.value[0].sequence, 9);
  assert.match(await renderDialog(m.b), /已记录决策摘要/);
  m.b.threadFilter.value = 'team'; assert.doesNotMatch(await renderDialog(m.b), /已记录决策摘要/);
});

test('actual status composable rejects malformed/private summaries before consuming the collaboration cursor', async t => {
  const m = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 20,
    timelineBeforeSequence: 20, timeline: [event('safe-before', 'mailbox_message', 20)] }) }); t.after(m.unmount);
  for (const change of [{ schemaVersion: 2 }, { advisoryOnly: false }, { rawBody: 'PRIVATE_BODY' },
    { summary: { ...rootItem().decisionRecord.summary, reasoning: 'PRIVATE_CHAIN' } },
    { usage: { inputTokens: 80, cachedInputTokens: 90, outputTokens: 20, totalTokens: 100, modelRequests: 1 } }]) {
    m.api.getNativeScanStatus = async () => snapshot(rootItem({}, change), 'A', { isIncremental: true });
    await m.b.loadStatus(20);
    assert.equal(m.b.statusSync.latestSequence.value, 20, JSON.stringify(change));
    assert.equal(m.b.state.value.timeline[0].id, 'safe-before'); assert.ok(m.b.statusError.value);
    assert.doesNotMatch(await renderDialog(m.b), /PRIVATE_BODY|PRIVATE_CHAIN|已记录决策摘要/);
  }
});

test('actual receipt envelope binds original Root, event tuple, call and independent channel cursor', async t => {
  const m = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 20,
    timelineBeforeSequence: 20, timeline: [event('safe-before', 'mailbox_message', 20)] }) }); t.after(m.unmount);
  for (const change of [{ id: '["other-root",7]' }, { fromRole: 'spa_api_mapper' },
    { threadKey: 'coordinator:foreign' }, { correlationId: 'b'.repeat(64) }, { status: 'completed' }]) {
    m.api.getNativeScanStatus = async () => snapshot(rootItem(change), 'A', { isIncremental: true });
    await m.b.loadStatus(20); assert.equal(m.b.statusSync.latestSequence.value, 20, JSON.stringify(change));
    assert.equal(m.b.state.value.timeline[0].id, 'safe-before');
  }
});

test('actual project and attempt switch fences pending decision reads without showing stale or fake progress', async t => {
  const m = await mount(); t.after(m.unmount);
  const late = deferred(); let calls = 0;
  m.api.getNativeScanStatus = id => ++calls === 1 ? late.promise : Promise.resolve(
    snapshot(rootItem({ fromRunId: 'root-B', id: '["root-B",7]', threadKey: 'coordinator:root-B' }), id, { attemptNumber: 2 }));
  const read = m.b.loadStatus(); m.rootProps.projectId = 2; await flush();
  late.resolve(snapshot(rootItem({ summary: 'STALE_FACT' }))); await read; await flush();
  assert.doesNotMatch(await renderDialog(m.b), /STALE_FACT|正在思考/);
});

test('actual chat SFC escapes public text and empty decision lists never imply completion', async t => {
  const summary = { schemaVersion: 1, observed: ['<img src=x onerror=bad()>已保存'], missing: [], suggestions: [], costNotes: [], risks: [] };
  const m = await mount({ getNativeScanStatus: async () => snapshot(rootItem({}, { summary })) }); t.after(m.unmount);
  const html = await renderDialog(m.b); assert.match(html, /&lt;img/); assert.doesNotMatch(html, /<img|全部完成|>没有风险</);
  assert.match(html, /未提供缺口/); assert.match(html, /不代表没有风险/);
});

test('actual malformed history page retains current valid view and does not acknowledge/read a decision', async t => {
  let writes = 0;
  const m = await mount({ getNativeScanStatus: async () => snapshot(rootItem(), 'A', { hasEarlierTimeline: true }),
    getNativeScanTimelinePage: async () => ({ scanId: 'A', attemptNumber: 1, beforeSequence: 21,
      timelineBeforeSequence: 9, hasEarlierTimeline: false, timeline: [rootItem({ sequence: 9 }, { rawBody: 'HIDDEN_BODY' })] }),
    saveAgentDialogView: async () => { writes++; throw Error('unexpected write'); } }); t.after(m.unmount);
  m.b.showEarlierMessages(); await flush(); assert.ok(m.b.historyError.value);
  assert.equal(m.b.historyPage.value, undefined); assert.equal(writes, 0);
  assert.match(await renderDialog(m.b), /已记录决策摘要/); assert.doesNotMatch(await renderDialog(m.b), /HIDDEN_BODY/);
});


test('actual chat renders original local tools as bounded recorded work without args or result bodies', async t => {
  const m = await mount({ getNativeScanStatus: async () => snapshot(rootItem({}, {
    localTools: ['snapshot.read', 'capability_budget.read', 'plan.propose'],
  })) }); t.after(m.unmount);
  const html = await renderDialog(m.b);
  for (const text of ['已记录本地工具', '读取任务快照', '查看预算与能力', '提出计划建议', '仅本地记录']) assert.ok(html.includes(text), text);
  assert.doesNotMatch(html, /localTools|capability_budget.read|plan.propose|工具已执行目标|正在思考/);
});

test('actual chat refuses malformed local tool names without advancing the original cursor', async t => {
  const m = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 20,
    timelineBeforeSequence: 20, timeline: [event('safe-before', 'mailbox_message', 20)] }) }); t.after(m.unmount);
  for (const localTools of [[], ['http.request'], ['snapshot.read', 'PRIVATE_CHAIN'],
    Array(5).fill('snapshot.read'), [{ name: 'snapshot.read', result: 'PRIVATE_RESULT' }], null]) {
    m.api.getNativeScanStatus = async () => snapshot(rootItem({}, { localTools }), 'A', { isIncremental: true });
    await m.b.loadStatus(20);
    assert.equal(m.b.statusSync.latestSequence.value, 20); assert.equal(m.b.state.value.timeline[0].id, 'safe-before');
    assert.ok(m.b.statusError.value); assert.doesNotMatch(await renderDialog(m.b), /PRIVATE_CHAIN|PRIVATE_RESULT|已记录本地工具/);
  }
});
