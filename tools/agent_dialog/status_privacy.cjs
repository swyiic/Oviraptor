// Error text is untrusted display input, including errors from local IPC.
const { assert, test, mount, status, event, deferred, renderDialog } = require('../agent_dialog_harness.cjs');
const secret = '/private/fixture.sqlite?token=fixture-secret Authorization: Bearer fixture-key';
const snapshot = () => ({ ...status('A'), latestSequence: 1, timelineBeforeSequence: 1,
  timeline: [event('cached', 'agent_run', 1, 'previously verified message')] });
const statusMessage = '任务状态读取失败，请重试。';
const integrityMessage = '任务回执校验失败，已清除缓存，请重新读取。';
const listenerMessage = '实时事件连接失败，使用数据库轮询。';

for (const kind of ['Error', 'string']) {
  const failure = message => kind === 'Error' ? new Error(message) : message;
  test(`status ${kind} failure hides private error text while preserving the last snapshot and retry`, async t => {
    const h = await mount({ getNativeScanStatus: async () => snapshot() }); t.after(h.unmount);
    const before = h.b.state.value;
    h.api.getNativeScanStatus = async () => { throw failure(secret); };
    await h.b.loadStatus();
    const html = await renderDialog(h.b);
    assert.ok(!html.includes('fixture-secret') && !html.includes('fixture-key') && !html.includes('/private/'));
    assert.ok(html.includes(statusMessage));
    assert.equal(h.b.statusError.value, statusMessage);
    assert.equal(h.b.state.value, before);
    h.api.getNativeScanStatus = async () => snapshot(); await h.b.loadStatus();
    assert.equal(h.b.statusError.value, '');
  });

  for (const prefix of ['request_review_', 'administrative_closure_', 'closure_handoff_']) {
    test(`${prefix} ${kind} failure hides details without weakening cache invalidation`, async t => {
      const h = await mount({ getNativeScanStatus: async () => snapshot() }); t.after(h.unmount);
      const pending = deferred(); h.api.getNativeScanStatus = () => pending.promise;
      const old = h.b.loadStatus(1);
      h.api.getNativeScanStatus = async () => { throw failure(`${prefix}event_integrity ${secret}`); };
      await h.b.loadStatus(1);
      assert.equal(h.b.statusError.value, integrityMessage);
      assert.equal(h.b.state.value, undefined); assert.equal(h.b.statusSync.latestSequence.value, 0);
      const html = await renderDialog(h.b);
      assert.ok(html.includes(integrityMessage));
      assert.ok(!html.includes('fixture-secret') && !html.includes('previously verified message'));
      pending.resolve({ ...snapshot(), latestSequence: 99 }); await old;
      assert.equal(h.b.state.value, undefined);
      assert.equal(h.b.statusError.value, integrityMessage);
      let request;
      h.api.getNativeScanStatus = async (...args) => { request = args; return snapshot(); };
      await h.b.loadStatus(1);
      assert.equal(request[1], undefined, 'invalidated history requires a full read');
      assert.equal(h.b.statusError.value, '');
    });
  }

  test(`listener ${kind} failure hides private text without preventing database reads`, async t => {
    let reads = 0;
    const h = await mount({ getNativeScanStatus: async () => { reads++; return snapshot(); } }, {
      listen: async () => { throw failure(secret); },
    }); t.after(h.unmount);
    assert.equal(h.b.statusSync.listenerError.value, listenerMessage);
    const html = await renderDialog(h.b);
    assert.ok(html.includes(listenerMessage));
    assert.ok(!html.includes('fixture-secret') && !html.includes('fixture-key') && !html.includes('/private/'));
    const before = reads; await h.b.loadStatus();
    assert.equal(reads, before + 1); assert.equal(h.b.state.value.scanId, 'A');
  });
}
