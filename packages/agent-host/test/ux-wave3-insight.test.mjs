import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { InsightLedger, INSIGHT_FIELD_WHITELIST } from '../src/insight.mjs';

test('insight propose whitelist and commit requires userConfirmed', () => {
  const ledger = new InsightLedger();
  assert.deepEqual([...INSIGHT_FIELD_WHITELIST].sort(), ['classification', 'labels', 'note', 'tags']);
  assert.throws(() => ledger.propose({
    projectId: 1, inventoryRecordId: 'inv-1', fields: { secret: 'x' }, operationId: 'op-1',
  }), error => error.details?.insightCode === 'field_denied' || /field_denied/.test(error.code));

  const proposal = ledger.propose({
    projectId: 1,
    inventoryRecordId: 'inv-1',
    fields: { note: 'likely ssh service', tags: ['ssh'] },
    baseVersion: 3,
    operationId: 'op-propose-1',
  });
  assert.ok(proposal.proposalId);
  assert.equal(proposal.committed, false);

  assert.throws(() => ledger.commit({
    projectId: 1, proposalId: proposal.proposalId, operationId: 'op-commit-1', userConfirmed: false,
  }), error => error.details?.insightCode === 'confirm_required' || /confirm_required/.test(error.code));

  const committed = ledger.commit({
    projectId: 1, proposalId: proposal.proposalId, operationId: 'op-commit-1', userConfirmed: true,
  });
  assert.equal(committed.committed, true);
  assert.equal(committed.fields.note, 'likely ssh service');
  assert.equal(committed.persistence, 'host_ledger_stub');

  const replay = ledger.commit({
    projectId: 1, proposalId: proposal.proposalId, operationId: 'op-commit-1', userConfirmed: true,
  });
  assert.equal(replay.idempotentReplay, true);
});

test('HTTP propose + commit gate round-trip', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-ux-insight-'));
  const store = new HostStore(join(directory, 'host.sqlite'));
  const host = new AgentHost({ dataDir: directory, store });
  const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); await rm(directory, { recursive: true, force: true }); });
  const headers = { 'Content-Type': 'application/json', 'X-Oviraptor-Session': web.token };
  const project = store.createProject('Insight project');

  const proposed = await fetch(`${web.origin}/api/projects/${project.id}/insights/propose`, {
    method: 'POST', headers,
    body: JSON.stringify({
      inventoryRecordId: 'rec-9',
      fields: { classification: 'service', labels: ['web'] },
      operationId: 'http-propose-1',
    }),
  });
  assert.equal(proposed.status, 200);
  const body = await proposed.json();
  assert.ok(body.proposalId);

  const denied = await fetch(`${web.origin}/api/projects/${project.id}/insights/commit`, {
    method: 'POST', headers,
    body: JSON.stringify({ proposalId: body.proposalId, operationId: 'http-commit-1', userConfirmed: false }),
  });
  assert.equal(denied.status, 403);

  const ok = await fetch(`${web.origin}/api/projects/${project.id}/insights/commit`, {
    method: 'POST', headers,
    body: JSON.stringify({ proposalId: body.proposalId, operationId: 'http-commit-1', userConfirmed: true }),
  });
  assert.equal(ok.status, 200);
  const committed = await ok.json();
  assert.equal(committed.committed, true);
  assert.equal(committed.inventoryRecordId, 'rec-9');
});
