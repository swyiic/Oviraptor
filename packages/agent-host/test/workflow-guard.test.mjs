import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';
import { admitContext, contextFits, createBudgetAllocation, previewIntent, renderTaskReport, renderTaskReportHtml, resolveModelCapability, spendAllocation } from '../src/workflow-guard.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';

test('tool-call budget admits one reservation and keeps an unknown settle', () => {
  const store = new HostStore(':memory:');
  const project = store.createProject('Budget');
  store.setToolCallBudget(project.id, 1);
  assert.equal(store.admitToolCall(project.id, 'op-1', 'write').state, 'reserved');
  assert.throws(() => store.admitToolCall(project.id, 'op-2', 'read'), { code: 'host_budget_hard_stop' });
  assert.equal(store.settleToolCall('op-1', 'indeterminate'), 'indeterminate');
  assert.equal(store.settleToolCall('op-1', 'released'), 'indeterminate');
  assert.throws(() => store.admitToolCall(project.id, 'op-3', 'read'), { code: 'host_budget_hard_stop' });
  store.close();
});

test('a bare URL is previewed and does not start the model', async t => {
  const root = await mkdtemp(join(tmpdir(), 'ovi-intent-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  let prompts = 0;
  const runtime = { status: { pid: 3 }, async start() {}, async newSession() { prompts += 1; return { sessionId: 's' }; }, async loadSession() {}, async prompt() { prompts += 1; return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {} };
  const host = new AgentHost({ dataDir: join(root, 'host'), runtime });
  t.after(() => host.close());
  const project = host.store.createProject('Intent');
  const conversation = host.store.createConversation(project.id);
  const preview = await host.send(conversation.id, { projectId: project.id, text: 'https://example.test/asset' });
  assert.equal(preview.intent.startsScan, false);
  assert.equal(prompts, 0);
  assert.equal(host.runs.size, 0);
  const chosen = await host.send(conversation.id, { projectId: project.id, text: 'https://example.test/asset', confirmedIntent: 'prepare_authorized_scan' });
  assert.match(chosen.userMessage.content, /example\.test/);
  assert.equal(prompts, 0);
  assert.equal(previewIntent('请扫描 https://example.test/asset'), null);
});

test('draft report names unknown effects and does not claim a verified vulnerability', () => {
  const markdown = renderTaskReport({ id: 'task-1', projectId: 7, goal: 'look at sk-secretkeyvalue123456', status: 'interrupted', evidence: [{ outcomeUnknown: true, operationId: 'op-1', toolName: 'fixture__write', rawRecordId: 'raw-1' }] });
  assert.match(markdown, /draft/);
  assert.match(markdown, /op-1/);
  assert.match(markdown, /无法确认/);
  assert.doesNotMatch(markdown, /sk-secretkeyvalue123456/);
  assert.deepEqual(contextFits([{ role: 'user', content: '约束：只读' }], 20), { ok: false, reason: 'context_insufficient', bytes: Buffer.byteLength(JSON.stringify([{ role: 'user', content: '约束：只读' }])) });
  assert.match(renderTaskReportHtml({ id: 't', projectId: 1, goal: '<script>' }), /&lt;script&gt;/);
  assert.doesNotMatch(renderTaskReportHtml({ id: 't', projectId: 1, goal: '<script>' }), /<script>/);
  const unknown = resolveModelCapability({ provider: 'openai-compatible', model: 'deepseek-v4-flash' });
  assert.equal(unknown.contextWindowTokens, null);
  assert.equal(unknown.price, null);
  assert.equal(admitContext({ capability: unknown, messages: [] }).reason, 'unsupported_capability');
  const allocation = createBudgetAllocation({ total: 5, host: 2, native: 1 });
  assert.equal(allocation.pending, 2);
  assert.throws(() => spendAllocation(allocation, 'host', 3), { code: 'host_budget_hard_stop' });
  assert.throws(() => spendAllocation(allocation, 'native', 2), { code: 'host_budget_hard_stop' });
  assert.equal(spendAllocation(allocation, 'host', 2).hostSpent, 2);
});

test('the same task snapshot reuses a report version and a changed snapshot gets the next one', async t => {
  const root = await mkdtemp(join(tmpdir(), 'ovi-report-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const runtime = { status: { pid: 4 }, async start() {}, async newSession() { return { sessionId: 's' }; }, async loadSession() {}, async prompt() { return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {} };
  const host = new AgentHost({ dataDir: join(root, 'host'), runtime });
  t.after(() => host.close());
  const project = host.store.createProject('Report');
  const conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: '记录这个任务' });
  await host.runs.get(conversation.id).completion;
  const first = host.taskReport(conversation.id, { projectId: project.id, format: 'html' });
  const again = host.taskReport(conversation.id, { projectId: project.id, format: 'html' });
  assert.equal(again.reused, true);
  assert.equal(again.version, first.version);
  assert.match(again.body, /unable_to_confirm|无法确认/);
  const prepared = host.prepareTask(conversation.id, { projectId: project.id, choice: 'prepare_authorized_scan', scope: 'https://example.test/a' });
  const confirmed = host.confirmTask(conversation.id, { projectId: project.id, approvalId: prepared.id, scopeHash: prepared.scopeHash });
  assert.equal(confirmed.startsScan, false);
  assert.throws(() => host.confirmTask(conversation.id, { projectId: project.id, approvalId: prepared.id, scopeHash: '0'.repeat(64) }), { code: 'host_approval_invalid' });
  host.setBudgetAllocation(project.id, { total: 3, host: 1, native: 1, softPause: 1 });
  assert.throws(() => host.store.admitToolCall(project.id, 'soft-1', 'read'), { code: 'host_budget_paused' });
  assert.equal(host.store.allocation(project.id).pending, 1);
  const recordId = host.store.saveToolRecord({ id: 'raw-1', projectId: project.id, conversationId: conversation.id, operationId: 'op', sha256: 'ab', body: 'visible' });
  assert.equal(host.queryHistory(project.id).items.some(item => item.id === recordId), true);
  host.store.purgeProjectRecords(project.id);
  assert.equal(host.queryHistory(project.id).items.some(item => item.id === recordId), false);
  assert.equal(host.queryHistory(project.id, { includePurged: true }).items[0].visibility, 'source_purged');
  host.reserveNativeBudget(project.id, 1);
  assert.throws(() => host.reserveNativeBudget(project.id, 1), { code: 'host_budget_hard_stop' });
  const review = host.addHumanReview(conversation.id, { projectId: project.id, reportVersion: first.version, note: '人工复核：仍不能确认' });
  assert.equal(review.reportVersion, first.version);
});

test('strict mode does not call the model when the context window is unknown', async () => {
  let called = 0;
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'https://fixture.invalid/v1', model: 'fixture', apiKey: 'xai-synthetic-not-a-real-key-000', contextMode: 'strict' },
    fetch: async () => { called += 1; throw new Error('sent'); },
  });
  const { sessionId } = await runtime.newSession();
  const result = await runtime.prompt({ sessionId, prompt: 'hello', userText: 'hello' });
  assert.equal(called, 0);
  assert.equal(result.stopReason, 'unsupported_capability');
});
