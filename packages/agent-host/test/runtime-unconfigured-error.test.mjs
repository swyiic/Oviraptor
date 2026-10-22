import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { describeAgentHostError } from '../../../src/features/agentHost/errors.ts';

test('unconfigured runtime points to chat model setup and retains the diagnostic code', () => {
  const view = describeAgentHostError('host_runtime_unconfigured');
  assert.equal(view.code, 'host_runtime_unconfigured');
  assert.equal(view.title[0], '尚未配置聊天模型');
  assert.match(view.hint[0], /配置中心.*聊天模型.*登录.*API key/);
  assert.match(view.hint[0], /消息仍可保存/);
  assert.doesNotMatch(view.hint[0], /请重试|切换运行时|Agent runtime/);
  assert.match(view.hint[1], /Configuration Center.*Chat models/);
  assert.match(view.hint[1], /Messages can still be saved/);
});

test('unconfigured Host saves user messages while task continuation returns the mapped code', async () => {
  const dataDir = await mkdtemp(join(await realpath(tmpdir()),'ovi-unconfigured-chat-'));
  const host = new AgentHost({ dataDir, runtimeSelection: 'none', providerFetch: () => assert.fail('unconfigured Host must not call a model') });
  try {
    const project = host.store.createProject('Unconfigured model fixture');
    const conversation = host.store.createConversation(project.id);
    const saved = await host.send(conversation.id, { text: '请先保存我的任务，稍后配置聊天模型。' });
    assert.equal(saved.savedOnly, true);
    assert.equal(saved.runtime.state, 'unconfigured');
    assert.equal(host.store.messages(conversation.id).length, 1);
    assert.equal(host.store.messages(conversation.id)[0].content, '请先保存我的任务，稍后配置聊天模型。');
    const beforeRevision = host.task(conversation.id).revision;
    await assert.rejects(host.continueTask(conversation.id), error => {
      const view = describeAgentHostError(error.code);
      assert.equal(view.code, 'host_runtime_unconfigured');
      assert.equal(view.title[0], '尚未配置聊天模型');
      return true;
    });
    assert.equal(host.store.messages(conversation.id).length, 1, 'rejected continuation must not enqueue a phantom retry');
    assert.equal(host.task(conversation.id).revision, beforeRevision);
    assert.equal(host.runs.size, 0);
  } finally { await host.close(); await rm(dataDir, { recursive: true, force: true }); }
});
