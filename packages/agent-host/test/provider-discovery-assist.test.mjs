import assert from 'node:assert/strict';
import test from 'node:test';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { discoveryAssistIntent } from '../src/quality-prompt.mjs';

const names = ['discovery_assist_start','discovery_assist_status','discovery_assist_pause','discovery_review_read','discovery_scope_propose'];
function response(requestNumber, entries, text = '后台任务状态已核对，新增范围请在面板一次确认。') {
  return { ok: true, status: 200, body: (async function* () {
    yield `data: ${JSON.stringify({ choices: [{ delta: entries ? { tool_calls: entries.map(([name,args],index) => ({ index, id: `${requestNumber}-${index}`, function: { name: 'assets__' + name, arguments: JSON.stringify(args) } })) } : { content: text }, finish_reason: entries ? 'tool_calls' : 'stop' }] })}\n\n`;
  })() };
}
async function run(text, fetchStep, execute = async () => ({ structuredContent: { runId: 'saved-review', phase: 'analyzing', review: { run: { requests: 1 } } } })) {
  const requests = [], calls = [];
  const runtime = new OpenAiCompatibleRuntime({ config: { provider: 'openai-compatible', baseUrl: 'https://fixture.invalid/v1', model: 'fixture-model', apiKey: 'synthetic' }, fetch: async (_url, input) => {
    const body = JSON.parse(input.body); requests.push(body); return fetchStep(body, requests.length);
  } });
  runtime.bindHost({ async mcp(_token, _conversation, method, params) {
    if (method === 'tools/list') return { tools: [...names.map(name => ({ name: 'assets__' + name, inputSchema: { type: 'object', properties: {} } })), { name: 'assets__certificate_task_start' }, { name: 'assets__discovery_enrichment_start' }, { name: 'hunter' }, { name: 'host_task_update' }] };
    calls.push(params); return execute(params);
  } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'scope' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'conversation' }] }] });
  const result = await runtime.prompt({ sessionId, prompt: text, userText: text });
  await runtime.close(); return { result, requests, calls };
}
test('whole-project AI start performs one owner control and one status, suppresses duplicate controls and exposes no paid or scope-confirm tool', async () => {
  const text = '开始 AI 发现，按中国移动集团从整个项目证书提炼种子';
  const result = await run(text, (body, n) => {
    assert.equal(body.parallel_tool_calls, n < 3 ? false : undefined);
    if (n === 1) {
      assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_start']);
      return response(n, [['discovery_assist_start',{ organization: '中国移动集团', includeIsolated: false }],['discovery_assist_start',{ organization: '中国移动集团', includeIsolated: false }]]);
    }
    if (n === 2) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_status']); return response(n, [['discovery_assist_status',{ projectId: 999, taskId: 'guessed' }]]); }
    assert.equal(body.tools, undefined); return response(n, null);
  });
  assert.equal(result.result.stopReason, 'end_turn'); assert.equal(result.requests.length, 3);
  assert.deepEqual(result.calls.map(c => c.name), ['assets__discovery_assist_start','assets__discovery_assist_status']);
  assert.deepEqual(result.calls[0].arguments, { organization: '中国移动集团', includeIsolated: false });
  assert.deepEqual(result.calls[1].arguments, {});
});
test('read-only AI progress requests cannot expose start or pause even when the old task goal mentioned collection', async () => {
  const text = '只读查看 AI 发现进度，不要启动或暂停任务';
  assert.equal(discoveryAssistIntent(text).action, 'status');
  const result = await run(text, (body, n) => {
    if (n === 1) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_status']); return response(n, [['discovery_assist_status',{}]]); }
    assert.equal(body.tools, undefined); return response(n, null, '已读取当前 AI 发现状态。');
  });
  assert.equal(result.requests.length, 2); assert.deepEqual(result.calls.map(c => c.name), ['assets__discovery_assist_status']);
});
test('an AI discovery control failure is checked once and never automatically replayed', async () => {
  const result = await run('暂停全项目 AI 发现', (body, n) => {
    if (n === 1) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_pause']); return response(n, [['discovery_assist_pause',{ projectId: 999 }]]); }
    if (n === 2) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_status']); return response(n, [['discovery_assist_status',{}]]); }
    assert.equal(body.tools, undefined); return response(n, null, '暂停请求响应未知，已核对后台状态，不重发。');
  }, async params => {
    if (params.name.endsWith('_pause')) throw Object.assign(Error('unknown'), { code: 'host_discovery_task_outcome_unknown', details: { outcomeUnknown: true, operationId: 'original' } });
    return { structuredContent: { phase: 'pausing', runId: 'saved-review' } };
  });
  assert.equal(result.requests.length, 3); assert.equal(result.calls.filter(c => c.name.endsWith('_pause')).length, 1);
  assert.deepEqual(result.calls[0].arguments, {});
});
