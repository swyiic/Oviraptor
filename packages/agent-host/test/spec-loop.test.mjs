import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';

test('the spec loop previews, confirms without scanning, pauses on budget, recalls one project, and regenerates the report', async t => {
  const root = await mkdtemp(join(tmpdir(), 'ovi-spec-loop-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  let prompts = 0;
  const runtime = {
    status: { pid: 20 },
    async start() {},
    async newSession() { return { sessionId: 'spec-session' }; },
    async loadSession() {},
    bindHost(host) { this.host = host; },
    async prompt() {
      prompts += 1;
      const run = [...this.host.runs.values()][0];
      const token = [...this.host.mcpScopes.keys()][0];
      await this.host.mcp(token, run.conversationId, 'tools/call', { name: 'fixture__read', arguments: {} });
      return { stopReason: 'end_turn', metrics: {} };
    },
    async cancel() { return true; },
    async close() {},
  };
  const apps = {
    store: { application: () => ({ manifest: {} }) },
    capabilities: () => [{ name: 'fixture::read', permission: 'fixture.read', appId: 'fixture', grantRevision: 1 }],
    async call() { return { content: [{ type: 'text', text: '{}' }] }; },
    async close() {},
  };
  const host = new AgentHost({ dataDir: join(root, 'host'), runtime, apps });
  t.after(() => host.close());
  const project = host.store.createProject('Spec loop');
  const other = host.store.createProject('Other project');
  const conversation = host.store.createConversation(project.id);
  const preview = await host.send(conversation.id, { projectId: project.id, text: 'https://127.0.0.1/synthetic' });
  assert.equal(preview.intent.startsScan, false);
  assert.equal(prompts, 0);
  const prepared = host.prepareTask(conversation.id, { projectId: project.id, choice: 'prepare_authorized_scan', scope: 'https://127.0.0.1/synthetic' });
  host.setBudgetAllocation(project.id, { total: 4, host: 2, native: 1, softPause: 0, modelCap: 2 });
  const receipt = JSON.parse(await readFile(join(root, 'host', 'owner-allocation.json'), 'utf8'));
  assert.equal(receipt.nativeCap, 1);
  assert.equal(receipt.pending, 1);
  assert.equal(receipt.price, null);
  assert.equal(receipt.limitsSource, 'runtime_config');
  assert.throws(() => host.confirmTask(conversation.id, { projectId: project.id, approvalId: prepared.id, scopeHash: prepared.scopeHash }), { code: 'host_approval_stale' });
  const current = host.prepareTask(conversation.id, { projectId: project.id, choice: 'prepare_authorized_scan', scope: 'https://127.0.0.1/synthetic' });
  const confirmed = host.confirmTask(conversation.id, { projectId: project.id, approvalId: current.id, scopeHash: current.scopeHash });
  assert.equal(confirmed.startsScan, false);
  const standard = host.bindPreset(conversation.id, { projectId: project.id, presetId: 'standard', actor: 'user' });
  assert.equal(standard.executionPreset.presetRef.id, 'standard');
  assert.throws(() => host.bindPreset(conversation.id, { projectId: project.id, presetId: 'deep', actor: 'agent' }), { code: 'host_preset_upgrade_denied' });
  host.setBudgetAllocation(project.id, { total: 4, host: 1, native: 1, softPause: 1, modelCap: 2 });
  await host.send(conversation.id, { projectId: project.id, text: '按标准预设核对这一条合成读取' });
  await host.runs.get(conversation.id)?.completion;
  assert.equal(host.task(conversation.id).status, 'paused_budget');
  assert.equal(prompts, 1);
  host.recordSummary(project.id, conversation.id, { sourceIds: ['constraint-1'], body: '排除项：不扫描公开资料页。反证：角色不同不是越权。' });
  host.recordSummary(other.id, conversation.id, { sourceIds: ['foreign'], body: '其他项目的排除项' });
  const recalled = host.recallSummaries(project.id);
  assert.match(recalled.items.at(-1).body, /排除项/);
  assert.equal(recalled.items.some(item => item.body.includes('其他项目')), false);
  const before = prompts;
  const report = host.taskReport(conversation.id, { projectId: project.id, format: 'html' });
  const again = host.taskReport(conversation.id, { projectId: project.id, format: 'html' });
  assert.equal(prompts, before);
  assert.equal(again.reused, true);
  assert.match(report.body, /unable_to_confirm|无法确认/);
  host.addHumanReview(conversation.id, { projectId: project.id, reportVersion: report.version, note: '复核后仍不能把角色差异写成漏洞' });
});
