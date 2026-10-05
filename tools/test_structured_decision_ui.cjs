const assert = require('node:assert/strict');
const test = require('node:test');
const { mountTrace, round, snapshot, deferred, flush, renderTimeline, mountTask } = require('./decision_summary_harness.cjs');
const props = detail => ({ detail, busy: false, targetUrl: '*', taskStatus: 'scanning' });

test('real trace IPC selection renders bounded decisions as distinct facts and advisory sections', async t => {
  const m = await mountTrace({ getAgentTrace: async id => snapshot(id) }); t.after(m.unmount);
  const html = await m.render();
  for (const text of ['已记录决策摘要', '观察', '缺口', '下一步建议', '本轮账本用量', '费用说明建议', '风险与局限']) assert.ok(html.includes(text), text);
  assert.match(html, /映射回执已经保存|身份B尚缺证据/);
  assert.doesNotMatch(html, /decisionSummary|rootControl|requestHash|dispatch:web_executor/);
  assert.match(html, /Web 执行阶段/);
  assert.match(html, /建议不授予执行权限，也不代表已派发或完成/);
  assert.equal(m.calls.filter(([name]) => name === 'getAgentTrace').length, 1);
});

test('real result timeline shares safe decision presentation with the trace page', async () => {
  const value = snapshot('scan-a'); const before = JSON.stringify(value);
  const html = await renderTimeline(props(value));
  assert.match(html, /已记录决策摘要/); assert.match(html, /风险与局限/);
  assert.doesNotMatch(html, /"decisionSummary"|"usage"/);
  assert.equal(JSON.stringify(value), before);
});

test('model cost notes cannot replace original usage or become a monetary bill', async () => {
  const event = round({}, { decisionSummary: { schemaVersion: 1, observed: [], missing: [],
    suggestions: [], costNotes: ['预估费用 999 美元，仅建议'], risks: [] } });
  const html = await renderTimeline(props(snapshot('scan-a', event)));
  assert.match(html, /本轮账本用量/); assert.match(html, /100 tokens/); assert.match(html, /1 次模型请求/);
  assert.match(html, /费用说明建议/); assert.match(html, /未核验单价，未换算金额/);
  assert.doesNotMatch(html, /实际费用.{0,25}999/);
});

test('missing or inconsistent original usage stays unknown without hiding valid semantic facts', async () => {
  for (const usage of [undefined, { inputTokens: 80, cachedInputTokens: 90, outputTokens: 20, totalTokens: 100, modelRequests: 1 },
    { inputTokens: 80, cachedInputTokens: 20, outputTokens: 20, totalTokens: 101, modelRequests: 1 }]) {
    const html = await renderTimeline(props(snapshot('scan-a', round({}, { usage }))));
    assert.match(html, /映射回执已经保存/); assert.match(html, /用量记录无法核验/);
    assert.doesNotMatch(html, /100 tokens|已付费|免费|费用为 0/);
  }
});

test('unknown schema, private fields, bounds and truncated payloads never expose raw root response', async () => {
  const seed = JSON.parse(round().detail).decisionSummary;
  const malformed = [ { ...seed, schemaVersion: 2 }, { ...seed, reasoning: 'PRIVATE_CHAIN' },
    { ...seed, observed: Array(17).fill('secret-detail') }, { ...seed, risks: ['x'.repeat(513)] },
    { ...seed, missing: ['unsafe\nrecord'] } ];
  for (const decisionSummary of malformed) {
    const html = await renderTimeline(props(snapshot('scan-a', round({}, { decisionSummary, response: 'RAW_RESPONSE_PRIVATE' }))));
    assert.match(html, /摘要不可核验/);
    assert.doesNotMatch(html, /RAW_RESPONSE_PRIVATE|PRIVATE_CHAIN|secret-detail|已记录决策摘要/);
  }
  const html = await renderTimeline(props(snapshot('scan-a', round({ detailTruncated: true }))));
  assert.match(html, /摘要不可核验/); assert.doesNotMatch(html, /映射回执已经保存/);
});

test('payload extra bodies are never rendered and HTML-like semantic text is escaped', async () => {
  const payload = JSON.parse(round().detail);
  payload.decisionSummary.observed = ['<img src=x onerror=bad()>已核验记录'];
  payload.response = 'RAW_RESPONSE_PRIVATE'; payload.reasoning = 'PRIVATE_CHAIN';
  const html = await renderTimeline(props(snapshot('scan-a', round({}, payload))));
  assert.match(html, /&lt;img/); assert.doesNotMatch(html, /<img|RAW_RESPONSE_PRIVATE|PRIVATE_CHAIN/);
});

test('historical, non-root, foreign event identity and unsigned summary do not gain native decision status', async () => {
  const historical = snapshot('scan-a'); historical.summary.sourceAuthority = 'historical_external';
  assert.doesNotMatch(await renderTimeline(props(historical)), /已记录决策摘要/);
  for (const event of [round({ role: 'spa_api_mapper' }), round({ id: 'native:another-root:7' }),
    round({ eventType: 'mailbox_message' }), round({}, { advisoryOnly: false })]) {
    assert.doesNotMatch(await renderTimeline(props(snapshot('scan-a', event))), /已记录决策摘要/);
  }
});

test('actual selection fences late A to B to A snapshots and ignores injected notification content', async t => {
  const stale = deferred(); let calls = 0;
  const m = await mountTrace({ getAgentTrace: id => ++calls === 1 ? stale.promise : Promise.resolve(snapshot(id,
    round({}, { decisionSummary: { schemaVersion: 1, observed: ['CURRENT_FACT'], missing: [], suggestions: [], costNotes: [], risks: [] } }))) });
  t.after(m.unmount);
  await m.b.selectTrace('scan-b'); await m.b.selectTrace('scan-a');
  stale.resolve(snapshot('scan-a', round({}, { decisionSummary: { schemaVersion: 1, observed: ['STALE_FACT'], missing: [], suggestions: [], costNotes: [], risks: [] } })));
  await flush();
  m.emit({ sequence: 1, scanId: 'foreign', attemptNumber: 1, decisionSummary: { observed: ['FORGED_FACT'] } });
  await m.drain();
  const html = await m.render(); assert.match(html, /CURRENT_FACT/); assert.doesNotMatch(html, /STALE_FACT|FORGED_FACT/);
});

test('actual Board composable project switch fences pending decision read and unmount discards late data', async t => {
  const first = deferred(), last = deferred(); let call = 0;
  const m = mountTask(() => ++call === 1 ? first.promise : last.promise); t.after(m.unmount);
  const read = m.b.load('scan-a'); m.project.value = 2;
  first.resolve(snapshot('scan-a')); await read; assert.equal(m.b.detail.value, undefined);
  const tail = m.b.load('scan-a'); m.unmount(); last.resolve(snapshot('scan-a')); await tail;
  assert.equal(m.b.detail.value, undefined); assert.equal(m.b.busy.value, false);
});

test('result target filtering preserves task scope and empty lists never imply work completion', async () => {
  const value = snapshot('scan-a', round({}, { decisionSummary: { schemaVersion: 1, observed: [], missing: [], suggestions: [], costNotes: [], risks: [] } }));
  const visible = await renderTimeline(props(value));
  assert.match(visible, /未提供观察|未提供缺口|未提供下一步建议/);
  assert.doesNotMatch(visible, /无缺口|全部完成|正在思考|当前正在/);
  const hidden = await renderTimeline({ ...props(value), targetUrl: 'https://other.invalid' });
  assert.doesNotMatch(hidden, /已记录决策摘要/);
});


test('actual trace and result SFC show bounded names from original local step without raw results', async () => {
  const localStep = { calls: [
    { id: 'local-a', name: 'evidence.read', arguments: {}, result: { frozenEvidence: 'PRIVATE_RESULT' } },
    { id: 'local-b', name: 'snapshot.read', arguments: {}, result: { raw: 'PRIVATE_BODY' } },
  ] };
  const html = await renderTimeline(props(snapshot('scan-a', round({}, { localStep }))));
  for (const text of ['已记录本地工具', '读取冻结证据', '读取任务快照', '仅本地记录']) assert.ok(html.includes(text), text);
  assert.doesNotMatch(html, /PRIVATE_RESULT|PRIVATE_BODY|evidence.read|local-a|arguments|frozenEvidence/);
});

test('actual trace malformed local step cannot show completed tools or raw body', async () => {
  const call = { id: 'tool-a', name: 'snapshot.read', arguments: {}, result: {} };
  for (const localStep of [{ calls: [] }, { calls: Array(5).fill(call) },
    { calls: [{ ...call, name: 'http.request' }] }, { calls: [call], reasoning: 'PRIVATE_CHAIN' },
    { calls: [{ ...call, rawBody: 'PRIVATE_BODY' }] }, { calls: [call, call] }, null]) {
    const html = await renderTimeline(props(snapshot('scan-a', round({}, { localStep }))));
    assert.match(html, /摘要不可核验/); assert.doesNotMatch(html, /已记录本地工具|PRIVATE_CHAIN|PRIVATE_BODY/);
  }
});
