// Real AgentDialog setup/render regressions: directives.
require('./agent_dialog/action_privacy.cjs');
const { assert, test, renderDialog, deferred, flush, scan, status, directive,
  mount, pendingReceipt, event } = require('./agent_dialog_harness.cjs');

test('a previous task response cannot erase the new composer or inject a rejected draft', async (t) => {
  const pending = deferred();
  const calls = [];
  const { b, unmount } = await mount({ draftScanDirective: (...args) => { calls.push(args); return pending.promise; } });
  t.after(unmount);
  b.draft.value = 'A message';
  const sending = b.send();
  b.scanId.value = 'B';
  await flush();
  b.draft.value = 'B message';
  pending.resolve(directive('A', 'rejected'));
  await sending;
  assert.deepEqual(calls, [['A', 'A message', 'team']]);
  assert.equal(b.draft.value, 'B message');
  assert.equal(b.directiveDrafts.value.find((item) => item.status === 'rejected'), undefined);
  assert.equal(b.error.value, '');
});

test('switching tasks isolates unsent text, including an A to B to A round trip', async (t) => {
  const pending = deferred();
  const { b, unmount } = await mount({ draftScanDirective: () => pending.promise });
  t.after(unmount);
  b.draft.value = 'A message';
  const sending = b.send();
  b.scanId.value = 'B';
  await flush();
  assert.equal(b.draft.value, '', 'A text must not be sent accidentally to B');
  b.scanId.value = 'A';
  await flush();
  b.draft.value = 'new A message';
  pending.reject(new Error('old request failed'));
  await sending;
  assert.equal(b.error.value, '');
  assert.equal(b.draft.value, 'new A message');
});

for (const action of ['confirmDirective', 'cancelDirective']) {
  test(`${action}: an old completion cannot unlock a new task operation`, async (t) => {
    const old = deferred(), next = deferred();
    const apiName = action === 'confirmDirective' ? 'confirmScanDirective' : 'cancelScanDirective';
    const { b, unmount } = await mount({ [apiName]: () => old.promise, draftScanDirective: () => next.promise });
    t.after(unmount);
    b.state.value.directiveDrafts = [directive('A')];
    const oldAction = b[action](directive('A'));
    assert.equal(b.sending.value, true, 'the original action must actually be in flight');
    b.scanId.value = 'B';
    await flush();
    assert.equal(b.sending.value, false, 'new task must not inherit old pending state');
    b.draft.value = 'new task message';
    const nextAction = b.send();
    assert.equal(b.sending.value, true);
    old.reject(new Error('old action failed'));
    await oldAction;
    assert.equal(b.error.value, '');
    assert.equal(b.sending.value, true, 'old finally must not unlock a newer request');
    next.resolve(directive('B'));
    await nextAction;
    assert.equal(b.sending.value, false);
  });
  test(`${action}: stale cards cannot be submitted under another scan`, async (t) => {
    const calls = [];
    const apiName = action === 'confirmDirective' ? 'confirmScanDirective' : 'cancelScanDirective';
    const { b, unmount } = await mount({ [apiName]: (...args) => calls.push(args) });
    t.after(unmount);
    await b[action](directive('B'));
    assert.equal(calls.length, 0);
  });
}

test('editing while parsing does not lose text composed after submission', async (t) => {
  const pending = deferred();
  const { b, unmount } = await mount({ draftScanDirective: () => pending.promise });
  t.after(unmount);
  b.draft.value = 'submitted text';
  const sending = b.send();
  b.draft.value = 'next thought';
  pending.resolve(directive('A'));
  await sending;
  assert.equal(b.draft.value, 'next thought');
});

test('a failed commit keeps the input and creates no optimistic sent message', async (t) => {
  const { b, unmount } = await mount({ draftScanDirective: async () => { throw new Error('commit failed'); } });
  t.after(unmount);
  b.draft.value = 'keep this input';
  await b.send();
  assert.equal(b.error.value, '草案提交结果无法核实，输入已保留；请刷新核对后再操作，不要直接重复提交。');
  assert.equal(b.draft.value, 'keep this input');
  assert.equal(b.directiveDrafts.value.find((item) => item.status === 'rejected'), undefined);
  assert.deepEqual(b.state.value.timeline, []);
  assert.equal(b.sending.value, false);
});

test('ambiguous multi-target routing explains thread selection and preserves unsent input', async (t) => {
  const h = await mount({ draftScanDirective: async () => { throw new Error('directive_thread_coordinator_ambiguous'); } });
  t.after(h.unmount);
  h.b.draft.value = '请优先检查权限控制';
  await h.b.send();
  assert.match(h.b.error.value, /多个目标|multiple targets/);
  assert.match(h.b.error.value, /线程|thread/);
  assert.equal(h.b.draft.value, '请优先检查权限控制');
  assert.deepEqual(h.b.state.value.timeline, []);
  assert.equal(h.b.sending.value, false);
});

test('active targets are selectable before their first message and sending retains the selected recipient', async (t) => {
  const calls = [];
  const target = 'https://second.authorized.example.test';
  const thread = 'coordinator:second';
  const h = await mount({
    getNativeScanStatus: async (id) => ({ ...status(id), directiveRecipients: [
      { rootRunId: 'first', targetKey: 'https://first.authorized.example.test' },
      { rootRunId: 'second', targetKey: target, threadKey: thread },
    ] }),
    draftScanDirective: async (...args) => { calls.push(args); return { ...directive(args[0], 'drafted'), threadKey: args[2] }; },
  });
  t.after(h.unmount);
  assert.ok(h.b.threadOptions.value.some((item) => item.key === thread && item.label === target && item.count === 0));
  h.b.threadFilter.value = thread;
  await flush();
  assert.equal(h.b.selectedThreadUnavailable.value, false);
  assert.equal(h.b.selectedThreadLabel.value, target);
  h.b.draft.value = '请优先检查权限控制';
  await h.b.send();
  assert.equal(calls.length, 1);
  assert.equal(calls[0][2], thread);
  assert.equal(h.b.draft.value, '');
  assert.equal(h.b.error.value, '');
  h.b.state.value.directiveRecipients = [];
  h.b.state.value.timeline = [{ id: 'saved-message', sequence: 1, eventType: 'user_directive',
    timestamp: 'stamp', fromRole: 'operator', summary: 'saved message', threadKey: thread, targetKey: target }];
  await flush();
  assert.equal(h.b.selectedThreadLabel.value, target, 'completed targets keep their readable label from persisted messages');
});

test('expired draft lease explains recreation without retrying confirmation or reporting success', async (t) => {
  let calls = 0;
  const h = await mount({ confirmScanDirective: async () => { calls++; throw new Error('directive_draft_stale_fencing_token'); } });
  t.after(h.unmount);
  const item = directive('A', 'drafted');
  h.b.state.value.directiveDrafts = [item];
  await h.b.confirmDirective(item);
  assert.equal(calls, 1);
  assert.match(h.b.error.value, /执行租约已变化|execution lease has changed/);
  assert.match(h.b.error.value, /重新创建|new draft/);
  assert.equal(h.b.sending.value, false);
  assert.equal(item.status, 'drafted');
});

test('an unmounted dialog ignores pending status failures', async () => {
  const { b, api, unmount } = await mount();
  const pending = deferred();
  api.getNativeScanStatus = () => pending.promise;
  const loading = b.loadStatus();
  unmount();
  pending.reject(new Error('late status failure'));
  await loading;
  assert.equal(b.error.value, '');
});

test('rejection remains visible after status refresh and preserves correctable text', async (t) => {
  const { b, unmount } = await mount({ draftScanDirective: async (id) => directive(id, 'rejected') });
  t.after(unmount);
  b.draft.value = 'needs correction';
  await b.send();
  assert.equal(b.draft.value, 'needs correction');
  assert.equal(b.directiveDrafts.value.find((item) => item.status === 'rejected')?.scanId, 'A');
  assert.match(b.error.value, /未进入执行队列/);
  await b.loadStatus();
  assert.match(b.error.value, /未进入执行队列/);
});

test('unsent messages remain private to their task and thread for this mounted session', async (t) => {
  const { b, unmount } = await mount();
  t.after(unmount);
  b.draft.value = 'team A';
  b.threadFilter.value = 'review';
  assert.equal(b.draft.value, '');
  b.draft.value = 'review A';
  b.scanId.value = 'B';
  await flush();
  assert.equal(b.draft.value, '');
  b.draft.value = 'team B';
  b.scanId.value = 'A';
  await flush();
  assert.equal(b.draft.value, 'team A');
  b.threadFilter.value = 'review';
  assert.equal(b.draft.value, 'review A');
});

for (const [label, change] of [
  ['another task', { scanId: 'B' }],
  ['another attempt', { attemptNumber: 2 }],
  ['another thread', { threadKey: 'unrelated-thread' }],
  ['missing identity', { id: '' }],
  ['invalid revision', { revision: 0 }],
  ['missing hash', { draftHash: '' }],
  ['terminal status', { status: 'confirmed' }],
  ['unknown status', { status: 'unexpected' }],
  ['invalid rejection details', { status: 'rejected', reasonCodes: 'not-an-array' }],
  ['invalid budget', { estimatedTokens: -1 }],
  ['pending but rejected validation', { validationResult: 'rejected' }],
  ['pending but rejected decision', { coordinatorDecision: 'reject' }],
  ['pending without confirmation', { confirmationRequired: false }],
  ['pending but already confirmed', { confirmedDirectiveId: 'old-directive' }],
  ['pending irreversible action', { sideEffectClass: 'irreversible_blocked' }],
  ['rejected with accepted validation', { status: 'rejected', coordinatorDecision: 'reject', confirmationRequired: false, validationResult: 'valid' }],
  ['rejected with accepted decision', { status: 'rejected', validationResult: 'rejected', confirmationRequired: false, coordinatorDecision: 'accept' }],
  ['rejected but requiring confirmation', { status: 'rejected', validationResult: 'rejected', coordinatorDecision: 'reject' }],
  ['rejected but already confirmed', { status: 'rejected', validationResult: 'rejected', coordinatorDecision: 'reject', confirmationRequired: false, confirmedDirectiveId: 'old-directive' }],
]) {
  test(`draft receipt: ${label} cannot consume input or inject a card`, async (t) => {
    let calls = 0;
    const h = await mount({ draftScanDirective: async () => { calls++; return { ...directive('A'), ...change }; } });
    t.after(h.unmount);
    let reads = 0;
    h.api.getNativeScanStatus = async (id) => { reads++; return status(id); };
    h.b.draft.value = 'keep my correction';
    const previous = h.b.state.value;
    await h.b.send();
    assert.equal(h.b.draft.value, 'keep my correction');
    assert.deepEqual(h.b.directiveDrafts.value, []);
    assert.match(h.b.actionError.value, /directive_draft_invalid_response/);
    assert.equal(h.b.state.value, previous);
    assert.equal(reads, 0, 'invalid acknowledgements must not trigger a success refresh');
    assert.equal(calls, 1, 'no automatic resubmission');
    assert.equal(h.b.sending.value, false);
    await renderDialog(h.b);
  });
}

test('a foreign rejected receipt cannot appear in the current task', async (t) => {
  const h = await mount({ draftScanDirective: async () => directive('B', 'rejected') });
  t.after(h.unmount);
  h.b.draft.value = 'my current message';
  await h.b.send();
  assert.deepEqual(h.b.directiveDrafts.value, []);
  assert.equal(h.b.draft.value, 'my current message');
  assert.match(h.b.actionError.value, /directive_draft_invalid_response/);
});

test('drafting requires an identified current-attempt snapshot', async (t) => {
  let calls = 0;
  const h = await mount({ draftScanDirective: async () => { calls++; return directive('A'); } });
  t.after(h.unmount);
  for (const invalid of [undefined, { ...status('B') }, { ...status('A'), attemptNumber: 0 }]) {
    h.b.state.value = invalid;
    h.b.draft.value = 'keep until status is known';
    await h.b.send();
    assert.equal(calls, 0);
    assert.equal(h.b.draft.value, 'keep until status is known');
    assert.match(h.b.actionError.value, /directive_status_unavailable/);
  }
  await h.b.loadStatus();
  await h.b.send();
  assert.equal(calls, 1);
  assert.equal(h.b.draft.value, '');
  assert.equal(h.b.actionError.value, '');
});

test('non-object draft responses preserve input and report the stable validation error', async (t) => {
  const h = await mount();
  t.after(h.unmount);
  for (const response of [null, undefined, [], 'not-a-draft']) {
    h.api.draftScanDirective = async () => response;
    h.b.draft.value = 'do not lose this';
    await h.b.send();
    assert.equal(h.b.draft.value, 'do not lose this');
    assert.match(h.b.actionError.value, /directive_draft_invalid_response/);
    assert.deepEqual(h.b.directiveDrafts.value, []);
    assert.equal(h.b.sending.value, false);
  }
});

for (const state of ['drafted', 'need_confirmation', 'rejected']) {
  test(`a valid redacted ${state} receipt remains compatible with the draft workflow`, async (t) => {
    const result = { ...directive('A', state), text: '[REDACTED]', proposedScopeChange: null };
    if (state === 'drafted') Object.assign(result, { validationResult: 'valid', coordinatorDecision: 'accept' });
    if (state === 'rejected') result.sideEffectClass = 'irreversible_blocked';
    const h = await mount({ draftScanDirective: async () => result });
    t.after(h.unmount);
    let reads = 0;
    h.api.getNativeScanStatus = async (id) => { reads++; return status(id); };
    h.b.draft.value = 'original user text';
    await h.b.send();
    assert.equal(reads, 1);
    assert.equal(h.b.draft.value, state === 'rejected' ? 'original user text' : '');
    assert.doesNotMatch(h.b.actionError.value, /directive_draft_invalid_response/);
    const html = await renderDialog(h.b);
    if (state === 'rejected') assert.match(html, /\[REDACTED\]/);
  });
}

test('receipt identity binds the submitted thread even when the composer switches threads', async (t) => {
  const pending = deferred();
  const h = await mount({ draftScanDirective: () => pending.promise });
  t.after(h.unmount);
  h.b.draft.value = 'submitted team message';
  const sending = h.b.send();
  h.b.threadFilter.value = 'review';
  h.b.draft.value = 'a separate review thought';
  pending.resolve(directive('A'));
  await sending;
  assert.equal(h.b.draft.value, 'a separate review thought');
  assert.equal(h.b.actionError.value, '');
  h.b.threadFilter.value = 'team';
  assert.equal(h.b.draft.value, '', 'the successfully submitted text must not return from the thread cache');
  h.b.threadFilter.value = 'review';
  assert.equal(h.b.draft.value, 'a separate review thought');
});

for (const { edit, settleAway } of ['none', 'different', 'same-text-again']
  .flatMap((edit) => [false, true].map((settleAway) => ({ edit, settleAway })))) {
  test(`a thread round trip settles only the submitted draft version: ${edit}, away=${settleAway}`, async (t) => {
    const pending = deferred();
    const h = await mount({ draftScanDirective: () => pending.promise });
    t.after(h.unmount);
    h.b.draft.value = 'original message';
    const sending = h.b.send();
    h.b.threadFilter.value = 'review';
    h.b.draft.value = 'review notes';
    h.b.threadFilter.value = 'team';
    assert.equal(h.b.draft.value, 'original message');
    if (edit !== 'none') h.b.draft.value = 'new team notes';
    if (edit === 'same-text-again') h.b.draft.value = 'original message';
    if (settleAway) h.b.threadFilter.value = 'review';
    pending.resolve(directive('A'));
    await sending;
    const expected = edit === 'none' ? '' : edit === 'different' ? 'new team notes' : 'original message';
    if (settleAway) {
      assert.equal(h.b.draft.value, 'review notes');
      h.b.threadFilter.value = 'team';
    }
    assert.equal(h.b.draft.value, expected);
    h.b.threadFilter.value = 'review';
    assert.equal(h.b.draft.value, 'review notes');
    h.b.threadFilter.value = 'team';
    assert.equal(h.b.draft.value, expected, 'settlement must also update the cached version');
  });
}

for (const outcome of ['rejected', 'error', 'invalid']) {
  test(`thread-cache text survives an unaccepted draft response: ${outcome}`, async (t) => {
    const pending = deferred();
    const h = await mount({ draftScanDirective: () => pending.promise });
    t.after(h.unmount);
    h.b.draft.value = 'correctable message';
    const sending = h.b.send();
    h.b.threadFilter.value = 'review';
    h.b.draft.value = 'private review notes';
    if (outcome === 'error') pending.reject(new Error('request failed'));
    else pending.resolve(directive(outcome === 'invalid' ? 'B' : 'A', 'rejected'));
    await sending;
    assert.equal(h.b.draft.value, 'private review notes');
    h.b.threadFilter.value = 'team';
    assert.equal(h.b.draft.value, 'correctable message');
  });
}
