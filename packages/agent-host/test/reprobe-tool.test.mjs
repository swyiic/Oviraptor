import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { DiscoveryAssist, DISCOVERY_ASSIST_TOOLS } from '../src/discovery-assist.mjs';

const TOKEN = 'a'.repeat(64);

function tempStore() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-reprobe-'));
  const file = join(dir, 'host.sqlite');
  return { store: new HostStore(file), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

/** A desktop reprobe executor stand-in. `respond(body)` returns the desktop JSON. */
async function fakeBridge(respond) {
  const calls = [];
  const server = createServer((request, response) => {
    let raw = '';
    request.on('data', chunk => { raw += chunk; });
    request.on('end', () => {
      assert.equal(request.headers.authorization, `Bearer ${TOKEN}`);
      const body = JSON.parse(raw);
      calls.push(body);
      response.end(JSON.stringify(respond(body)));
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { calls, origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise(resolve => server.close(resolve)) };
}

test('reprobe_assets is advertised through the discovery bridge map as the reprobe action', () => {
  // The model tool name and the desktop action it maps to are the contract the
  // apps.mjs dispatch and the Rust bridge both key on.
  assert.equal(DISCOVERY_ASSIST_TOOLS.get('reprobe_assets'), 'reprobe');
});

test('a query-scope reprobe forwards the scope verbatim and returns the run id unshaped', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  const bridge = await fakeBridge(body => ({ ok: true, result: { projectId: body.projectId, runId: 42, pipeline: 'reprobe', started: true } }));
  try {
    const assist = new DiscoveryAssist({ origin: bridge.origin, token: TOKEN });
    const scope = { kind: 'query', query: { search: 'example.com' }, expectedCount: 12 };
    const result = await assist.call(store, project.id, 'reprobe_assets', { scope }, 'op-1');
    // The desktop saw the reprobe action, the right desktop project id, and the exact scope.
    assert.equal(bridge.calls.length, 1);
    assert.equal(bridge.calls[0].action, 'reprobe');
    assert.equal(bridge.calls[0].projectId, 7);
    assert.deepEqual(bridge.calls[0].args, { scope });
    // Passthrough: the run id is returned unshaped (no proposal/humanReview rewriting).
    assert.equal(result.structuredContent.runId, 42);
    assert.equal(result.structuredContent.started, true);
    assert.equal(result.structuredContent.humanReview, undefined);
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('a selection-scope reprobe forwards the desktop asset ids it was handed', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '9' });
  const bridge = await fakeBridge(body => ({ ok: true, result: { projectId: body.projectId, runId: 7, started: true } }));
  try {
    const assist = new DiscoveryAssist({ origin: bridge.origin, token: TOKEN });
    // apps.mjs has already translated model ids to desktop asset ids by this point.
    const scope = { kind: 'selection', assetIds: [101, 102, 103], expectedCount: 3 };
    await assist.call(store, project.id, 'reprobe_assets', { scope }, 'op-2');
    assert.deepEqual(bridge.calls[0].args.scope.assetIds, [101, 102, 103]);
    assert.equal(bridge.calls[0].args.scope.expectedCount, 3);
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('a busy-project refusal is reported as not_started, never as a spent-outcome-unknown', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  // The desktop marks every reprobe pre-admission refusal not_started (the job
  // row never committed and no worker spawned), so the agent must not treat it
  // as "maybe already running / maybe charged".
  const bridge = await fakeBridge(() => ({ ok: false, error: 'reprobe_not_started: 项目已有运行中的任务 #5', phase: 'not_started', outcomeUnknown: false }));
  try {
    const assist = new DiscoveryAssist({ origin: bridge.origin, token: TOKEN });
    await assert.rejects(
      assist.call(store, project.id, 'reprobe_assets', { scope: { kind: 'query', expectedCount: 1 } }, 'op-3'),
      error => {
        assert.equal(error.code, 'host_discovery_task_rejected');
        assert.equal(error.details?.phase, 'not_started');
        assert.equal(error.details?.outcomeUnknown, false);
        assert.match(error.details?.reason ?? '', /项目已有运行中的任务/);
        return true;
      },
    );
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('a count-changed refusal is also not_started (strict expectedCount guard, nothing ran)', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  const bridge = await fakeBridge(() => ({ ok: false, error: 'reprobe_not_started: 复测范围数量已变化（原 10，当前 8），请刷新列表后重试', phase: 'not_started', outcomeUnknown: false }));
  try {
    const assist = new DiscoveryAssist({ origin: bridge.origin, token: TOKEN });
    await assert.rejects(
      assist.call(store, project.id, 'reprobe_assets', { scope: { kind: 'selection', assetIds: [1], expectedCount: 10 } }, 'op-4'),
      error => {
        assert.equal(error.details?.phase, 'not_started');
        assert.equal(error.details?.outcomeUnknown, false);
        return true;
      },
    );
  } finally { await bridge.close(); store.close(); cleanup(); }
});
