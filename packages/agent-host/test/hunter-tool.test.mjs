import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { DiscoveryAssist, DISCOVERY_ASSIST_TOOLS, DISCOVERY_PASSTHROUGH_TOOLS } from '../src/discovery-assist.mjs';

const TOKEN = 'a'.repeat(64);

function tempStore() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-hunter-'));
  return { store: new HostStore(join(dir, 'host.sqlite')), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

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

test('hunter_expansion_start maps to the hunter_start desktop action and is a write passthrough', () => {
  assert.equal(DISCOVERY_ASSIST_TOOLS.get('hunter_expansion_start'), 'hunter_start');
  assert.equal(DISCOVERY_PASSTHROUGH_TOOLS.has('hunter_expansion_start'), true);
});

test('a funded start forwards the seed scope and cap, and returns the query id unshaped', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  const bridge = await fakeBridge(body => ({ ok: true, result: { projectId: body.projectId, queryId: 'hx-1', cap: 150, started: true } }));
  try {
    const assist = new DiscoveryAssist({ origin: bridge.origin, token: TOKEN });
    const args = { seedScope: { kind: 'domain', target: 'sub.example.com' }, maxPoints: 150 };
    const result = await assist.call(store, project.id, 'hunter_expansion_start', args, 'op-1');
    assert.equal(bridge.calls[0].action, 'hunter_start');
    assert.equal(bridge.calls[0].projectId, 7);
    assert.deepEqual(bridge.calls[0].args, args);
    assert.equal(result.structuredContent.queryId, 'hx-1');
    assert.equal(result.structuredContent.cap, 150);
    assert.equal(result.structuredContent.humanReview, undefined);
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('every hard-gate denial surfaces not_started (nothing spent), never outcome-unknown', async () => {
  // The desktop runs the gate (allow ∧ key ∧ account-match ∧ within-budget ∧
  // idle) before prepare/spawn, so a refusal never spent a point.
  const denials = [
    'hunter_not_authorized', 'hunter_key_missing', 'hunter_account_changed',
    'hunter_budget_unset', 'hunter_budget_exhausted', 'hunter_points_over_budget',
    'hunter_project_busy',
  ];
  for (const code of denials) {
    const { store, cleanup } = tempStore();
    const project = store.createProject('p', { externalId: '7' });
    const bridge = await fakeBridge(() => ({ ok: false, error: code, phase: 'not_started', outcomeUnknown: false }));
    try {
      const assist = new DiscoveryAssist({ origin: bridge.origin, token: TOKEN });
      await assert.rejects(
        assist.call(store, project.id, 'hunter_expansion_start', { seedScope: { kind: 'domain', target: 'x.example.com' }, maxPoints: 10 }, 'op'),
        error => {
          assert.equal(error.code, 'host_discovery_task_rejected', `${code} should be a rejection`);
          assert.equal(error.details?.phase, 'not_started', `${code} must be not_started`);
          assert.equal(error.details?.outcomeUnknown, false, `${code} must not be outcome-unknown`);
          assert.equal(error.details?.reason, code);
          return true;
        },
      );
    } finally { await bridge.close(); store.close(); cleanup(); }
  }
});
