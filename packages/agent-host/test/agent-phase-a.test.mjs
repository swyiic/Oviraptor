import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  humanizeToolTitle,
  DATA_TOOL_HINT,
  QUALITY_SYSTEM_PROMPT,
  isDataReadTool,
} from '../src/quality-prompt.mjs';
import {
  emptyAgentState,
  setGoal,
  verifyBeforeEnd,
  formatAgentStatePromptBlock,
} from '../src/agent-state.mjs';
import { compressSessionMessages, SESSION_MESSAGE_LIMITS } from '../src/memory.mjs';
import { AgentHost } from '../src/service.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';

const KEY = 'xai-synthetic-not-a-real-key-000';
const sse = chunks => (async function* () {
  for (const c of chunks) yield Buffer.from(`data: ${typeof c === 'string' ? c : JSON.stringify(c)}\n\n`);
})();

test('Phase A: tool true-names aligned in prompt / hint / humanize / isDataReadTool', () => {
  assert.match(QUALITY_SYSTEM_PROMPT, /inventory_stats|inventory_query/);
  assert.match(QUALITY_SYSTEM_PROMPT, /task_status|journal_read/);
  assert.match(QUALITY_SYSTEM_PROMPT, /不要调用不存在的旧名|inventory_stats/);
  assert.match(DATA_TOOL_HINT, /inventory_stats|task_status|journal_read/);
  assert.doesNotMatch(DATA_TOOL_HINT, /saved_assets_\*|scan_list/);
  assert.equal(humanizeToolTitle('assets__inventory_stats'), '正在统计资产数量');
  assert.equal(humanizeToolTitle('scanning__task_status'), '正在读取任务状态');
  assert.equal(humanizeToolTitle('scanning__journal_read'), '正在读取任务 journal');
  assert.equal(humanizeToolTitle('assets__inventory_search'), '正在搜索资产记录');
  assert.equal(isDataReadTool('assets__inventory_query'), true);
  assert.equal(isDataReadTool('scanning__task_status'), true);
  assert.equal(isDataReadTool('scanning__journal_read'), true);
  assert.equal(isDataReadTool('assets__exposure_overview'), true);
});

test('Phase A: setGoal uses only raw user sentence (not turn prompt blob)', () => {
  const blob = [
    'Host-selected project (literal untrusted data): {"id":"p"}',
    'Host capability summary: inventory_query task_status',
    '本轮为质量档：事实先查工具再答',
    '',
    '当前项目有多少资产？',
  ].join('\n');
  const polluted = setGoal(emptyAgentState(), blob);
  assert.ok(polluted.goal.length > 80 || /Host-selected|capability/.test(polluted.goal), 'control: full blob pollutes goal');

  const clean = setGoal(emptyAgentState(), '当前项目有多少资产？');
  assert.match(clean.goal, /资产/);
  assert.ok(!/Host-selected|capability summary/.test(clean.goal));
  assert.ok(clean.goal.length < 120);
  assert.ok(clean.openIntents.some(i => /需工具核实/.test(i)));
});

test('Phase A: runtime prompt setGoal receives userText not full prompt', async () => {
  const requests = [];
  const fetchMock = async (_url, init) => {
    requests.push(JSON.parse(init.body));
    return {
      ok: true, status: 200,
      body: sse([{ choices: [{ delta: { content: '你好' }, finish_reason: 'stop' }] }]),
    };
  };
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'http://127.0.0.1:9/v1', model: 'm', apiKey: KEY },
    fetch: fetchMock,
    answerMode: 'quality',
  });
  runtime.bindHost({ async mcp() { return { tools: [] }; } });
  const { sessionId } = await runtime.newSession();
  const full = 'Host-selected project: {}\n\n质量档提醒\n\n你好';
  await runtime.prompt({ sessionId, prompt: full, userText: '你好' });
  const session = runtime.sessions.get(sessionId);
  assert.match(session.harness.goal, /你好|回答用户/);
  assert.ok(!/Host-selected|质量档提醒/.test(session.harness.goal));
  const block = formatAgentStatePromptBlock(session.harness);
  assert.ok(!/Host-selected project/.test(block) || block.length < 500);
});

test('Phase A: compressSessionMessages keeps recent window and folds old tools', () => {
  const messages = [{ role: 'system', content: 'sys' }];
  for (let i = 0; i < 8; i++) {
    messages.push({ role: 'user', content: `q-${i}` });
    messages.push({
      role: 'assistant',
      content: `a-${i}`,
      tool_calls: [{ id: `c${i}`, type: 'function', function: { name: 'assets__inventory_query', arguments: '{}' } }],
    });
    messages.push({ role: 'tool', tool_call_id: `c${i}`, name: 'assets__inventory_query', content: JSON.stringify({ n: i, pad: 'x'.repeat(100) }) });
  }
  const packed = compressSessionMessages(messages, { keepRecentUserTurns: 4 });
  assert.equal(packed.compressed, true);
  assert.ok(packed.foldedFacts.length >= 1);
  assert.ok(packed.foldedFacts.some(f => /确认/.test(f)));
  const users = packed.messages.filter(m => m.role === 'user');
  assert.ok(users.length <= 5); // marker + 4 recent
  assert.ok(Buffer.byteLength(JSON.stringify(packed.messages)) <= SESSION_MESSAGE_LIMITS.maxTotalBytes);
  assert.ok(packed.messages.some(m => m.role === 'system'));
});

test('Phase A: verifyBeforeEnd blocks data end without evidence once', () => {
  const state = setGoal(emptyAgentState(), '当前项目有多少资产？');
  const fail = verifyBeforeEnd(state, {
    answerMode: 'quality',
    usedToolThisPrompt: false,
    assistantText: '大概有不少资产。',
    userText: '当前项目有多少资产？',
    toolNamesThisPrompt: [],
  });
  assert.equal(fail.ok, false);
  assert.match(fail.hint, /verifier|验收|证据/);

  const withTools = {
    ...state,
    confirmedFacts: ['[确认·inventory_query] {"total":3}'],
  };
  const pass = verifyBeforeEnd(withTools, {
    answerMode: 'quality',
    usedToolThisPrompt: true,
    assistantText: '根据 inventory_query，共 3 个资产。',
    userText: '当前项目有多少资产？',
    toolNamesThisPrompt: ['assets__inventory_query'],
  });
  assert.equal(pass.ok, true);

  const speed = verifyBeforeEnd(state, {
    answerMode: 'speed',
    usedToolThisPrompt: false,
    assistantText: '99',
    userText: '当前项目有多少资产？',
  });
  assert.equal(speed.ok, true);
});

test('Phase A: ensureOAuthAccess is invoked before xai-api send', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'ovi-phase-a-oauth-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  let ensureCalls = 0;
  const runtime = {
    evidence: 'synthetic-or-custom',
    status: { pid: 42 },
    config: { apiKey: KEY },
    async start() {},
    async newSession() { return { sessionId: 's-oauth' }; },
    async loadSession() {},
    async prompt({ userText, prompt }) {
      assert.equal(userText, 'ping');
      assert.match(prompt, /ping/);
      return { stopReason: 'end_turn', metrics: {}, harness: emptyAgentState() };
    },
    async cancel() { return true; },
    async close() {},
  };
  const host = new AgentHost({ dataDir: directory, runtime });
  t.after(() => host.close());
  // Force runtimeId path: inject as if xai-api
  host.runtimeId = 'xai-api';
  const orig = host.ensureOAuthAccess.bind(host);
  host.ensureOAuthAccess = async (opts) => { ensureCalls += 1; return orig(opts); };
  const project = host.store.createProject('p');
  const conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: 'ping' });
  await host.runs.get(conversation.id)?.completion;
  assert.ok(ensureCalls >= 1, `expected ensureOAuthAccess before send, got ${ensureCalls}`);
});

test('Phase A: 401 triggers force ensureOAuthAccess once inside modelTurn', async () => {
  let calls = 0;
  let ensureForced = 0;
  const fetchMock = async () => {
    calls += 1;
    if (calls === 1) return { ok: false, status: 401, body: null };
    return {
      ok: true, status: 200,
      body: sse([{ choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }] }]),
    };
  };
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'http://127.0.0.1:9/v1', model: 'm', apiKey: KEY },
    fetch: fetchMock,
    answerMode: 'speed',
  });
  runtime.bindHost({
    providerStore: { bearer: () => 'xai-refreshed-token-00000000' },
    async ensureOAuthAccess({ force } = {}) {
      if (force) ensureForced += 1;
      return { accessToken: 'xai-refreshed-token-00000000' };
    },
    async mcp() { return { tools: [] }; },
  });
  const { sessionId } = await runtime.newSession();
  const result = await runtime.prompt({ sessionId, prompt: 'hi', userText: 'hi' });
  assert.equal(result.stopReason, 'end_turn');
  assert.equal(ensureForced, 1);
  assert.equal(calls, 2);
  assert.equal(runtime.config.apiKey, 'xai-refreshed-token-00000000');
});

test('Phase A: settings mark xai-api as preferred primary path', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'ovi-phase-a-settings-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const host = new AgentHost({ dataDir: directory, runtimeSelection: 'none' });
  t.after(() => host.close());
  const settings = host.settings();
  assert.equal(settings.preferredRuntime, 'xai-api');
  assert.match(settings.runtimeGuidance, /主路径|xAI Grok/);
});

test('Phase A: verifier fires once in quality loop when ending data answer without tools', async () => {
  const requests = [];
  const fetchMock = async (_url, init) => {
    requests.push(JSON.parse(init.body));
    if (requests.length === 1) {
      return { ok: true, status: 200, body: sse([{ choices: [{ delta: { content: '资产大概有一些。' }, finish_reason: 'stop' }] }]) };
    }
    if (requests.length === 2) {
      // After empty-block or verifier, call tool
      return {
        ok: true, status: 200,
        body: sse([
          { choices: [{ delta: { tool_calls: [{ index: 0, id: 'c1', function: { name: 'assets__inventory_stats', arguments: '{}' } }] }, finish_reason: 'tool_calls' }] },
        ]),
      };
    }
    return { ok: true, status: 200, body: sse([{ choices: [{ delta: { content: '根据 inventory_stats 共 2 个。' }, finish_reason: 'stop' }] }]) };
  };
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'http://127.0.0.1:9/v1', model: 'm', apiKey: KEY },
    fetch: fetchMock,
    answerMode: 'quality',
  });
  let called = 0;
  runtime.bindHost({
    async mcp(_t, _c, method) {
      if (method === 'tools/list') return { tools: [{ name: 'assets__inventory_stats', inputSchema: { type: 'object' } }] };
      called += 1;
      return { content: [{ type: 'text', text: '{"total":2}' }] };
    },
  });
  const { sessionId } = await runtime.newSession({
    mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'tok' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'c' }] }],
  });
  const result = await runtime.prompt({
    sessionId,
    prompt: 'Host context\n\n当前项目有多少资产？',
    userText: '当前项目有多少资产？',
  });
  assert.equal(result.stopReason, 'end_turn');
  assert.ok(called >= 1);
  assert.ok(result.metrics.forcedEmptyBlock || result.metrics.forcedToolHint || result.metrics.forcedVerifier);
  // Goal must stay short / user-only
  assert.ok(!/Host context/.test(result.harness.goal));
});
