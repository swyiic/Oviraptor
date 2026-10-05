// Real AgentDialog setup/render regressions: action_fencing.
const { assert, test, renderDialog, deferred, flush, scan, status, directive,
  mount, pendingReceipt, event } = require('./agent_dialog_harness.cjs');

test('resuming the same scan invalidates in-flight actions bound to the old attempt', async (t) => {
  const pending = deferred();
  const { b, api, unmount } = await mount({ draftScanDirective: () => pending.promise });
  t.after(unmount);
  b.draft.value = 'old attempt message';
  const sending = b.send();
  api.getNativeScanStatus = async () => ({ ...status('A'), attemptNumber: 2, latestSequence: 20 });
  await b.loadStatus();
  assert.equal(b.sending.value, false);
  pending.resolve(directive('A', 'rejected'));
  await sending;
  assert.equal(b.directiveDrafts.value.find((item) => item.status === 'rejected'), undefined);
  assert.equal(b.error.value, '');
});

test('receipt-integrity cache invalidation fences every pending human action across recovery', async (t) => {
  for (const action of ['send', 'confirmDirective', 'cancelDirective', 'reconcileReceipt']) {
    for (const attemptNumber of [1, 2]) {
      for (const fails of [false, true]) {
        const old = deferred(); const next = deferred(); let draftCalls = 0;
        const { b, api, unmount } = await mount({
          getNativeScanStatus: async (id) => ({ ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
            timeline: [pendingReceipt()], directiveDrafts: [directive(id)] }),
          draftScanDirective: () => (++draftCalls === 1 && action === 'send' ? old.promise : next.promise),
          confirmScanDirective: () => old.promise,
          cancelScanDirective: () => old.promise,
          reconcileScanDirectiveReceipt: () => old.promise,
        });
        t.after(unmount);
        b.draft.value = 'old message';
        const operation = action === 'send' ? b.send()
          : b[action](action === 'reconcileReceipt' ? b.state.value.timeline[0] : directive('A'));
        assert.equal(b.sending.value, true, `${action}: the original operation must actually start`);
        api.getNativeScanStatus = async () => { throw new Error('request_review_receipt_invalid'); };
        await b.loadStatus();
        assert.equal(b.state.value, undefined);
        assert.equal(b.sending.value, false, `${action}: invalidated view cannot retain old lock`);
        let reads = 0;
        api.getNativeScanStatus = async (id) => { reads++; return { ...status(id), attemptNumber }; };
        await b.loadStatus();
        b.draft.value = 'new message';
        const newer = b.send();
        assert.equal(b.sending.value, true);
        b.actionError.value = 'current action error';
        if (fails) old.reject(new Error('old action error'));
        else old.resolve(directive('A', 'rejected'));
        await operation;
        assert.equal(b.directiveDrafts.value.find((item) => item.status === 'rejected'), undefined, action);
        assert.equal(b.actionError.value, 'current action error', action);
        assert.equal(b.sending.value, true, `${action}: stale finally cannot unlock new operation`);
        assert.equal(b.draft.value, 'new message', action);
        assert.equal(reads, 1, `${action}: stale completion cannot refresh current view`);
        next.resolve({ ...directive('A'), attemptNumber }); await newer;
        assert.equal(b.sending.value, false);
      }
    }
  }
});

test('same-attempt polling keeps a pending human action single-flight', async (t) => {
  const pending = deferred(); let calls = 0;
  const { b, unmount } = await mount({ draftScanDirective: () => { calls++; return pending.promise; } });
  t.after(unmount);
  b.draft.value = 'current message';
  const operation = b.send();
  await b.loadStatus();
  assert.equal(b.sending.value, true);
  await b.send();
  assert.equal(calls, 1);
  pending.resolve(directive('A')); await operation;
  assert.equal(b.sending.value, false);
});

test('task drafts from a previous attempt cannot be confirmed', async (t) => {
  const calls = [];
  const { b, unmount } = await mount({
    getNativeScanStatus: async () => ({ ...status('A'), attemptNumber: 2 }),
    confirmScanDirective: (...args) => calls.push(args),
  });
  t.after(unmount);
  await b.confirmDirective(directive('A'));
  assert.equal(calls.length, 0);
});

test('same scan IDs across projects do not share composer contents', async (t) => {
  const { b, rootProps, unmount } = await mount();
  t.after(unmount);
  b.draft.value = 'project one';
  rootProps.projectId = 2;
  await flush();
  assert.equal(b.draft.value, '');
  b.draft.value = 'project two';
  rootProps.projectId = 1;
  await flush();
  assert.equal(b.draft.value, 'project one');
});

for (const action of ['confirmDirective', 'cancelDirective']) {
  test(`${action} sends the bound draft revision/hash and refreshes the committed projection`, async (t) => {
    const calls = [];
    const apiName = action === 'confirmDirective' ? 'confirmScanDirective' : 'cancelScanDirective';
    const { b, api, unmount } = await mount({ [apiName]: async (...args) => { calls.push(args); } });
    t.after(unmount);
    b.state.value.directiveDrafts = [directive('A')];
    let reads = 0;
    api.getNativeScanStatus = async (id) => { reads++; return status(id); };
    await b[action](directive('A'));
    assert.deepEqual(calls, [['A', 'A-draft', 1, 'hash']]);
    assert.equal(reads, 1);
    assert.equal(b.sending.value, false);
  });
}

test('unmounted dialog ignores a pending directive response', async () => {
  const pending = deferred();
  const { b, unmount } = await mount({ draftScanDirective: () => pending.promise });
  b.draft.value = 'message';
  const sending = b.send();
  unmount();
  pending.resolve(directive('A', 'rejected'));
  await sending;
  assert.equal(b.directiveDrafts.value.find((item) => item.status === 'rejected'), undefined);
  assert.equal(b.error.value, '');
  assert.equal(b.draft.value, 'message');
});

for (const action of ['confirmDirective', 'cancelDirective']) {
  test(`${action} refuses absent, superseded, ambiguous and terminal draft cards before IPC`, async (t) => {
    const calls = [];
    const apiName = action === 'confirmDirective' ? 'confirmScanDirective' : 'cancelScanDirective';
    const h = await mount({ [apiName]: async (...args) => calls.push(args) });
    t.after(h.unmount);
    let reads = 0;
    h.api.getNativeScanStatus = async (id) => { reads++; return status(id); };
    const base = directive('A');
    const cases = [
      ['absent', base, []],
      ['new revision', base, [{ ...base, revision: 2 }]],
      ['new hash', base, [{ ...base, draftHash: 'new-hash' }]],
      ['ambiguous id', base, [base, { ...base }]],
      ...['confirmed', 'cancelled', 'expired', 'unknown'].map((status) =>
        [status, { ...base, status }, [{ ...base, status }]]),
      ...[{ revision: 0 }, { revision: 1.5 }, { revision: Number.MAX_SAFE_INTEGER + 1 },
        { draftHash: '' }, { id: ' ' }].map((patch) => ['invalid identity', { ...base, ...patch }, [{ ...base, ...patch }]]),
    ];
    for (const [label, item, drafts] of cases) {
      h.b.state.value = { ...status('A'), directiveDrafts: drafts };
      h.b.draft.value = 'keep my unsent thought';
      await h.b[action](item);
      assert.equal(calls.length, 0, label);
      assert.equal(reads, 0, label);
      assert.equal(h.b.sending.value, false, label);
      assert.equal(h.b.draft.value, 'keep my unsent thought', label);
      assert.match(h.b.actionError.value, /directive_draft_not_current/, label);
    }
    h.b.state.value = { ...status('B'), directiveDrafts: [base] };
    await h.b[action](base);
    assert.equal(calls.length, 0, 'a foreign snapshot cannot authorize a current-task card');
  });
}

test('confirmation refuses contradictory validation or changed routing despite a matching revision/hash', async (t) => {
  let calls = 0;
  const h = await mount({ confirmScanDirective: async () => { calls++; } });
  t.after(h.unmount);
  const base = directive('A');
  for (const patch of [{ confirmationRequired: false }, { validationResult: 'rejected' },
    { coordinatorDecision: 'reject' }, { sideEffectClass: 'irreversible_blocked' },
    { confirmedDirectiveId: 'already-confirmed' }, { rootRunId: '' }]) {
    const item = { ...base, ...patch };
    h.b.state.value = { ...status('A'), directiveDrafts: [item] };
    await h.b.confirmDirective(item);
    assert.equal(calls, 0, JSON.stringify(patch));
    const html = await renderDialog(h.b);
    assert.match(html, /<button[^>]*disabled[^>]*>确认并入队<\/button>/);
  }
  for (const key of ['rootRunId', 'targetKey', 'recipientRole', 'threadKey']) {
    h.b.state.value = { ...status('A'), directiveDrafts: [{ ...base, [key]: 'changed' }] };
    await h.b.confirmDirective(base);
    assert.equal(calls, 0, key);
  }
});

test('same-version polling keeps current draft actions usable without relying on object identity', async (t) => {
  for (const action of ['confirmDirective', 'cancelDirective']) {
    for (const draftStatus of ['drafted', 'need_confirmation']) {
      const calls = [];
      const apiName = action === 'confirmDirective' ? 'confirmScanDirective' : 'cancelScanDirective';
      const item = directive('A', draftStatus);
      const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), directiveDrafts: [{ ...item }] }),
        [apiName]: async (...args) => calls.push(args) });
      t.after(h.unmount);
      await h.b.loadStatus();
      assert.notEqual(h.b.state.value.directiveDrafts[0], item);
      await h.b[action](item);
      assert.deepEqual(calls, [['A', 'A-draft', 1, 'hash']]);
    }
  }
});

test('an unbound historical pending draft can be cancelled but cannot be confirmed', async (t) => {
  const item = { ...directive('A'), rootRunId: '', targetKey: '', recipientRole: '' };
  const calls = [];
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), directiveDrafts: [item] }),
    confirmScanDirective: async () => calls.push('confirm'),
    cancelScanDirective: async (...args) => calls.push(args) });
  t.after(h.unmount);
  await h.b.confirmDirective(item);
  assert.deepEqual(calls, []);
  await h.b.cancelDirective(item);
  assert.deepEqual(calls, [['A', 'A-draft', 1, 'hash']]);
});
