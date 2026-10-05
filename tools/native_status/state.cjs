// Read-only status, dispatch evidence and stale response presentation.
const { assert, test, flush, state, mount } = require('./harness.cjs');

test('source run cards identify actual repository and analysis roles', async (t) => {
  const h = await mount(() => ({ ...state('claimed'), timeline: ['repo_mapper', 'source_analyst'].map((role, index) => ({
    id: `source-${index}`, eventType: 'agent_run', fromRole: role, status: 'completed_with_gaps',
    summary: '独立复核尚未完成',
  })) }));
  t.after(h.unmount);
  const html = await h.render();
  assert.match(html, /仓库梳理/);
  assert.match(html, /源码分析/);
  assert.match(html, /独立复核尚未完成/);
});

test('pausing is not rendered as paused and offers no recovery or closure', async (t) => {
  const pending = { ...state('claimed'), status: 'pausing', stopDiagnostic: {
    category: 'active', code: 'pause_waiting_for_quiescence', stage: 'branch',
    nextAction: 'wait_for_worker_exit', obligations: [], automaticResumeAllowed: false,
  } };
  const h = await mount(() => pending);
  t.after(h.unmount);
  h.props.status = 'pausing';
  await flush();
  const html = await h.render();
  assert.ok(html.includes('暂停中，等待执行线程退出与清理确认'));
  assert.ok(html.includes('不能提前恢复或重放请求'));
  assert.ok(!html.includes('已暂停'));
  assert.equal(h.b.canRecover.value, false);
  assert.equal(h.b.canClose.value, false);
});

for (const [phase, label] of [
  ['branch_admission', '未执行·准入失败'], ['thread_spawn', '未执行·线程未启动'],
]) {
  test(`renders proven unexecuted ${phase} without cancelling a sibling or offering replay`, async (t) => {
    const value = state('never_claimed');
    value.branches[0].status = 'failed';
    value.branches[0].report = {code:'workbench_admission_rejected', failurePhase:phase, executionStarted:false};
    value.branches.push({branch:'source',status:'pending',checkpoint:'independent source',report:{},updatedAt:'',
      dispatch:{state:'claimed',claimedAt:'fixture',automaticReplayAllowed:false}});
    const h = await mount(() => value);
    t.after(h.unmount);
    const html = await h.render();
    assert.ok(html.includes(label));
    assert.ok(html.includes('本分支未开始执行，不会自动重试'));
    assert.ok(html.includes('仍在等待：'));
    assert.ok(html.includes('independent source'));
    assert.ok(!html.includes('检查并恢复未派发任务'));
    assert.equal(h.b.canRecover.value,false);
    assert.equal(h.recoveryCalls.length,0);
    assert.equal(h.closureCalls.length,0);
  });
}

test('unexecuted failure badge requires consistent phase, report, status and dispatch proof', async (t) => {
  const value = state('never_claimed');
  value.branches[0].status = 'failed';
  const report = {code:'workbench_admission_rejected',failurePhase:'branch_admission',executionStarted:false};
  value.branches[0].report = report;
  const h = await mount(() => value);
  t.after(h.unmount);
  for (const change of [
    {dispatch:{state:'claimed'}}, {dispatch:{state:'legacy_unknown'}}, {dispatch:{state:'invalid_receipt'}},
    {dispatch:undefined}, {status:'pending'}, {report:{...report,executionStarted:true}},
    {report:{...report,executionStarted:'false'}}, {report:{...report,code:'unknown'}},
    {report:{...report,failurePhase:'execution'}}, {report:null},
  ]) {
    const modified = {...value,branches:[{...value.branches[0],...change}]};
    h.b.state.value = modified;
    await flush();
    const html = await h.render();
    assert.ok(!html.includes('未执行·准入失败'),JSON.stringify(change));
    assert.ok(!html.includes('本分支未开始执行，不会自动重试'),JSON.stringify(change));
  }
  assert.equal(h.recoveryCalls.length,0);
  assert.equal(h.closureCalls.length,0);
});

for (const [receipt, text] of [
  ['never_claimed', '尚未取得执行权；不会自动恢复派发'],
  ['claimed', '不代表仍在运行或已经完成，不能直接重放'],
  ['legacy_unknown', '历史执行缺少派发凭据'],
  ['invalid_receipt', '派发凭据异常'],
  [undefined, '历史执行缺少派发凭据'],
]) {
  test(`renders ${receipt ?? 'old API'} receipt without execution or replay`, async (t) => {
    const h = await mount(() => state(receipt));
    t.after(h.unmount);
    const html = await h.render();
    assert.ok(html.includes(text));
    assert.ok(html.includes('&lt;script&gt;not executable&lt;/script&gt;'));
    assert.equal(h.calls.length, 1);
    await h.b.load();
    assert.equal(h.calls.length, 2);
    assert.equal(h.timers.size, 0, 'status updates use committed events, not short polling');
  });
}

test('late receipt from previous attempt cannot replace current status', async (t) => {
  let resolveOld;
  const old = new Promise((resolve) => { resolveOld = resolve; });
  let count = 0;
  const h = await mount(() => ++count === 1 ? old : { ...state('never_claimed'), attemptNumber: 2 });
  t.after(h.unmount);
  h.props.attempt = 2;
  await flush();
  resolveOld(state('claimed'));
  await flush();
  assert.equal(h.b.state.value.attemptNumber, 2);
  assert.ok((await h.render()).includes('尚未取得执行权'));
  h.unmount();
  assert.equal(h.timers.size, 0);
});


test('paused scan identifies retained running rows as records and shows aggregate cost once', async (t) => {
  const value = { ...state('claimed'), status: 'paused', llmRequests: 7, totalTokens: 140,
    branches: [state('claimed').branches[0], { branch: 'source', status: 'pending', checkpoint: 'Source retained', updatedAt: '' }],
    timeline: [{ id: 'original-root', eventType: 'agent_run', fromRole: 'coordinator', status: 'running', summary: 'Original fee remains reserved' }],
    stopDiagnostic: { category: 'soft', code: 'paused_requires_review', stage: 'financial', nextAction: 'review_before_manual_resume', obligations: [] },
  };
  const h = await mount(() => value);
  t.after(h.unmount);
  h.props.status = 'paused';
  await flush();
  const html = await h.render();
  assert.ok(html.includes('任务已暂停；下方智能体状态是保留的执行记录，不表示仍在工作。'));
  assert.ok(html.includes('原执行状态：'));
  assert.ok(html.includes('Original fee remains reserved'));
  assert.equal((html.match(/模型请求/g) || []).length, 1, 'aggregate request total must not appear once per branch');
  assert.equal(h.recoveryCalls.length, 0);
  assert.equal(h.closureCalls.length, 0);
  assert.equal(h.b.canRecover.value, false);
});
