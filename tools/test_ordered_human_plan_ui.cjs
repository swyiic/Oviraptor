// Actual SFC/composable with an IPC transport harness; no runtime or billing
// claim is made from these frontend fixtures.
const { assert, test, renderDialog, deferred, flush, directive, mount } = require('./agent_dialog_harness.cjs');
function fresh() {
  const row = { ...directive('A', 'drafted'), intent: 'agent_proposal_request',
    text: '@investigator 然后 @mapper 评估已有证据', safeExecutionText: '只评估原证据',
    requestedRoles: ['deep_investigator', 'spa_api_mapper'], validationResult: 'valid', coordinatorDecision: 'accept',
    estimatedTokens: 8000, estimatedRequests: 2,
    reasonCodes: ['read_only_change_within_frozen_plan', 'ordered_readonly_assessment_v2', 'proposal_ordered_execution_not_connected'] };
  row.readonlyAssessmentPlan = { schemaVersion: 2, purpose: 'human_readonly_assessment', bindingHash: 'a'.repeat(64),
    planHash: 'b'.repeat(64), dispatchState: 'not_connected', totalTokenCeiling: 8000, totalModelRequests: 2, targetRequests: 0,
    actions: row.requestedRoles.map((role,index) => ({ actionId: (index ? 'd' : 'c').repeat(64), order: index + 1, role,
      tokenCeiling: 4000, modelRequests: 1, maxOutputTokens: 512, targetRequests: 0,
      previousActionId: index ? 'c'.repeat(64) : null,
      requiredPreviousState: index ? 'valid_advisory_receipt' : 'frozen_original_evidence',
      advisoryOnly: true, executionState: 'not_started' })) };
  return row;
}
const clone = row => JSON.parse(JSON.stringify(row));

test('ordered human plan actual draft card displays per-action costs, order, total and truthful blocked state', async t => {
  const h = await mount(); t.after(h.unmount);
  const row = fresh(); const saved = JSON.stringify(row); h.b.state.value.directiveDrafts = [row];
  const html = await renderDialog(h.b);
  assert.match(html, /有序评估预算/); assert.equal((html.match(/4000 tokens/g) || []).length, 2);
  assert.match(html, /合计上限 8000 tokens/); assert.match(html, /2 次模型请求/);
  assert.match(html, /实际费用以原调用账本为准/); assert.match(html, /有序角色执行尚未接线/);
  assert.match(html, /需前一步有效评估回执/);
  assert.doesNotMatch(html, /正在思考|正在派发|已执行评估|成本为 0|已完成全部任务/);
  assert.equal(JSON.stringify(row), saved);
});

test('ordered human plan confirm compares the complete displayed frozen plan instead of only draft revision/hash', async t => {
  let calls = 0;
  const h = await mount({ confirmScanDirective: async () => { calls++; } }); t.after(h.unmount);
  const row = fresh(); h.b.state.value.directiveDrafts = [row];
  const altered = clone(row); altered.readonlyAssessmentPlan.planHash = 'e'.repeat(64);
  await h.b.confirmDirective(altered);
  assert.equal(calls, 0, 'same native draft identity cannot submit a different visible plan');
  assert.match(h.b.error.value, /草案已变化|无法核实/);
  assert.equal(row.readonlyAssessmentPlan.planHash, 'b'.repeat(64));
});

test('ordered human plan malformed parse receipts preserve composer and cannot enter reviewable pending state', async t => {
  for (const mutate of [
    row => { row.readonlyAssessmentPlan.actions[1].previousActionId = null; },
    row => { row.readonlyAssessmentPlan.actions[0].tokenCeiling = 1; },
    row => { row.readonlyAssessmentPlan.totalModelRequests = 1; },
    row => { row.readonlyAssessmentPlan.actions[1].executionState = 'completed'; },
    row => { row.readonlyAssessmentPlan.response = 'RAW_MODEL_PRIVATE'; },
    row => { row.readonlyAssessmentPlan.actions[0].role = 'evidence_reviewer'; },
    row => { delete row.readonlyAssessmentPlan; },
    row => { row.reasonCodes = []; },
  ]) {
    const value = fresh(); mutate(value);
    const h = await mount({ draftScanDirective: async () => value });
    try {
      h.b.draft.value = 'keep original input'; await h.b.send();
      assert.equal(h.b.draft.value, 'keep original input');
      assert.match(h.b.error.value, /草案回执无法核实/);
      assert.equal(h.b.state.value.timeline.length, 0);
    } finally { h.unmount(); }
  }
});

test('ordered human plan a valid typed creation receipt settles only the submitted edit', async t => {
  const pending = deferred(); const h = await mount({ draftScanDirective: () => pending.promise }); t.after(h.unmount);
  h.b.draft.value = fresh().text; const sending = h.b.send(); h.b.draft.value = 'next unsent thought';
  pending.resolve(fresh()); await sending;
  assert.equal(h.b.draft.value, 'next unsent thought'); assert.equal(h.b.error.value, '');
  assert.equal(h.b.state.value.timeline.length, 0, 'creation receipt does not prove a dispatched action');
});

test('ordered human plan old v1 blocked plan remains reviewable without v2 upgrade', async t => {
  let calls = 0; const h = await mount({ confirmScanDirective: async () => { calls++; } }); t.after(h.unmount);
  const row = fresh(); delete row.readonlyAssessmentPlan;
  row.reasonCodes = ['read_only_change_within_frozen_plan']; row.requestedRoles = ['spa_api_mapper', 'deep_investigator'];
  row.estimatedTokens = 4000; row.estimatedRequests = 1; h.b.state.value.directiveDrafts = [row];
  assert.equal(h.b.canConfirmDirective(row), true);
  await h.b.confirmDirective(row); assert.equal(calls, 1);
  assert.equal(row.readonlyAssessmentPlan, undefined); assert.equal(row.estimatedTokens, 4000);
});

test('ordered human plan a late A to B to A parse cannot consume a new edit or inject plan state', async t => {
  const pending = deferred(); const h = await mount({ draftScanDirective: () => pending.promise }); t.after(h.unmount);
  h.b.draft.value = 'first A edit'; const sending = h.b.send();
  h.b.scanId.value = 'B'; await flush(); h.b.scanId.value = 'A'; await flush();
  h.b.draft.value = 'new A edit'; pending.resolve(fresh()); await sending;
  assert.equal(h.b.draft.value, 'new A edit'); assert.equal(h.b.error.value, '');
  assert.equal(h.b.state.value.timeline.length, 0); assert.equal(h.b.sending.value, false);
});
