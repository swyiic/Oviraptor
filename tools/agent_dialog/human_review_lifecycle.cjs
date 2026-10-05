const { assert, test, renderDialog, deferred, flush, status, clone, frozenDraft,
  decision, mountReview, assertPreserved } = require('./human_review_transport.cjs');

for (const kind of ['revise', 'reject']) {
  test(`${kind}: transport failure retains reason/edit and never exposes private exception text`, async t => {
    const h = await mountReview(() => Promise.reject(new Error('Authorization: Bearer private-secret password=hidden')));
    t.after(h.unmount);
    h.open(kind); await h.b.submitReview(); assertPreserved(h);
    assert.doesNotMatch(h.b.actionError.value, /private-secret|hidden|Authorization/);
    assert.doesNotMatch(await renderDialog(h.b), /private-secret|hidden/);
  });

  test(`${kind}: duplicate submission stays single-flight and cannot become legacy cancel or approval`, async t => {
    const pending = deferred();
    const h = await mountReview(() => pending.promise); t.after(h.unmount);
    const item = h.open(kind); const operation = h.b.submitReview();
    assert.equal(h.b.sending.value, true);
    assert.match(await renderDialog(h.b), /<textarea[^>]*disabled/);
    await h.b.submitReview(); await h.b.confirmDirective(item); await h.b.cancelDirective(item);
    h.b.beginReview(item, kind === 'revise' ? 'reject' : 'revise');
    assert.equal(h.calls.length, 1);
    assert.equal(h.b.reviewKind.value, kind);
    assert.equal(h.b.reviewText.value, 'operator edit');
    pending.resolve(decision(item, kind)); await operation;
    assert.equal(h.b.sending.value, false);
    assert.equal(h.calls.length, 1);
  });

  test(`${kind}: an empty edited text/reason makes no business request`, async t => {
    const h = await mountReview(); t.after(h.unmount);
    h.open(kind, '  '); await h.b.submitReview();
    assert.equal(h.calls.length, 0); assert.equal(h.b.sending.value, false);
    assert.equal(h.b.reviewingDraft.value.id, h.base.id);
    assert.equal(h.b.reviewText.value, '  ');
  });
}

test('legacy cancel still uses cancel_scan_directive while reject writes a distinct operator reason', async t => {
  const h = await mountReview(); t.after(h.unmount);
  const item = h.b.state.value.directiveDrafts[0];
  await h.b.cancelDirective(item);
  assert.deepEqual(h.calls, [{ command: 'cancel_scan_directive', input: {
    scanId: item.scanId, draftId: item.id, revision: item.revision, draftHash: item.draftHash,
  } }]);
  h.clearReads(); h.open('reject', 'human rejection reason'); await h.b.submitReview();
  assert.equal(h.calls[1].command, 'reject_scan_directive');
  assert.equal(h.calls[1].input.reason, 'human rejection reason');
});

for (const kind of ['revise', 'reject']) {
  for (const statusName of ['confirmed', 'cancelled', 'expired', 'rejected']) {
    test(`${kind}: an ${statusName} original cannot be revoked through the editor`, async t => {
      const h = await mountReview(); t.after(h.unmount);
      const row = frozenDraft('A', { status: statusName,
        confirmedDirectiveId: statusName === 'confirmed' ? 'already-queued' : '' });
      h.b.state.value = { ...status('A'), directiveDrafts: [row] };
      h.b.beginReview(row, kind); await h.b.submitReview();
      assert.equal(h.calls.length, 0); assert.equal(h.b.reviewingDraft.value, undefined);
    });
  }
}

const changedSnapshot = [
  ['revision', { revision: 2 }], ['hash', { draftHash: 'changed' }],
  ['Root', { rootRunId: 'changed-root' }], ['target', { targetKey: 'https://changed.test' }],
  ['recipient', { recipientRole: 'unimplemented-role' }], ['thread', { threadKey: 'other-thread' }],
  ['roles', { requestedRoles: ['coordinator', 'unimplemented-role'] }],
  ['intent', { intent: 'scope_expansion' }], ['text', { text: 'different frozen text' }],
  ['safe execution text', { safeExecutionText: 'different frozen action' }],
];
for (const [label, patch] of changedSnapshot) {
  test(`review refuses a displayed old draft after same-attempt polling changes its ${label}`, async t => {
    const h = await mountReview(); t.after(h.unmount);
    const displayed = h.b.state.value.directiveDrafts[0];
    h.setSnapshot({ ...status('A'), directiveDrafts: [{ ...clone(displayed), ...patch }] });
    await h.b.loadStatus(); h.clearReads();
    h.b.beginReview(displayed, 'revise'); await h.b.submitReview();
    assert.equal(h.calls.length, 0, 'the displayed frozen snapshot is not the current proposal');
    assert.equal(h.b.reviewingDraft.value, undefined);
  });
}

for (const kind of ['revise', 'reject']) {
  for (const view of ['scan-roundtrip', 'project-roundtrip', 'attempt-resume', 'lost-status', 'unmount']) {
    for (const fails of [false, true]) {
      test(`${kind}: ${view} fences a late ${fails ? 'failure' : 'receipt'} without consuming current input`, async t => {
        const pending = deferred();
        const h = await mountReview(() => pending.promise); t.after(h.unmount);
        const item = h.open(kind); const operation = h.b.submitReview();
        assert.equal(h.calls.length, 1, 'the real command must actually start');
        if (view === 'scan-roundtrip') { h.b.scanId.value = 'B'; h.b.scanId.value = 'A'; await flush(); }
        if (view === 'project-roundtrip') {
          h.rootProps.projectId = 2; await flush();
          h.rootProps.projectId = 1; await flush();
        }
        if (view === 'attempt-resume') h.b.state.value = { ...status('A'), attemptNumber: 2 };
        if (view === 'lost-status') h.b.state.value = undefined;
        if (view === 'unmount') h.unmount();
        h.clearReads(); h.b.draft.value = 'current composer text'; h.b.actionError.value = 'current view error';
        const lock = h.b.sending.value;
        const editor = h.b.reviewText.value;
        if (fails) pending.reject(new Error('stale-private-error'));
        else pending.resolve(decision(item, kind));
        await operation;
        assert.equal(h.reads(), 0, 'late old decision cannot refresh the new view');
        assert.equal(h.b.actionError.value, 'current view error');
        assert.equal(h.b.draft.value, 'current composer text');
        assert.equal(h.b.reviewText.value, editor);
        assert.equal(h.b.sending.value, lock, 'stale finally cannot change the new operation lock');
      });
    }
  }
}

for (const kind of ['revise', 'reject']) {
  for (const fails of [false, true]) {
    test(`${kind}: thread switch fences the old review and permits a new review without stale settlement`, async t => {
      const old = deferred(); const next = deferred(); let calls = 0;
      const h = await mountReview(() => (++calls === 1 ? old.promise : next.promise)); t.after(h.unmount);
      const item = h.open(kind); const operation = h.b.submitReview();
      h.b.threadFilter.value = 'other-thread';
      assert.equal(h.b.sending.value, false, 'leaving a review view must release only that old review lock');
      h.b.threadFilter.value = 'team';
      h.open(kind, 'new review text'); const newer = h.b.submitReview();
      assert.equal(h.calls.length, 2); assert.equal(h.b.sending.value, true);
      h.b.actionError.value = 'new action error'; h.clearReads();
      if (fails) old.reject(new Error('stale-thread-error')); else old.resolve(decision(item, kind));
      await operation;
      assert.equal(h.b.actionError.value, 'new action error');
      assert.equal(h.b.reviewText.value, 'new review text');
      assert.equal(h.b.reviewingDraft.value.id, item.id);
      assert.equal(h.b.sending.value, true);
      assert.equal(h.reads(), 0);
      next.resolve(decision(item, kind)); await newer;
      assert.equal(h.b.sending.value, false);
      assert.equal(h.b.reviewingDraft.value, undefined);
    });
  }
}
