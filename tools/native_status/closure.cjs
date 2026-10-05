// Never-dispatched closure and mutual exclusion with recovery.
const { assert, test, flush, state, mount, recoverable, recovered } = require('./harness.cjs');

const closable = () => {
  const value = recoverable();
  value.branches[0].dispatch.manualClosureAvailable = true;
  return value;
};
const closureReceipt = () => ({ scanId: 'scan-a', attemptNumber: 1, closureId: 'closure-a',
  closedAt: '2026-09-26T12:00:00Z', executionState: 'closed_without_dispatch', automaticReplayAllowed: false });
const closedState = () => {
  const value = state('never_claimed');
  value.status = 'cancelled';
  value.branches[0].status = 'failed';
  value.branches[0].report = { code: 'operator_closed_before_dispatch' };
  return value;
};

test('closure needs two explicit actions and never dispatches or starts a new attempt', async (t) => {
  let value = closable();
  const h = await mount(() => value, undefined, (...args) => {
    assert.deepEqual(args, ['scan-a', 1]); value = closedState(); return closureReceipt();
  });
  t.after(h.unmount);
  await h.b.load();
  await h.b.confirmClosure();
  assert.equal(h.closureCalls.length, 0);
  h.b.requestClosure();
  assert.ok((await h.render()).includes('不会自动重新扫描'));
  await h.b.confirmClosure();
  assert.equal(h.closureCalls.length, 1);
  assert.equal(h.recoveryCalls.length, 0);
  assert.deepEqual(h.events, [['scan-a', 1]]);
  assert.equal(h.b.canClose.value, false);
  assert.equal(h.b.canRecover.value, false);
  assert.equal(h.timers.size, 0, 'terminal receipt does not schedule short polling');
  assert.equal(h.live.intervals, 1, 'mounted view retains low-frequency missed-event reconciliation');
  assert.ok((await h.render()).includes('未执行·人工结束'));
  await h.b.confirmClosure();
  assert.equal(h.closureCalls.length, 1);
});

for (const receipt of ['claimed', 'legacy_unknown', 'invalid_receipt', undefined]) {
  test(`closure rejects ${receipt ?? 'missing'} dispatch even if hint is true`, async (t) => {
    const value = state(receipt);
    if (value.branches[0].dispatch) value.branches[0].dispatch.manualClosureAvailable = true;
    const h = await mount(() => value);
    t.after(h.unmount);
    assert.equal(h.b.canClose.value, false);
    h.b.requestClosure();
    await h.b.confirmClosure();
    assert.equal(h.closureCalls.length, 0);
    assert.equal(h.b.closureConfirmation.value, undefined);
  });
}

for (const action of ['closure', 'recovery']) {
  test(`${action} is single flight and excludes the other action`, async (t) => {
    let finish;
    const suspended = () => new Promise(resolve => { finish = resolve; });
    const h = await mount(closable, suspended, suspended);
    t.after(h.unmount);
    h.b.requestRecovery();
    h.b.requestClosure();
    assert.equal(h.b.recoveryConfirmation.value, undefined);
    let pending;
    if (action === 'closure') pending = h.b.confirmClosure();
    else { h.b.requestRecovery(); assert.equal(h.b.closureConfirmation.value, undefined); pending = h.b.confirmRecovery(); }
    h.b.requestClosure(); h.b.requestRecovery();
    await h.b.confirmClosure(); await h.b.confirmRecovery();
    assert.equal(h.closureCalls.length + h.recoveryCalls.length, 1);
    finish(action === 'closure' ? closureReceipt() : recovered());
    await pending;
  });
}

for (const fail of [false,true]) {
  test(`late closure ${fail ? 'failure' : 'success'} cannot change another attempt`, async (t) => {
    let finish, reject, value = closable();
    const h = await mount(() => value, undefined, () => new Promise((yes,no) => {finish = yes; reject = no;}));
    t.after(h.unmount);
    h.b.requestClosure();
    const pending = h.b.confirmClosure();
    value = {...closable(),scanId:'scan-b',attemptNumber:2};
    h.props.scanId = 'scan-b'; h.props.attempt = 2;
    await flush();
    const reads = h.calls.length;
    if (fail) reject(new Error('old failure')); else finish(closureReceipt());
    await pending;
    assert.equal(h.b.closureMessage.value, '');
    assert.equal(h.b.closureError.value, '');
    assert.equal(h.events.length, 0);
    assert.equal(h.calls.length, reads);
    assert.equal(h.b.closing.value, false);
  });
}

test('closure confirmation becomes inert when a claim or new attempt is observed', async (t) => {
  let value = closable();
  const h = await mount(() => value);
  t.after(h.unmount);
  h.b.requestClosure();
  value = state('claimed');
  await h.b.load();
  await h.b.confirmClosure();
  assert.equal(h.closureCalls.length, 0);
  h.props.attempt = 2;
  await flush();
  assert.equal(h.b.closureConfirmation.value, undefined);
});

test('ambiguous closure only refreshes receipts, never retries or starts work', async (t) => {
  let value = closable();
  const h = await mount(() => value, undefined, () => {
    value = closedState(); throw new Error('<script>lost closure acknowledgement</script>');
  });
  t.after(h.unmount);
  h.b.requestClosure(); await h.b.confirmClosure();
  assert.equal(h.closureCalls.length, 1);
  assert.equal(h.b.canClose.value, false);
  assert.equal(h.events.length, 0);
  const html = await h.render();
  assert.ok(html.includes('不会自动重试或启动扫描'));
  assert.ok(html.includes('&lt;script&gt;lost closure acknowledgement&lt;/script&gt;'));
  await h.b.load();
  assert.equal(h.closureCalls.length, 1);
});

test('unmounted view ignores late closure, events and polling', async () => {
  let finish;
  const h = await mount(closable, undefined, () => new Promise(resolve => {finish = resolve;}));
  h.b.requestClosure(); const pending = h.b.confirmClosure();
  h.unmount(); const reads = h.calls.length;
  finish(closureReceipt()); await pending;
  assert.equal(h.calls.length, reads);
  assert.equal(h.events.length, 0);
  assert.equal(h.timers.size, 0);
});

test('invalid closure acknowledgements never appear successful', async () => {
  for (const changes of [{scanId:'wrong'}, {attemptNumber:2}, {closureId:''}, {closedAt:'invalid'},
    {executionState:'submitted'}, {automaticReplayAllowed:true}]) {
    const h = await mount(closable, undefined, () => ({...closureReceipt(),...changes}));
    try {
      h.b.requestClosure(); await h.b.confirmClosure();
      assert.equal(h.b.closureMessage.value, '');
      assert.ok(h.b.closureError.value.includes('回执不匹配'));
      assert.equal(h.events.length, 0);
    } finally {h.unmount();}
  }
});
