import test from 'node:test';
import assert from 'node:assert/strict';
import { isProjectReadQuestion, isInventoryCountQuestion } from '../src/quality-prompt.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';

const question = '请重新调用工具读取当前项目的资产总数和已保存的证书 SHA-256 去重数量，仅用两行回答。';
test('existing saved certificate counts are reads; writes and filtered counts keep their workflows', () => {
  for (const text of [question, '已保存的证书指纹有多少？', '当前项目已采集的资产总数是多少']) {
    assert.equal(isProjectReadQuestion(text), true, text);
    assert.equal(isInventoryCountQuestion(text), true, text);
  }
  for (const text of ['读取当前项目资产数量并保存到文件', '开始执行当前项目证书补采', '删除已保存的证书', '修改当前项目范围']) {
    assert.equal(isProjectReadQuestion(text), false, text);
    assert.equal(isInventoryCountQuestion(text), false, text);
  }
  for (const text of ['当前项目 example.com 下有多少资产', '当前项目按状态分组的资产数量', '为什么当前项目只有一个证书指纹',
    '当前项目状态码 200 的资产有多少', '当前项目端口 443 的资产数量', '当前项目 FOFA 来源的资产数量']) {
    assert.equal(isInventoryCountQuestion(text), false, text);
  }
});

const sse = delta => (async function* () {
  yield Buffer.from(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: delta.tool_calls ? 'tool_calls' : 'stop' }] })}\n\n`);
})();

test('the production question reads once and presents results without task DSL or repeated verification', async () => {
  const requests = [], executions = [];
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-v4-flash', apiKey: 'synthetic-test-key-000' },
    fetch: async (_url, init) => {
      const body = JSON.parse(init.body); requests.push(body);
      if (requests.length === 1) return { ok: true, body: sse({ tool_calls: [{ index: 0, id: 'stats', function: { name: 'assets__inventory_stats', arguments: '{}' } }] }) };
      return { ok: true, body: sse({ content: '根据 inventory_stats，资产 30227 条，证书指纹 1 条。' }) };
    },
  });
  runtime.bindHost({ async mcp(_token, _conversation, method, params) {
    if (method === 'tools/list') return { tools: [
      { name: 'assets__inventory_stats', inputSchema: { type: 'object' } },
      { name: 'assets__project_overview', inputSchema: { type: 'object' } },
      { name: 'host_task_update', inputSchema: { type: 'object' } },
      { name: 'host_task_verify', inputSchema: { type: 'object' } },
    ] };
    executions.push(params.name);
    return { structuredContent: { total: 30227, discoverySeeds: { certificateFingerprints: 1 } } };
  } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [
    { name: 'OVIRAPTOR_MCP_TOKEN', value: 'test' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'test' },
  ] }] });
  const result = await runtime.prompt({ sessionId, prompt: question, userText: question });
  assert.equal(result.stopReason, 'end_turn');
  assert.deepEqual(executions, ['assets__inventory_stats']);
  assert.equal(requests.length, 2);
  assert.deepEqual(requests[0].tools.map(tool => tool.function.name), ['assets__inventory_stats']);
  assert.equal(requests[1].tools, undefined);
  assert.ok(requests[1].messages.some(message => message.role === 'tool' && message.content.includes('30227')));
});
