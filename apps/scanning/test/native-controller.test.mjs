import test from 'node:test';
import assert from 'node:assert/strict';
import { createScanController } from '../native-controller.mjs';

function fixture() {
  const calls = [], listeners = new Set(); let destroyed = false, failure, pending;
  const client = {
    ready: Promise.resolve({ projectId: 71, projectName: 'Synthetic project', appRevision: 2, grantRevision: 1 }),
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
  return { controller, calls, get destroyed() { return destroyed; }, get listenerCount() { return listeners.size; }, fail(name) { failure = name; }, defer(name) { let resolve; const promise = new Promise(done => { resolve = done; }); pending = { name, promise }; return resolve; } };
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
