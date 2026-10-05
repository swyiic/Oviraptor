// Explicit recovery confirmation, request isolation and receipt validation.
const { assert, test, flush, state, mount, recoverable, recovered } = require('./harness.cjs');

test('recovery needs two explicit actions, pins scan and attempt, and never runs on polling', async (t) => {
  let dispatched = false;
  const h = await mount(() => dispatched ? state('claimed') : recoverable(), (...args) => {
    assert.deepEqual(args, ['scan-a', 1]); dispatched = true; return recovered();
  });
  t.after(h.unmount);
  assert.ok((await h.render()).includes('检查并恢复未派发任务'));
  await h.b.load();
  assert.equal(h.recoveryCalls.length, 0);
  await h.b.confirmRecovery();
  assert.equal(h.recoveryCalls.length, 0, 'confirm without opening the confirmation is inert');
  h.b.requestRecovery();
  assert.ok((await h.render()).includes('不会创建新尝试、扩大范围或启用主机测试'));
  assert.equal(h.recoveryCalls.length, 0);
  await h.b.confirmRecovery();
  assert.equal(h.recoveryCalls.length, 1);
  assert.ok((await h.render()).includes('不代表仍在运行或已经完成'));
  assert.equal(h.b.canRecover.value, false);
  await h.b.confirmRecovery();
  assert.equal(h.recoveryCalls.length, 1);
});

for (const receipt of ['claimed', 'legacy_unknown', 'invalid_receipt', undefined]) {
  test(`even a server hint cannot recover ${receipt ?? 'missing'} receipt`, async (t) => {
    const value = state(receipt);
    if (value.branches[0].dispatch) value.branches[0].dispatch.manualRecoveryAvailable = true;
    const h = await mount(() => value);
    t.after(h.unmount);
    assert.equal(h.b.canRecover.value, false);
    h.b.requestRecovery();
    await h.b.confirmRecovery();
    assert.equal(h.b.recoveryConfirmation.value, undefined);
    assert.ok(!(await h.render()).includes('检查并恢复未派发任务'));
  });
}

test('duplicate clicks cannot submit a second recovery while the first is pending', async (t) => {
  let finish;
  const h = await mount(recoverable, () => new Promise(resolve => { finish = resolve; }));
  t.after(h.unmount);
  h.b.requestRecovery();
  const pending = h.b.confirmRecovery();
  h.b.requestRecovery();
  await h.b.confirmRecovery();
  assert.equal(h.recoveryCalls.length, 1);
  assert.equal(h.b.recovering.value, true);
  finish(recovered());
  await pending;
  assert.equal(h.b.recovering.value, false);
});

for (const fail of [false, true]) {
  test(`late recovery ${fail ? 'failure' : 'success'} cannot affect another scan or attempt`, async (t) => {
    let finish, reject;
    let value = recoverable();
    const h = await mount(() => value, () => new Promise((resolve, no) => { finish = resolve; reject = no; }));
    t.after(h.unmount);
    h.b.requestRecovery();
    const pending = h.b.confirmRecovery();
    value = { ...recoverable(), scanId: 'scan-b', attemptNumber: 2 };
    h.props.scanId = 'scan-b';
    h.props.attempt = 2;
    await flush();
    const reads = h.calls.length;
    if (fail) reject(new Error('old failure')); else finish(recovered());
    await pending;
    assert.equal(h.b.recoveryMessage.value, '');
    assert.equal(h.b.recoveryError.value, '');
    assert.equal(h.b.state.value.scanId, 'scan-b');
    assert.equal(h.calls.length, reads, 'old recovery must not refresh a new view');
    assert.equal(h.b.recovering.value, false);
  });
}

test('confirmation becomes inert when polling observes a claim or attempt changes', async (t) => {
  let value = recoverable();
  const h = await mount(() => value);
  t.after(h.unmount);
  h.b.requestRecovery();
  value = state('claimed');
  await h.b.load();
  await h.b.confirmRecovery();
  assert.equal(h.recoveryCalls.length, 0);
  h.props.attempt = 2;
  await flush();
  assert.equal(h.b.recoveryConfirmation.value, undefined);
  assert.equal(h.b.canRecover.value, false);
});

test('ambiguous IPC failure refreshes receipts and does not retry a committed claim', async (t) => {
  let value = recoverable();
  const h = await mount(() => value, () => {
    value = state('claimed');
    throw new Error('<script>lost acknowledgement</script>');
  });
  t.after(h.unmount);
  h.b.requestRecovery();
  await h.b.confirmRecovery();
  assert.equal(h.recoveryCalls.length, 1);
  assert.equal(h.b.canRecover.value, false);
  const html = await h.render();
  assert.ok(html.includes('不会自动重试'));
  assert.ok(html.includes('&lt;script&gt;lost acknowledgement&lt;/script&gt;'));
  await h.b.load();
  assert.equal(h.recoveryCalls.length, 1);
});

test('unmounted view cannot display late recovery or restart status polling', async () => {
  let finish;
  const h = await mount(recoverable, () => new Promise(resolve => { finish = resolve; }));
  h.b.requestRecovery();
  const pending = h.b.confirmRecovery();
  h.unmount();
  const reads = h.calls.length;
  finish(recovered());
  await pending;
  assert.equal(h.calls.length, reads);
  assert.equal(h.timers.size, 0);
  assert.equal(h.b.recoveryMessage.value, '');
});

test('a mismatched recovery acknowledgement is not shown as success', async (t) => {
  const h = await mount(recoverable, () => recovered('wrong-scan', 2));
  t.after(h.unmount);
  h.b.requestRecovery();
  await h.b.confirmRecovery();
  assert.equal(h.b.recoveryMessage.value, '');
  assert.ok(h.b.recoveryError.value.includes('回执不匹配'));
  assert.equal(h.recoveryCalls.length, 1);
});
