// Administrative closure, unsettled history and linked task presentation.
const { assert, test, flush, state, mount } = require('./harness.cjs');

const administrativeState = () => ({ ...state('claimed'), status: 'paused', manualAdministrativeClosureAvailable: true });
const administrativePreview = () => ({ receipt: null, snapshotHash: 'a'.repeat(64), snapshot: {
  schemaVersion: 1, scanId: 'scan-a', attemptNumber: 1, previousStatus: 'paused', records: [],
} });
const administrativeReceipt = (scanId, attemptNumber, closureId, snapshotHash) => ({
  scanId, attemptNumber, closureId, snapshotHash, closedAt: '2026-01-01T00:00:00Z', previousStatus: 'paused',
  actor: 'local_operator', executionState: 'administratively_closed_unsettled', executionSettled: false,
  automaticReplayAllowed: false, requiresIndependentTask: true,
});
async function mountAdministrative(handlers, getStatus = administrativeState) {
  const h = await mount(getStatus, undefined, undefined, handlers);
  h.props.status = 'paused';
  await flush();
  return h;
}

test('administrative closure requires explicit preview and confirmation, and labels unsettled history', async (t) => {
  let value = administrativeState();
  const h = await mountAdministrative({ preview: administrativePreview, close: (...args) => {
    const receipt = administrativeReceipt(...args);
    value = { ...value, status: 'cancelled', manualAdministrativeClosureAvailable: false, administrativeClosure: receipt };
    return receipt;
  } }, () => value);
  t.after(h.unmount);
  await h.b.confirmAdministrativeClosure();
  assert.equal(h.administrativeCalls.length, 0);
  await h.b.requestAdministrativeClosure();
  assert.equal(h.administrativeCalls.length, 1);
  let html = await h.render();
  assert.ok(html.includes('此操作不能撤销'));
  await h.b.confirmAdministrativeClosure();
  assert.equal(h.administrativeCalls.length, 2);
  assert.equal(h.events.length, 1);
  assert.equal(h.b.administrativeConfirmation.value, undefined);
  html = await h.render();
  assert.ok(html.includes('不代表执行成功或费用结清'));
  assert.ok(html.includes('原始执行状态，不表示仍在运行'));
  assert.equal(h.b.pending.value.length, 0);
  assert.equal(h.b.canRecover.value, false);
  assert.equal(h.b.canClose.value, false);
});

test('ambiguous administrative closure retains the same UUID and snapshot on explicit retry', async (t) => {
  let attempts = 0;
  const h = await mountAdministrative({ preview: administrativePreview, close: (...args) => {
    if (++attempts === 1) throw new Error('acknowledgement lost');
    return administrativeReceipt(...args);
  } });
  t.after(h.unmount);
  await h.b.requestAdministrativeClosure();
  const frozen = { ...h.b.administrativeConfirmation.value };
  await h.b.confirmAdministrativeClosure();
  assert.deepEqual(h.b.administrativeConfirmation.value, frozen);
  assert.equal(h.events.length, 0);
  await h.b.load();
  assert.equal(attempts, 1, 'refresh is never an automatic write');
  await h.b.confirmAdministrativeClosure();
  assert.deepEqual(h.administrativeCalls[1], h.administrativeCalls[2]);
  assert.equal(h.events.length, 1);
});

test('administrative closure rejects mismatched or falsely settled receipts', async () => {
  for (const changes of [{scanId:'other'}, {attemptNumber:2}, {closureId:'other'}, {snapshotHash:'b'.repeat(64)},
    {previousStatus:'failed'}, {actor:'agent'}, {executionState:'completed'}, {executionSettled:true},
    {automaticReplayAllowed:true}, {requiresIndependentTask:false}, {closedAt:'invalid'}, {executionUnlocked:true}]) {
    const h = await mountAdministrative({ preview: administrativePreview,
      close: (...args) => ({...administrativeReceipt(...args),...changes}) });
    try {
      await h.b.requestAdministrativeClosure();
      const frozen = { ...h.b.administrativeConfirmation.value };
      await h.b.confirmAdministrativeClosure();
      assert.equal(h.events.length, 0);
      assert.equal(h.b.administrativeMessage.value, '');
      assert.deepEqual(h.b.administrativeConfirmation.value, frozen);
      assert.ok(h.b.administrativeError.value.includes('receipt_mismatch'));
    } finally { h.unmount(); }
  }
});

test('administrative closure ignores stale preview, stale completion and duplicate clicks', async (t) => {
  let finishPreview, finishClose;
  const h = await mountAdministrative({ preview: () => new Promise(resolve => { finishPreview = resolve; }),
    close: (...args) => new Promise(resolve => { finishClose = () => resolve(administrativeReceipt(...args)); }) });
  t.after(h.unmount);
  const preview = h.b.requestAdministrativeClosure();
  await h.b.requestAdministrativeClosure();
  assert.equal(h.administrativeCalls.length, 1);
  h.props.scanId = 'scan-b'; await flush();
  finishPreview(administrativePreview()); await preview;
  assert.equal(h.b.administrativeConfirmation.value, undefined);
  h.props.scanId = 'scan-a'; await flush();
  const nextPreview = h.b.requestAdministrativeClosure();
  finishPreview(administrativePreview()); await nextPreview;
  const closure = h.b.confirmAdministrativeClosure();
  await h.b.confirmAdministrativeClosure();
  assert.equal(h.administrativeCalls.length, 3);
  h.props.attempt = 2; await flush();
  finishClose(); await closure;
  assert.equal(h.events.length, 0);
  assert.equal(h.b.administrativeMessage.value, '');
});

test('administrative closure requires a matching preview and clears corrupt cached receipts', async (t) => {
  let failure = false;
  const h = await mountAdministrative({ preview: () => ({...administrativePreview(),snapshotHash:'invalid'}), close: administrativeReceipt },
    () => { if (failure) throw new Error('administrative_closure_event_integrity'); return administrativeState(); });
  t.after(h.unmount);
  await h.b.requestAdministrativeClosure();
  assert.equal(h.b.administrativeConfirmation.value, undefined);
  await h.b.confirmAdministrativeClosure();
  assert.equal(h.administrativeCalls.length, 1);
  failure = true;
  await h.b.load();
  assert.equal(h.b.state.value, undefined);
  assert.equal(h.b.canAdministrativelyClose.value, false);
});

test('closure handoff renders real source and successor without implying settlement and clears corrupted links', async (t) => {
  let failure = false;
  const h = await mount(() => {
    if (failure) throw new Error('closure_handoff_receipt_integrity');
    return { ...state('claimed'), status: 'cancelled',
      administrativeClosure: administrativeReceipt('scan-a',1,'closed','a'.repeat(64)),
      closureHandoff: { source: null, successor: { scanId: 'independent-task', scan: { status: 'draft' } } } };
  });
  t.after(h.unmount);
  const html = await h.render();
  assert.match(html, /independent-task/);
  assert.match(html, /准备独立关联任务/);
  assert.match(html, /关联不代表原执行结果已结清/);
  assert.equal(h.recoveryCalls.length, 0);
  failure = true; await h.b.load();
  assert.equal(h.b.state.value, undefined);
  assert.doesNotMatch(await h.render(), /independent-task/);
});
