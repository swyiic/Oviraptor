import test from 'node:test';
import assert from 'node:assert/strict';
import { HostStore } from '../src/store.mjs';
import { runtimeMetrics } from '../src/metrics.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

test('metrics keep evidence groups separate, include unsuccessful attempts, and distinguish missing observations', () => {
  const store = new HostStore(':memory:');
  try {
    store.metrics('fixture-ok', 'c', { runtimeEvidence: 'synthetic-or-custom', outcome: 'completed', coldStartMs: 10, firstTextMs: 2, firstToolMs: null, totalMs: 30, hostTotalMs: 40 });
    store.metrics('fixture-cancel', 'c', { runtimeEvidence: 'synthetic-or-custom', outcome: 'cancelled', outcomeUnknown: false, totalMs: 20 });
    store.metrics('fixture-unknown', 'c', { runtimeEvidence: 'synthetic-or-custom', outcome: 'failed', outcomeUnknown: true, totalMs: 50 });
    store.metrics('unverified-official', 'c', { runtimeEvidence: 'official-cli-unverified', outcome: 'completed', totalMs: 500 });
    const report = runtimeMetrics(store);
    assert.equal(report.total, 4); assert.equal(report.truncated, false); assert.equal(report.officialPerformanceVerified, false);
    const fixture = report.groups.find(group => group.evidence === 'synthetic-or-custom');
    assert.equal(fixture.attempts, 3); assert.equal(fixture.successes, 1); assert.equal(fixture.successRate, 1 / 3); assert.equal(fixture.unknownOutcomes, 1);
    assert.deepEqual(fixture.timings.firstTextMs, { observations: 1, mean: 2 });
    assert.deepEqual(fixture.timings.firstToolMs, { observations: 0, mean: null });
    assert.equal(runtimeMetrics(store, 2).truncated, true); assert.equal(runtimeMetrics(store, 2).returned, 2);
    for (const limit of [0, 1001, NaN, 1.5]) assert.throws(() => runtimeMetrics(store, limit), /host_metrics_limit_invalid/);
  } finally { store.close(); }
});

test('metrics HTTP export needs the host token and is bounded before reading stored observations', async t => {
  const store = new HostStore(':memory:');
  const apps = { async close() {} };
  const host = new AgentHost({ dataDir: '/tmp/unused-metrics-test', store, apps });
  const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); });
  assert.equal((await fetch(`${web.origin}/api/runtime/metrics`)).status, 403);
  const headers = { 'X-Oviraptor-Session': web.token };
  assert.equal((await fetch(`${web.origin}/api/runtime/metrics?limit=1001`, { headers })).status, 400);
  const response = await fetch(`${web.origin}/api/runtime/metrics?limit=100`, { headers });
  assert.equal(response.status, 200); assert.deepEqual((await response.json()).groups, []);
});
