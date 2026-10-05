const { assert, test, status, clone, frozenDraft, successor, decision,
  mountReview, assertPreserved } = require('./human_review_transport.cjs');

for (const kind of ['revise', 'reject']) {
  test(`${kind}: real API sends the frozen identity and only refreshes a verified human decision`, async t => {
    const h = await mountReview(); t.after(h.unmount);
    const text = '  operator edit  ';
    const item = h.open(kind, text);
    const unchanged = clone(item);
    const response = decision(item, kind);
    h.setSnapshot({ ...status('A'), latestSequence: 4, timelineBeforeSequence: 4,
      directiveDrafts: kind === 'revise' ? [response.draft] : [],
      timeline: [{ id: 'human-event', sequence: 4, eventType: 'human_directive_review',
        timestamp: response.receipt.createdAt, summary: 'human decision', humanReview: response.receipt }] });
    await h.b.submitReview();
    assert.deepEqual(h.calls, [{ command: `${kind}_scan_directive`, input: {
      scanId: item.scanId, draftId: item.id, revision: item.revision, draftHash: item.draftHash,
      [kind === 'revise' ? 'text' : 'reason']: text.trim(),
    } }]);
    assert.deepEqual(item, unchanged, 'the displayed original snapshot is immutable');
    assert.equal(h.reads(), 1);
    assert.equal(h.b.reviewingDraft.value, undefined);
    assert.equal(h.b.reviewText.value, '');
    assert.equal(h.b.actionError.value, '');
    assert.equal(h.b.sending.value, false);
    assert.equal(h.b.state.value.timeline[0].humanReview.executionCompleted, false);
    assert.ok(h.b.state.value.timeline[0].humanReview.actions.every(a => a.executionState === 'not_started'));
    await h.b.confirmDirective(item);
    assert.equal(h.calls.length, 1, 'the original frozen draft is terminal and cannot be confirmed again');
    if (kind === 'revise') {
      const next = h.b.state.value.directiveDrafts[0];
      assert.equal(next.confirmationRequired, true);
      await h.b.confirmDirective(next);
      assert.deepEqual(h.calls[1], { command: 'confirm_scan_directive', input: {
        scanId: next.scanId, draftId: next.id, revision: next.revision, draftHash: next.draftHash,
      } });
      assert.equal(h.calls.filter(c => c.command === 'confirm_scan_directive').length, 1,
        'revision does not automatically approve or dispatch a role');
    }
  });
}

const receiptFaults = [
  ['missing receipt', r => { r.receipt = null; }],
  ['foreign scan', r => { r.receipt.scanId = 'other-scan'; }],
  ['foreign attempt', r => { r.receipt.attemptNumber++; }],
  ['foreign Root', r => { r.receipt.rootRunId = 'other-root'; }],
  ['foreign target', r => { r.receipt.targetKey = 'https://other.test'; }],
  ['foreign thread', r => { r.receipt.threadKey = 'other-thread'; }],
  ['wrong old draft', r => { r.receipt.draftId = 'other-draft'; }],
  ['wrong old revision', r => { r.receipt.revision++; }],
  ['wrong old hash', r => { r.receipt.draftHash = 'other-hash'; }],
  ['wrong decision kind', r => { r.receipt.kind = 'approve'; }],
  ['not terminal', r => { r.receipt.terminal = false; }],
  ['invented execution completed', r => { r.receipt.executionCompleted = true; }],
  ['already queued', r => { r.receipt.directiveId = 'queue'; }],
  ['missing receipt identity', r => { r.receipt.receiptId = ''; }],
  ['invalid committed sequence', r => { r.receipt.sequence = 0; }],
  ['invalid argument hash', r => { r.receipt.argumentHash = 'hash'; }],
  ['missing timestamp', r => { r.receipt.createdAt = ''; }],
  ['missing actions', r => { r.receipt.actions = null; }],
  ['missing role action', r => { r.receipt.actions.pop(); }],
  ['reordered roles', r => { r.receipt.actions.reverse(); }],
  ['wrong ordinal', r => { r.receipt.actions[0].order = 2; }],
  ['foreign role', r => { r.receipt.actions[0].role = 'unimplemented-role'; }],
  ['wrong action intent', r => { r.receipt.actions[0].intent = 'scope_expansion'; }],
  ['missing action reason', r => { delete r.receipt.actions[0].reasonCode; }],
  ['wrong terminal action reason', r => { r.receipt.actions[0].reasonCode = 'execution_completed'; }],
  ['queued action', r => { r.receipt.actions[0].reviewDisposition = 'coordinator_queued'; }],
  ['granted capability', r => { r.receipt.actions[0].capabilityState = 'existing_policy_required'; }],
  ['invented action execution', r => { r.receipt.actions[0].executionState = 'completed'; }],
  ['invented action queue', r => { r.receipt.actions[0].directiveId = 'queue'; }],
  ['invented action receipt', r => { r.receipt.actions[0].executionReceipt = {}; }],
];
for (const kind of ['revise', 'reject']) {
  for (const [label, alter] of receiptFaults) {
    test(`${kind}: ${label} cannot consume the editor or refresh as success`, async t => {
      const result = decision(frozenDraft(), kind); alter(result);
      const h = await mountReview(() => Promise.resolve(result)); t.after(h.unmount);
      h.open(kind); await h.b.submitReview(); assertPreserved(h);
    });
  }
}

const successorFaults = [
  ['foreign Root', d => { d.rootRunId = 'other-root'; }],
  ['foreign target', d => { d.targetKey = 'https://other.test'; }],
  ['foreign recipient', d => { d.recipientRole = 'unimplemented-role'; }],
  ['foreign scan', d => { d.scanId = 'B'; }],
  ['foreign attempt', d => { d.attemptNumber++; }],
  ['foreign thread', d => { d.threadKey = 'other-thread'; }],
  ['reused draft identity', d => { d.id = 'A-draft'; }],
  ['reused source message', d => { d.sourceMessageId = 'A-message'; }],
  ['reused frozen revision', d => { d.revision = 1; }],
  ['reused frozen hash', d => { d.draftHash = 'hash'; }],
  ['already confirmed', d => { d.status = 'confirmed'; d.confirmedDirectiveId = 'queue'; }],
  ['pending without confirmation', d => { d.confirmationRequired = false; }],
];
for (const [label, alter] of successorFaults) {
  test(`revise: a ${label} successor cannot replace the original proposal`, async t => {
    const item = frozenDraft(); const next = successor(item); alter(next);
    const h = await mountReview(() => Promise.resolve(decision(item, 'revise', next))); t.after(h.unmount);
    h.open(); await h.b.submitReview(); assertPreserved(h);
  });
}
for (const [label, change] of [
  ['non-null successor revision', { successorRevision: 2 }],
  ['non-null successor hash', { successorHash: 'other-hash' }],
  ['empty rejection reason', { reason: ' ' }],
  ['required confirmation', { requiresConfirmation: true }],
]) {
  test(`reject: ${label} cannot settle a terminal rejection`, async t => {
    const result = decision(frozenDraft(), 'reject'); Object.assign(result.receipt, change);
    const h = await mountReview(() => Promise.resolve(result)); t.after(h.unmount);
    h.open('reject'); await h.b.submitReview(); assertPreserved(h);
  });
}

test('revise: a reparsed rejected child is recorded without approval or execution', async t => {
  const item = frozenDraft(); const next = successor(item, { status: 'rejected',
    validationResult: 'rejected', coordinatorDecision: 'reject', confirmationRequired: false,
    sideEffectClass: 'irreversible_blocked', reasonCodes: ['unsupported_role'] });
  const h = await mountReview(() => Promise.resolve(decision(item, 'revise', next))); t.after(h.unmount);
  h.open(); h.setSnapshot({ ...status('A'), directiveDrafts: [next] });
  await h.b.submitReview();
  assert.equal(h.b.reviewingDraft.value, undefined);
  assert.equal(h.b.state.value.directiveDrafts[0].status, 'rejected');
  await h.b.confirmDirective(next);
  assert.equal(h.calls.length, 1);
});
