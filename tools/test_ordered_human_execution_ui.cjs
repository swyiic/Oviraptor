// Actual current SFC and composable; IPC fixture is display protection only.
const { assert, test, renderDialog, directive, mount, deferred, flush } = require('./agent_dialog_harness.cjs');
const clone = row => JSON.parse(JSON.stringify(row));
function fresh(version = 3) {
  const row = { ...directive('A', 'drafted'), intent: 'agent_proposal_request',
    text: '@investigator 然后 @mapper 评估已有证据', safeExecutionText: '只评估原证据',
    requestedRoles: ['deep_investigator', 'spa_api_mapper'], validationResult: 'valid', coordinatorDecision: 'accept',
    estimatedTokens: 8000, estimatedRequests: 2,
    reasonCodes: ['read_only_change_within_frozen_plan', `ordered_readonly_assessment_v${version}`,
      version === 3 ? 'ordered_original_dispatch_checks_required' : 'proposal_ordered_execution_not_connected'] };
  row.readonlyAssessmentPlan = { schemaVersion: version, purpose: 'human_readonly_assessment', bindingHash: 'a'.repeat(64),
    planHash: 'b'.repeat(64), dispatchState: version === 3 ? 'requires_dispatch_checks' : 'not_connected',
    totalTokenCeiling: 8000, totalModelRequests: 2, targetRequests: 0,
    actions: row.requestedRoles.map((role, index) => ({actionId: (index ? 'd' : 'c').repeat(64), order: index + 1, role,
      tokenCeiling: 4000, modelRequests: 1, maxOutputTokens: 512, targetRequests: 0,
      previousActionId: index ? 'c'.repeat(64) : null,
      requiredPreviousState: index ? 'valid_advisory_receipt' : 'frozen_original_evidence', advisoryOnly: true, executionState: 'not_started'})) };
  return row;
}
function itemFor(view) {
  const item = {id:'ordered-directive',sequence:10,timestamp:'2026-10-03 10:00:00',eventType:'user_directive',
    fromRole:'operator',fromRunId:'',toRole:'coordinator',toRunId:'root-A',messageKind:'human_directive',correlationId:'',
    assignmentId:'',evidenceRevision:0,threadKey:'team',targetKey:'https://scope.test',deliveryState:'persisted',ackState:'assigned',status:'assigned',summary:'原有序请求'};
  const plan=fresh().readonlyAssessmentPlan;
  const receipt={schemaVersion:1,kind:'ordered_readonly_assessment_receipt',receiptId:'receipt-one',directiveId:item.id,sourceDraftId:'draft-one',
    draftRevision:1,draftHash:'f'.repeat(64),scanId:view.scanId,attemptNumber:view.attemptNumber,rootRunId:item.toRunId,targetKey:item.targetKey,
    bindingHash:plan.bindingHash,planHash:plan.planHash,actionId:plan.actions[0].actionId,order:1,role:plan.actions[0].role,
    assignmentId:'asg-one',childRunId:'child-one',leaseAttemptId:'original-lease',workerId:'original-worker',requestHash:'1'.repeat(64),
    responseHash:'2'.repeat(64),modelEventSequence:5,usage:{inputTokens:10,cachedInputTokens:0,outputTokens:10,totalTokens:20,modelRequests:1},
    usageReported:true,resultMessageId:'result-one',resultPayloadHash:'3'.repeat(64),predecessor:null,outcome:'valid_advisory',
    assessment:{summary:'原始已保存评估',suggestions:['只读建议'],limitations:['尚未访问目标'],valid:true},advisoryOnly:true,coverageVerified:false,
    targetRequests:0,independentReviewApproved:false,reviewerReceipt:null};
  item.orderedAssessmentExecution={schemaVersion:1,directiveId:item.id,sourceDraftId:receipt.sourceDraftId,draftHash:receipt.draftHash,planHash:plan.planHash,
    scanId:view.scanId,attemptNumber:view.attemptNumber,rootRunId:item.toRunId,targetKey:item.targetKey,threadKey:'team',state:'assigned',
    completedAssessments:1,plannedAssessments:2,advisoryOnly:true,coverageVerified:false,independentReviewApproved:false,
    actions:[{actionId:plan.actions[0].actionId,order:1,role:plan.actions[0].role,state:'completed',assignmentId:'asg-one',childRunId:'child-one',receipt},
      {actionId:plan.actions[1].actionId,order:2,role:plan.actions[1].role,state:'not_started',assignmentId:null,childRunId:null,receipt:null}]};
  return item;
}

test('ordered execution v3 actual creation accepts the frozen dispatch confirmation instead of rejecting valid receipt', async t => {
  const h=await mount({draftScanDirective:async()=>fresh()});t.after(h.unmount);
  h.b.draft.value='original submitted input';await h.b.send();
  assert.equal(h.b.error.value,'');assert.equal(h.b.draft.value,'');assert.equal(h.b.state.value.timeline.length,0);
});

test('ordered execution v3 actual card discloses sequential model dispatch and original authority constraints', async t => {
  const h=await mount();t.after(h.unmount);h.b.state.value.directiveDrafts=[fresh()];
  assert.equal(h.b.canConfirmDirective(fresh()),true);
  const html=await renderDialog(h.b);
  assert.match(html,/确认授权按所示顺序逐项派发只读模型评估/);assert.match(html,/独立 Reviewer 未审核/);
  assert.match(html,/原任务权限、预算和有效租约/);assert.doesNotMatch(html,/有序角色执行尚未接线/);
  assert.equal((html.match(/4000 tokens/g)||[]).length,2);
});

test('ordered execution v2 actual card preserves explicitly blocked prior confirmation', async t => {
  const h=await mount();t.after(h.unmount);h.b.state.value.directiveDrafts=[fresh(2)];
  const html=await renderDialog(h.b);assert.match(html,/有序角色执行尚未接线/);assert.doesNotMatch(html,/确认授权按所示顺序逐项派发/);
  assert.equal(h.b.state.value.directiveDrafts[0].readonlyAssessmentPlan.schemaVersion,2);
});

test('ordered execution actual timeline displays only paid first assessment and keeps the second not dispatched', async t => {
  const h=await mount();t.after(h.unmount);const item=itemFor(h.b.state.value);h.b.state.value.timeline=[item];
  const html=await renderDialog(h.b);
  assert.match(html,/有序只读评估/);assert.match(html,/原始已保存评估/);assert.match(html,/实际已报告用量/);
  assert.match(html,/20 tokens/);assert.match(html,/尚未派发/);assert.match(html,/独立 Reviewer 未审核/);
  assert.doesNotMatch(html,/已完成全部任务|正在思考|已验证漏洞|\$|原响应|RAW_PRIVATE/);
  assert.equal(item.orderedAssessmentExecution.actions[1].receipt,null);
});

test('ordered execution actual consumer rejects scope fee and fake review or ordered predecessor damage', async t => {
  const h=await mount();t.after(h.unmount);
  assert.equal(typeof h.b.orderedAssessmentExecution,'function','actual consumer is required');
  for (const mutate of [
    row=>row.scanId='B',row=>row.attemptNumber++,row=>row.rootRunId='foreign-root',row=>row.threadKey='other-thread',
    row=>row.actions[0].receipt.workerId='',row=>row.actions[0].receipt.usage.modelRequests=0,
    row=>row.actions[0].receipt.usage.totalTokens=0,row=>row.actions[0].receipt.independentReviewApproved=true,
    row=>row.actions[0].receipt.reviewerReceipt={approved:true},row=>row.actions[1].state='completed',
    row=>row.completedAssessments=2,
  ]) {
    const item=itemFor(h.b.state.value);mutate(item.orderedAssessmentExecution);
    assert.equal(h.b.orderedAssessmentExecution(item,h.b.state.value),null);
    h.b.state.value.timeline=[item];const html=await renderDialog(h.b);
    assert.doesNotMatch(html,/原始已保存评估|20 tokens/);assert.match(html,/有序评估回执无法核实/);
  }
});

test('ordered execution unknown actual receipt state does not become paid or completed', async t => {
  const h=await mount();t.after(h.unmount);const item=itemFor(h.b.state.value);const value=item.orderedAssessmentExecution;
  value.actions[0].state='outcome_unknown';value.actions[0].receipt=null;value.completedAssessments=0;
  h.b.state.value.timeline=[item];const html=await renderDialog(h.b);
  assert.match(html,/结果未知，未自动重试/);assert.doesNotMatch(html,/实际已报告用量|原始已保存评估|评估已保存/);
});

test('ordered execution v3 malformed authority cannot consume edits or submit a different frozen version', async t => {
  let calls=0;const h=await mount({draftScanDirective:async()=>{const row=fresh();row.readonlyAssessmentPlan.dispatchState='not_connected';return row;},confirmScanDirective:async()=>{calls++;}});t.after(h.unmount);
  h.b.draft.value='preserve original';await h.b.send();assert.equal(h.b.draft.value,'preserve original');assert.match(h.b.error.value,/无法核实/);
  const row=fresh();h.b.state.value.directiveDrafts=[row];const changed=clone(row);changed.readonlyAssessmentPlan.schemaVersion=2;
  await h.b.confirmDirective(changed);assert.equal(calls,0);assert.equal(row.readonlyAssessmentPlan.schemaVersion,3);
});

test('ordered execution v3 late A B A parse keeps current edit and cannot append stale state', async t => {
  const pending=deferred();const h=await mount({draftScanDirective:()=>pending.promise});t.after(h.unmount);
  h.b.draft.value='old A';const sending=h.b.send();h.b.scanId.value='B';await flush();h.b.scanId.value='A';await flush();
  h.b.draft.value='new A';pending.resolve(fresh());await sending;
  assert.equal(h.b.draft.value,'new A');assert.equal(h.b.state.value.timeline.length,0);assert.equal(h.b.sending.value,false);
});


test('ordered execution actual card shows closed before dispatch while retaining the first paid assessment', async t => {
  const h=await mount();t.after(h.unmount);const item=itemFor(h.b.state.value);
  item.status='deferred';item.orderedAssessmentExecution.state='deferred';
  item.taskClosure={fromStatus:'assigned',disposition:'not_applied',rootTerminalCode:'incomplete',requiresReconciliation:false,automaticRetry:false,closedAt:'2026-10-03 22:00:00'};
  Object.assign(item.orderedAssessmentExecution.actions[1],{state:'cancelled_before_dispatch',assignmentId:'asg-two',childRunId:'child-two'});
  h.b.state.value.timeline=[item];const html=await renderDialog(h.b);
  assert.match(html,/已关闭，未派发/);assert.match(html,/原始已保存评估/);assert.match(html,/20 tokens/);
  assert.doesNotMatch(html,/等待原权限核对|已完成全部任务/);
  assert.equal(item.orderedAssessmentExecution.actions[1].receipt,null);
});

test('ordered execution actual consumer refuses closed action without the original terminal disposition', async t => {
  const h=await mount();t.after(h.unmount);
  for(const mutate of [item=>{},item=>{item.status='deferred';item.orderedAssessmentExecution.state='deferred';},item=>{
    item.status='deferred';item.orderedAssessmentExecution.state='deferred';item.taskClosure={disposition:'reconciliation_required',requiresReconciliation:true,automaticRetry:false};
  },item=>{
    item.status='deferred';item.orderedAssessmentExecution.state='deferred';item.taskClosure={disposition:'not_applied',requiresReconciliation:false,automaticRetry:false};
  }]) {
    const item=itemFor(h.b.state.value);Object.assign(item.orderedAssessmentExecution.actions[1],{state:'cancelled_before_dispatch',assignmentId:'asg-two',childRunId:'child-two'});
    mutate(item);assert.equal(h.b.orderedAssessmentExecution(item,h.b.state.value),null);
    h.b.state.value.timeline=[item];const html=await renderDialog(h.b);assert.doesNotMatch(html,/已关闭，未派发|原始已保存评估/);
  }
});
