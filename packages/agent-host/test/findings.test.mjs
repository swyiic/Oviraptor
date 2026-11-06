import test from 'node:test';
import assert from 'node:assert/strict';
import { HostStore } from '../src/store.mjs';
import { recordFindingFromTool } from '../src/findings.mjs';

function setup() {
  const store = new HostStore(':memory:');
  const project = store.createProject('p', { externalId: '1' });
  const conv = store.createConversation(project.id, 'scan');
  return { store, project, conv };
}

test('record_finding persists a structured vuln (http req/resp + chain) as unverified', () => {
  const { store, project, conv } = setup();
  const r = recordFindingFromTool(store, conv.id, project.id, {
    title: '越权读取他人订单', severity: 'high', cvss: '8.1', cwe: 'CWE-639',
    evidence: '把 userId 改成 1002 返回了别人的数据',
    chainContext: '信息泄露 + 本越权 + 默认口令 → 账户接管',
    businessImpact: '可批量拖取全量用户订单',
    pocScript: 'decrypt_and_replay.js',
    httpRequest: { method: 'GET', url: 'https://t/api/order?userId=1002', headers: { Cookie: 'sid=x' }, body: '' },
    httpResponse: { status: 200, headers: { 'Content-Type': 'application/json' }, body: '{"owner":"someone-else"}' },
  });
  assert.equal(r.ok, true);
  assert.equal(r.status, 'unverified');
  const list = store.findings(conv.id);
  assert.equal(list.length, 1);
  const f = list[0];
  assert.equal(f.title, '越权读取他人订单');
  assert.equal(f.severity, 'high');
  assert.equal(f.status, 'unverified');
  assert.equal(f.record.http_request.method, 'GET');
  assert.equal(f.record.http_response.status, 200);
  assert.match(f.record.chain_context, /账户接管/);
});

test('empty title rejected; severity normalized', () => {
  const { store, project, conv } = setup();
  assert.equal(recordFindingFromTool(store, conv.id, project.id, { title: '  ', severity: 'high' }).ok, false);
  const r = recordFindingFromTool(store, conv.id, project.id, { title: 't', severity: 'bogus' });
  assert.equal(store.finding(r.id).severity, 'info'); // unknown severity → info
});

test('decideFinding confirms once; status filter works', () => {
  const { store, project, conv } = setup();
  const r = recordFindingFromTool(store, conv.id, project.id, { title: 't', severity: 'low' });
  assert.equal(store.findings(conv.id, 'unverified').length, 1);
  const decided = store.decideFinding(r.id, true);
  assert.equal(decided.status, 'confirmed');
  assert.equal(store.findings(conv.id, 'confirmed').length, 1);
  assert.equal(store.findings(conv.id, 'unverified').length, 0);
  assert.throws(() => store.decideFinding(r.id, false)); // already decided
  assert.equal(store.deleteFinding(r.id), true);
  assert.equal(store.findings(conv.id).length, 0);
});
