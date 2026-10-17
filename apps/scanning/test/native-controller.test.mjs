import test from 'node:test';
import assert from 'node:assert/strict';
import { createScanController, readScanTargetHandoff } from '../native-controller.mjs';

function fixture({ handoff } = {}) {
  const calls = [], listeners = new Set(); let destroyed = false, failure, pending;
  let context = { projectId: 71, projectName: 'Synthetic project', appRevision: 2, grantRevision: 1, ...(handoff ? { handoff } : {}) };
  const client = {
    ready: Promise.resolve(context), get context() { return context; },
    async callAction(name, args, options) {
      calls.push({ name, args, options });
      if (failure === name) throw Object.assign(Error('app_page_result_unknown'), { code: 'app_page_result_unknown' });
      if (pending?.name === name) return pending.promise;
      if (name === 'scan_list') return { items: [{ taskId: 'original-task', title: 'Saved', status: 'draft', attemptNumber: 0 }], total: 2, returned: 1, hasMore: !args.cursor, nextCursor: args.cursor ? null : 'project-bound-cursor', asOf: 'synthetic-read' };
      if (name === 'scan_status') return { taskId: args.taskId, status: 'draft', attemptNumber: 0 };
      if (name === 'configuration_status') return { configured: true };
      if (name === 'scan_draft_create') return { taskId: 'original-task', creationReceipt: { liveState: false }, executionAvailable: true };
      if (name === 'configure_model') return { configured: true, providerRequestSent: false };
      return { taskId: args.taskId, outcome: { accepted: true, task: { taskId: args.taskId, status: name === 'scan_pause' ? 'pausing' : 'queued' } } };
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    destroy() { destroyed = true; },
  };
  let sequence = 0;
  const controller = createScanController(client, { operationId: () => `synthetic-operation-${++sequence}` });
  return { controller, calls, get destroyed() { return destroyed; }, get listenerCount() { return listeners.size; }, changeContext(next) { context = { ...context, ...next }; }, fail(name) { failure = name; }, defer(name) { let resolve; const promise = new Promise(done => { resolve = done; }); pending = { name, promise }; return resolve; } };
}

test('page controller finds saved tasks and forwards the real cursor without creating or starting anything', async () => {
  const f = fixture();
  await f.controller.initialize();
  assert.deepEqual(f.calls.map(call => call.name).sort(), ['configuration_status', 'scan_list']);
  assert.equal(f.controller.state.context.projectId, 71);
  await f.controller.list(f.controller.state.page.nextCursor);
  assert.deepEqual(f.calls.at(-1).args, { limit: 20, cursor: 'project-bound-cursor' });
  assert.equal(f.controller.state.page.hasMore, false);
  f.controller.destroy();
});

test('draft creation and explicit start remain two actions with distinct durable operation identifiers', async () => {
  const f = fixture(); await f.controller.initialize();
  await f.controller.create({ title: 'Manual draft', targets: ['https://fixture.invalid/'] });
  assert.equal(f.calls.filter(call => call.name === 'scan_start').length, 0);
  assert.equal(f.controller.state.task.status, 'draft');
  await f.controller.act('scan_start');
  const mutations = f.calls.filter(call => ['scan_draft_create', 'scan_start'].includes(call.name));
  assert.deepEqual(mutations.map(call => call.options.operationId), ['synthetic-operation-1', 'synthetic-operation-2']);
  assert.deepEqual(mutations[1].args, { taskId: 'original-task' });
  // This transport fixture does not claim the engine ran. Only the received
  // historical receipt says queued; current status still comes from read.
  assert.equal(f.controller.state.receipt.outcome.task.status, 'queued');
  assert.equal(f.controller.state.task.status, 'draft');
  f.controller.destroy();
});

test('lost mutation reply retains its operation ID, blocks automatic retry and still allows readonly inspection', async () => {
  const f = fixture(); await f.controller.initialize(); await f.controller.read('original-task');
  f.fail('scan_start');
  await assert.rejects(f.controller.act('scan_start'), /app_page_result_unknown/u);
  assert.equal(f.controller.state.uncertain, true);
  assert.equal(f.controller.state.operationId, 'synthetic-operation-1');
  await assert.rejects(f.controller.act('scan_start'), /scan_mutation_outcome_requires_review/u);
  assert.equal(f.calls.filter(call => call.name === 'scan_start').length, 1);
  await f.controller.read();
  assert.equal(f.controller.state.uncertain, true);
  f.controller.destroy();
});

test('page teardown detaches observers during an accepted pending call without issuing cancel, pause or lifecycle stop', async () => {
  const f = fixture(); await f.controller.initialize(); await f.controller.read('original-task');
  const complete = f.defer('scan_start');
  const call = f.controller.act('scan_start');
  f.controller.destroy();
  complete({ taskId: 'original-task', outcome: { accepted: true } });
  await call;
  assert.equal(f.destroyed, true); assert.equal(f.listenerCount, 0);
  assert.equal(f.calls.some(call => ['scan_pause', 'scan_draft_cancel', 'lifecycle/stop'].includes(call.name)), false);
});

test('operator configuration uses only the application action and does not retain credentials in controller state', async () => {
  const f = fixture(); await f.controller.initialize();
  await f.controller.configure({ llm: 'fixture', deployment: 'local', apiBase: 'http://127.0.0.1:1/v1', apiKey: 'synthetic-private-token' });
  assert.equal(f.calls.filter(call => call.name === 'configure_model').length, 1);
  assert.equal(JSON.stringify(f.controller.state).includes('synthetic-private-token'), false);
  assert.equal(f.controller.state.configuration.configured, true);
  f.controller.destroy();
});

test('a durable uncertain engine receipt remains visible and blocks a second mutation after a successful status read', async () => {
  const f = fixture(); await f.controller.initialize(); await f.controller.read('original-task');
  const complete = f.defer('scan_start');
  const call = f.controller.act('scan_start');
  complete({ taskId: 'original-task', outcome: { accepted: false, disposition: 'uncertain', executionStarted: 'unknown' } });
  const receipt = await call;
  assert.equal(f.controller.state.receipt, receipt);
  assert.equal(f.controller.state.uncertain, true);
  assert.equal(f.controller.state.task.status, 'draft');
  await assert.rejects(f.controller.act('scan_retry'), /scan_mutation_outcome_requires_review/u);
  assert.equal(f.calls.some(call => call.name === 'scan_retry'), false);
  f.controller.destroy();
});

test('inline execution confirmation cancels without dispatch and confirms exactly one operation without modal APIs', async () => {
  const f = fixture(); await f.controller.initialize(); await f.controller.read('original-task');
  for (const name of ['scan_start', 'scan_resume', 'scan_retry']) {
    f.controller.requestExecution(name);
    assert.deepEqual(f.controller.state.confirmation, { name, taskId: 'original-task' });
    assert.equal(f.calls.some(call => call.name === name), false);
    f.controller.cancelExecution();
    assert.equal(f.controller.state.confirmation, null);
    assert.equal(f.calls.some(call => call.name === name), false);
  }
  f.controller.requestExecution('scan_start');
  await f.controller.confirmExecution();
  await assert.rejects(f.controller.confirmExecution(), /scan_confirmation_stale/u);
  assert.equal(f.calls.filter(call => call.name === 'scan_start').length, 1);
  f.controller.destroy();
});

test('selecting another task invalidates an earlier inline confirmation without starting either task', async () => {
  const f = fixture(); await f.controller.initialize(); await f.controller.read('original-task');
  f.controller.requestExecution('scan_start');
  await f.controller.read('other-synthetic-task');
  assert.equal(f.controller.state.confirmation, null);
  await assert.rejects(f.controller.confirmExecution(), /scan_confirmation_stale/u);
  assert.equal(f.calls.some(call => call.name === 'scan_start'), false);
  f.controller.destroy();
});

function handoff(overrides = {}) {
  return { protocol: 'oviraptor.handoff.v1', purpose: 'draft-only', sourceAppId: 'org.oviraptor.assets', projectId: 71,
    payload: { kind: 'scan.targets.v1', targets: ['https://fixture.invalid/Exact?Keep=Case'], mode: 'quick', source: { kind: 'asset-selection', applicationInstanceId: 'synthetic-asset-instance', selectionId: 'f9b7e667-3c18-4567-89ab-123456789012', selectionDigest: 'a'.repeat(64), assetCount: 2 } }, ...overrides };
}

test('handoff reception and explicit review make zero mutation calls; draft creation never forwards provenance or starts execution', async () => {
  const f = fixture({ handoff: handoff() }); await f.controller.initialize();
  assert.deepEqual(f.calls.map(call => call.name).sort(), ['configuration_status', 'scan_list']);
  assert.equal(f.controller.state.handoffReviewed, false);
  assert.equal(f.controller.state.handoff.source.assetCount, 2);
  const input = f.controller.reviewHandoff();
  assert.equal(f.calls.length, 2); assert.equal(f.controller.state.handoffReviewed, true);
  assert.deepEqual(input, { title: '已审核的资产线索', targets: ['https://fixture.invalid/Exact?Keep=Case'], scanMode: 'quick' });
  await f.controller.create(input);
  const create = f.calls.filter(call => call.name === 'scan_draft_create');
  assert.equal(create.length, 1); assert.deepEqual(create[0].args, input);
  assert.equal(f.calls.some(call => ['scan_start', 'scan_resume', 'scan_retry'].includes(call.name)), false);
  assert.equal(JSON.stringify(create[0]).includes('selectionDigest'), false);
  f.controller.destroy();
});

test('handoff project, purpose, target bounds and closed scan payload are verified before any draft action', async () => {
  const valid = handoff();
  assert.equal(readScanTargetHandoff(valid, 71).targets[0], valid.payload.targets[0]);
  for (const changed of [
    handoff({ projectId: 72 }), handoff({ purpose: 'execute' }), handoff({ sourceAppId: '' }),
    handoff({ payload: { ...valid.payload, kind: 'scan.start' } }),
    handoff({ payload: { ...valid.payload, confirmed: true } }),
    handoff({ payload: { ...valid.payload, mode: 'unbounded' } }),
    handoff({ payload: { ...valid.payload, source: undefined } }),
    handoff({ payload: { ...valid.payload, source: { ...valid.payload.source, confirmed: true } } }),
    handoff({ payload: { ...valid.payload, source: { ...valid.payload.source, selectionId: 'not-a-uuid' } } }),
    handoff({ payload: { ...valid.payload, source: { ...valid.payload.source, selectionDigest: 'not-a-digest' } } }),
    handoff({ payload: { ...valid.payload, source: { ...valid.payload.source, assetCount: 0 } } }),
    ...[[], Array(201).fill('https://fixture.invalid/'), ['file:///etc/passwd'], ['https://u:p@fixture.invalid/'], ['https://fixture.invalid/#fragment'], ['https://fixture.invalid/;https://u:p@other.invalid/'], ['https://fixture.invalid/\nnext']].map(targets => handoff({ payload: { ...valid.payload, targets } })),
  ]) {
    assert.throws(() => readScanTargetHandoff(changed, 71));
    const f = fixture({ handoff: changed }); await f.controller.initialize();
    assert.equal(f.controller.state.handoff, null);
    assert.match(f.controller.state.error, /handoff/u);
    assert.throws(() => f.controller.reviewHandoff(), /scan_handoff_review_unavailable/u);
    assert.equal(f.calls.some(call => call.name === 'scan_draft_create' || call.name === 'scan_start'), false);
    f.controller.destroy();
  }
});

test('changed trusted page context invalidates a reviewed handoff and task confirmation without dispatch or cancellation', async () => {
  const f = fixture({ handoff: handoff() }); await f.controller.initialize();
  const input = f.controller.reviewHandoff(); await f.controller.read('original-task');
  f.controller.requestExecution('scan_start'); const before = f.calls.length;
  f.changeContext({ projectId: 72, handoff: undefined });
  await assert.rejects(f.controller.create(input), /scan_page_context_changed_reopen_required/u);
  assert.throws(() => f.controller.reviewHandoff(), /scan_page_context_changed_reopen_required/u);
  assert.throws(() => f.controller.confirmExecution(), /scan_page_context_changed_reopen_required/u);
  assert.equal(f.calls.length, before); assert.equal(f.controller.state.handoff, null);
  assert.equal(f.controller.state.confirmation, null); assert.equal(f.controller.state.ready, false);
  assert.equal(f.calls.some(call => ['scan_start', 'scan_pause', 'scan_draft_cancel'].includes(call.name)), false);
  f.controller.destroy();
});

test('operator handoff retains all 200 original owner targets and refuses 201 without silent truncation', async () => {
  const targets=Array.from({length:200},(_,n)=>`https://fixture.invalid/Exact/${n}`);
  const source=handoff();source.payload.targets=targets;source.payload.source.assetCount=200;
  const f=fixture({handoff:source});await f.controller.initialize();
  assert.deepEqual(f.controller.reviewHandoff().targets,targets);assert.equal(f.calls.length,2);
  const oversized=structuredClone(source);oversized.payload.targets.push('https://fixture.invalid/201');oversized.payload.source.assetCount=201;
  assert.throws(()=>readScanTargetHandoff(oversized,71),/scan_handoff_targets_invalid/u);
  assert.equal(f.calls.some(call=>call.name==='scan_draft_create'||call.name==='scan_start'),false);f.controller.destroy();
});
