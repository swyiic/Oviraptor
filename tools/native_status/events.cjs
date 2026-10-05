// Exercise production committed-refresh and SFC; only IPC, events and clocks are fake.
const { assert, test, flush, state, mount } = require('./harness.cjs');
const hint = (scanId = 'scan-a', attemptNumber = 1) => ({ scanId, attemptNumber, sequence: 1 });
async function open(read = () => state('claimed'), options) {
  const h = await mount(read, undefined, undefined, undefined, options);
  h.live.fire(); await flush();
  return h;
}

test('status events validate scope, coalesce bursts and never display payload content', async t => {
  const h = await open(); t.after(h.unmount);
  const before = h.calls.length;
  for (const payload of [null, {}, hint('other'), hint('scan-a', 2), { ...hint(), sequence: 0 }]) h.live.emit(payload);
  assert.equal(h.live.timers, 0);
  for (let i = 0; i < 20; i++) h.live.emit({ ...hint(), summary: 'untrusted-event' });
  assert.equal(h.live.timers, 1);
  h.live.fire(); await flush();
  assert.equal(h.calls.length, before + 1);
  assert.equal(h.timers.size, 0, 'no three-second status poll');
  assert.ok(!(await h.render()).includes('untrusted-event'));
  assert.equal(h.recoveryCalls.length + h.closureCalls.length + h.administrativeCalls.length, 0);
});

test('an event during manual reading triggers one follow-up after the read finishes', async t => {
  let resolve, reads = 0;
  const h = await open(() => ++reads === 3 ? new Promise(r => { resolve = r; }) : state('claimed'));
  t.after(h.unmount);
  const manual = h.b.load();
  h.live.emit(hint()); h.live.fire(); await flush();
  assert.equal(reads, 3);
  resolve(state('claimed')); await manual; await flush();
  h.live.fire(); await flush();
  assert.equal(reads, 4);
});

for (const mismatch of [{ scanId: 'other' }, { attemptNumber: 2 }]) {
  test(`mismatched snapshot is not displayed: ${JSON.stringify(mismatch)}`, async t => {
    const h = await open(() => ({ ...state('claimed'), ...mismatch })); t.after(h.unmount);
    assert.equal(h.b.state.value, undefined);
    assert.match(h.b.error.value, /任务或尝试编号不匹配/);
    assert.equal(h.b.canRecover.value, false);
    assert.equal(h.b.canClose.value, false);
  });
}

test('hidden view defers event reads and catches up on visibility', async t => {
  const h = await open(); t.after(h.unmount); const before = h.calls.length;
  h.live.visibility(true); h.live.emit(hint()); h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.calls.length, before);
  h.live.visibility(false); h.live.fire(); await flush();
  assert.equal(h.calls.length, before + 1);
});

test('subscription failure is visible without raw error and fallback still reads', async t => {
  const h = await open(undefined, { failure: true }); t.after(h.unmount);
  assert.match(await h.render(), /实时状态通知暂不可用/);
  assert.ok(!(await h.render()).includes('private-registration-error'));
  const before = h.calls.length; h.live.reconcile(); h.live.fire(); await flush();
  assert.equal(h.calls.length, before + 1);
});

test('late registration catches up and unmount releases every refresh resource', async () => {
  let register;
  const h = await mount(() => state('claimed'), undefined, undefined, undefined,
    { registration: new Promise(r => { register = r; }) });
  assert.equal(h.calls.length, 1);
  register(); await flush(); h.live.fire(); await flush();
  assert.equal(h.calls.length, 2);
  h.live.emit(hint()); h.unmount(); h.live.fire(); h.live.reconcile(); await flush();
  assert.equal(h.calls.length, 2);
  assert.equal(h.live.timers + h.live.intervals, 0);
  assert.equal(h.live.observing, false);
  assert.equal(h.live.releases, 1);
});
