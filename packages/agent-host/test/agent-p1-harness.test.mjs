import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  answerModeOptions,
  prefersResponsesApi,
  normalizeReasoningEffort,
  buildQualitySystemPrompt,
  looksLikeDataQuestion,
} from '../src/quality-prompt.mjs';
import {
  emptyAgentState,
  setGoal,
  absorbToolResults,
  shouldBlockEmptyCompletion,
  shouldInjectReviewer,
  isDeadEnd,
  formatAgentStatePromptBlock,
  REVIEWER_HINT,
  DEEP_THINK_HINT,
} from '../src/agent-state.mjs';
import {
  compressMemory,
  normalizeMemory,
  recordTurn,
  readConversationMemory,
  MEMORY_LIMITS,
} from '../src/memory.mjs';
import { AgentHost } from '../src/service.mjs';
import {
  OpenAiCompatibleRuntime,
  readResponsesStream,
  PLANNING_GUIDANCE,
} from '../src/provider.mjs';

const KEY = 'xai-synthetic-not-a-real-key-000';
const sse = chunks => (async function* () {
  for (const c of chunks) {
    if (typeof c === 'string' && c.startsWith('event:')) {
      yield Buffer.from(`${c}\n`);
    } else {
      yield Buffer.from(`data: ${typeof c === 'string' ? c : JSON.stringify(c)}\n\n`);
    }
  }
})();

test('P1 answerMode enables Responses + reasoning medium/low', () => {
  const q = answerModeOptions('quality');
  const s = answerModeOptions('speed');
  assert.equal(q.useResponsesApi, true);
  assert.equal(q.reasoningEffort, 'medium');
  assert.equal(q.forceToolOnDataQuestion, true);
  assert.equal(s.reasoningEffort, 'low');
  assert.equal(normalizeReasoningEffort(undefined, 'quality'), 'medium');
  assert.equal(prefersResponsesApi({ provider: 'xai', baseUrl: 'https://api.x.ai/v1' }), true);
  assert.equal(prefersResponsesApi({ provider: 'openai-compatible', baseUrl: 'http://127.0.0.1:8080/v1' }), false);
  const prompt = buildQualitySystemPrompt({ answerMode: 'quality' });
  assert.match(prompt, /reasoning\.effort=medium/);
  assert.match(prompt, /禁止把假设当事实|引用工具|未知/);
  assert.ok(prompt.includes(PLANNING_GUIDANCE) || /执行策略/.test(prompt));
});

test('harness: fact vs hypothesis and scoped failure', () => {
  let state = setGoal(emptyAgentState(), '当前项目有多少资产？');
  assert.match(state.goal, /数据问题|资产/);
  assert.ok(state.openIntents.length >= 1);
  state = absorbToolResults(state, [
    { name: 'assets__inventory_query', ok: true, content: '{"total":3}', arguments: {} },
  ]);
  assert.ok(state.confirmedFacts.some(f => /确认.*inventory_query/.test(f) && /total.:3/.test(f)));
  state = absorbToolResults(state, [
    { name: 'assets__inventory_query', ok: false, content: 'timeout', arguments: { limit: 1000 } },
  ]);
  assert.ok(state.hypotheses.some(h => /≠证明目标不存在|本次失败/.test(h)));
  const block = formatAgentStatePromptBlock(state);
  assert.match(block, /confirmedFacts/);
  assert.match(block, /hypotheses/);
  assert.match(block, /NOT facts|不是事实|禁止/);
});

test('harness: block empty completion on unmet data goal', () => {
  const state = setGoal(emptyAgentState(), '列出已装应用');
  const blocked = shouldBlockEmptyCompletion(state, {
    answerMode: 'quality',
    usedToolThisPrompt: false,
    forcedToolHint: false,
    assistantText: '暂时没什么。',
    userText: '列出已装应用',
  });
  assert.equal(blocked.block, true);
  assert.match(blocked.hint, /禁止空结束|工具/);

  const ok = shouldBlockEmptyCompletion(state, {
    answerMode: 'quality',
    usedToolThisPrompt: true,
    assistantText: '根据工具，有 2 个应用',
    userText: '列出已装应用',
  });
  assert.equal(ok.block, false);

  const speed = shouldBlockEmptyCompletion(state, {
    answerMode: 'speed',
    usedToolThisPrompt: false,
    assistantText: 'ok',
    userText: '列出已装应用',
  });
  assert.equal(speed.block, false);
});

test('harness: dead-end detection and reviewer cadence', () => {
  let state = emptyAgentState();
  state = absorbToolResults(state, [
    { name: 'x__y', ok: false, content: 'e', arguments: { a: 1 } },
    { name: 'x__y', ok: false, content: 'e', arguments: { a: 1 } },
  ]);
  assert.equal(isDeadEnd(state), true);
  assert.equal(shouldInjectReviewer(state, { answerMode: 'quality', step: 2 }), true);
  assert.match(REVIEWER_HINT, /偏离|假设/);
  assert.match(DEEP_THINK_HINT, /第一性原理/);
});

test('memory compresses older turns into confirmed facts vs hypotheses', () => {
  const turns = [];
  for (let i = 0; i < 8; i++) {
    turns.push({ role: 'user', text: `q-${i}`, tools: [] });
    turns.push({
      role: 'assistant',
      text: `a-${i}`,
      tools: [{ name: 'assets__inventory_query', status: 'completed', summary: `{"n":${i}}` }],
    });
  }
  const compressed = compressMemory({ turns, facts: [], hypotheses: [] }, { keepRecent: 4 });
  assert.equal(compressed.compressed, true);
  assert.ok(compressed.turns.length <= 4);
  assert.ok(compressed.facts.length >= 1);
  assert.ok(compressed.facts.some(f => /确认/.test(f)));
  assert.ok(Buffer.byteLength(JSON.stringify(compressed)) <= MEMORY_LIMITS.maxTotalBytes);
});

test('host settings expose reasoningEffort', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'ovi-p1-settings-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const host = new AgentHost({ dataDir: directory, runtimeSelection: 'none' });
  t.after(() => host.close());
  const settings = host.settings();
  assert.equal(settings.answerMode, 'quality');
  assert.equal(settings.answerModeOptions.reasoningEffort, 'medium');
  host.setAnswerMode('speed');
  assert.equal(host.settings().answerModeOptions.reasoningEffort, 'low');
});

test('Responses SSE parser collects text and function calls', async () => {
  const deltas = [];
  const out = await readResponsesStream(sse([
    'event: response.output_text.delta',
    { type: 'response.output_text.delta', delta: '根据' },
    'event: response.output_text.delta',
    { type: 'response.output_text.delta', delta: '查询' },
    'event: response.function_call_arguments.delta',
    { type: 'response.function_call_arguments.delta', output_index: 0, call_id: 'c1', name: 'assets__inventory_query', delta: '{"limit"' },
    'event: response.function_call_arguments.done',
    { type: 'response.function_call_arguments.done', output_index: 0, call_id: 'c1', name: 'assets__inventory_query', arguments: '{"limit":5}' },
    'event: response.completed',
    { type: 'response.completed', response: { id: 'resp_1', output: [] } },
  ]), d => deltas.push(d));
  assert.deepEqual(deltas, ['根据', '查询']);
  assert.equal(out.toolCalls[0].name, 'assets__inventory_query');
  assert.equal(out.toolCalls[0].arguments, '{"limit":5}');
  assert.equal(out.responseId, 'resp_1');
});

test('quality runtime uses /responses with reasoning.effort and blocks empty data answer once', async () => {
  const requests = [];
  const fetchMock = async (url, init) => {
    const body = JSON.parse(init.body);
    requests.push({ url, body });
    const n = requests.length;
    if (n === 1) {
      // Model tries to end without tools
      return {
        ok: true, status: 200,
        body: sse([
          { type: 'response.output_text.delta', delta: '大概有一些资产吧。' },
          { type: 'response.completed', response: { id: 'r1', output: [{ type: 'message', content: [{ type: 'output_text', text: '大概有一些资产吧。' }] }] } },
        ]),
      };
    }
    if (n === 2) {
      // After harness block, model calls tool
      return {
        ok: true, status: 200,
        body: sse([
          { type: 'response.function_call_arguments.done', output_index: 0, call_id: 'c1', name: 'assets__inventory_query', arguments: '{}' },
          { type: 'response.completed', response: { id: 'r2', output: [{ type: 'function_call', call_id: 'c1', name: 'assets__inventory_query', arguments: '{}' }] } },
        ]),
      };
    }
    return {
      ok: true, status: 200,
      body: sse([
        { type: 'response.output_text.delta', delta: '根据 inventory_query，共 3 个资产。' },
        { type: 'response.completed', response: { id: 'r3', output: [] } },
      ]),
    };
  };
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'xai', baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7', apiKey: KEY },
    fetch: fetchMock,
    answerMode: 'quality',
  });
  let called = 0;
  runtime.bindHost({
    async mcp(_t, _c, method) {
      if (method === 'tools/list') return { tools: [{ name: 'assets__inventory_query', description: 'q', inputSchema: { type: 'object' } }] };
      called += 1;
      return { content: [{ type: 'text', text: JSON.stringify({ total: 3 }) }] };
    },
  });
  const { sessionId } = await runtime.newSession({
    mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'tok' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'c' }] }],
  });
  const result = await runtime.prompt({ sessionId, prompt: '当前项目有多少资产？' });
  assert.equal(result.stopReason, 'end_turn');
  assert.equal(result.metrics.api, 'responses');
  assert.equal(result.metrics.reasoningEffort, 'medium');
  assert.ok(result.metrics.forcedEmptyBlock || result.metrics.forcedToolHint);
  assert.match(requests[0].url, /\/responses$/);
  assert.equal(requests[0].body.reasoning.effort, 'medium');
  assert.equal(requests[0].body.stream, true);
  assert.equal(requests[0].body.parallel_tool_calls, true);
  assert.ok(called >= 1);
});

test('speed mode uses reasoning.effort low on responses', async () => {
  const requests = [];
  const fetchMock = async (url, init) => {
    requests.push({ url, body: JSON.parse(init.body) });
    return {
      ok: true, status: 200,
      body: sse([
        { type: 'response.output_text.delta', delta: '你好' },
        { type: 'response.completed', response: { id: 's1', output: [] } },
      ]),
    };
  };
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'xai', baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7', apiKey: KEY },
    fetch: fetchMock,
    answerMode: 'speed',
  });
  runtime.bindHost({ async mcp() { return { tools: [] }; } });
  const { sessionId } = await runtime.newSession();
  await runtime.prompt({ sessionId, prompt: '你好' });
  assert.equal(requests[0].body.reasoning.effort, 'low');
});

test('session memory recordTurn writes confirmed facts from tools', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'ovi-p1-mem-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const host = new AgentHost({ dataDir: join(directory, 'h'), runtimeSelection: 'none' });
  t.after(() => host.close());
  const project = host.store.createProject('p');
  const conversation = host.store.createConversation(project.id);
  recordTurn(host.store, conversation.id, {
    userText: '多少资产',
    assistantText: '3',
    tools: [{ name: 'assets__inventory_query', status: 'completed', summary: '{"total":3}' }],
  });
  const memory = readConversationMemory(host.store, conversation.id);
  assert.ok(memory.facts.some(f => /确认.*inventory_query/.test(f)));
  assert.equal(looksLikeDataQuestion('多少资产'), true);
});
