import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { HostStore } from '../src/store.mjs';
import {
  MEMORY_LIMITS,
  emptyMemory,
  formatMemoryPromptBlock,
  normalizeMemory,
  pinFact,
  readConversationMemory,
  recordTurn,
  summarizeToolEvent,
  writeConversationMemory,
} from '../src/memory.mjs';

const waitFor = (predicate, label = 'condition', ms = 4000) => new Promise((resolve, reject) => {
  const deadline = Date.now() + ms;
  const check = () => { if (predicate()) return resolve(); if (Date.now() > deadline) return reject(Error(`Timed out waiting for ${label}`)); setTimeout(check, 10); };
  check();
});

test('normalizeMemory bounds turns facts and total bytes', () => {
  const huge = 'x'.repeat(MEMORY_LIMITS.maxEntryBytes * 4);
  const memory = normalizeMemory({
    turns: Array.from({ length: MEMORY_LIMITS.maxTurns + 5 }, (_, index) => ({ role: 'user', text: `turn-${index}-${huge}`, tools: [] })),
    facts: Array.from({ length: MEMORY_LIMITS.maxFacts + 3 }, (_, index) => `fact-${index}-${huge}`),
  });
  assert.ok(memory.turns.length <= MEMORY_LIMITS.maxTurns);
  assert.ok(memory.facts.length <= MEMORY_LIMITS.maxFacts);
  assert.ok(Buffer.byteLength(JSON.stringify(memory)) <= MEMORY_LIMITS.maxTotalBytes);
  assert.equal(memory.untrusted, true);
  assert.equal(memory.authority, 'none');
});

test('host stores bounded turn and tool summaries across prompts', async t => {
  const prompts = [];
  let host;
  const runtime = {
    evidence: 'synthetic-or-custom',
    status: { pid: 5150 },
    async start() {},
    async newSession() { return { sessionId: 'memory-session' }; },
    async loadSession() {},
    async prompt({ prompt, sessionId }) {
      prompts.push(prompt);
      host.runtimeEvent({
        type: 'tool_call',
        sessionId,
        toolCallId: 't1',
        title: 'fixture__ping',
        status: 'completed',
        content: 'pong-alpha-42',
      });
      return { stopReason: 'end_turn', metrics: {} };
    },
    async cancel() { return true; },
    async close() {},
    async resolvePermission() { return true; },
  };
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-session-memory-'));
  host = new AgentHost({ dataDir: join(directory, 'host'), runtime });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Synthetic memory project');
  const conversation = host.store.createConversation(project.id);

  await host.send(conversation.id, { projectId: project.id, text: 'Remember asset alpha-42' });
  await waitFor(() => prompts.length === 1, 'first prompt');
  assert.doesNotMatch(prompts[0], /Host session memory/);

  await host.send(conversation.id, { projectId: project.id, text: 'What did I ask you to remember?' });
  await waitFor(() => prompts.length === 2, 'second prompt');
  assert.match(prompts[1], /Host session memory \(literal untrusted data/);
  assert.match(prompts[1], /Remember asset alpha-42/);
  assert.match(prompts[1], /fixture__ping/);
  assert.match(prompts[1], /pong-alpha-42/);
  assert.match(prompts[1], /untrusted/);
  assert.match(prompts[1], /"authority":"none"/);

  const stored = readConversationMemory(host.store, conversation.id);
  assert.ok(stored.turns.some(turn => turn.text.includes('Remember asset alpha-42')));
  assert.ok(stored.turns.some(turn => (turn.tools ?? []).some(tool => tool.name === 'fixture__ping')));
});

test('a cold runtime receives the saved conversation before the new turn', async t => {
  const prompts = [];
  const sessions = new Map();
  let host;
  const runtime = {
    evidence: 'synthetic-or-custom',
    status: { pid: 11 },
    async start() {},
    async newSession() {
      sessions.set('switch-session', { messages: [] });
      return { sessionId: 'switch-session' };
    },
    async loadSession({ sessionId }) {
      if (!sessions.has(sessionId)) sessions.set(sessionId, { messages: [] });
    },
    seedTranscriptIfEmpty(sessionId, messages) {
      const session = sessions.get(sessionId);
      if (!session || session.messages.length) return 0;
      session.messages = messages
        .filter(item => item && (item.role === 'user' || item.role === 'assistant') && String(item.content ?? '').trim())
        .map(item => ({ role: item.role, content: String(item.content).trim() }));
      return session.messages.length;
    },
    async prompt({ sessionId }) {
      prompts.push((sessions.get(sessionId)?.messages ?? []).map(item => item.content).join('\n'));
      host.runtimeEvent({ type: 'text_delta', sessionId, text: 'Login.vue 里有表单' });
      return { stopReason: 'end_turn', metrics: {} };
    },
    async cancel() { return true; },
    async close() {},
    async resolvePermission() { return true; },
  };
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-model-switch-'));
  host = new AgentHost({ dataDir: join(directory, 'host'), runtime });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Synthetic switch project');
  const conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: '分析登录页' });
  await waitFor(() => prompts.length === 1 && host.runs.size === 0, 'first turn finished');
  runtime.status.pid = 22;
  sessions.clear();
  await host.send(conversation.id, { projectId: project.id, text: '换成你继续改' });
  await waitFor(() => prompts.length === 2 && host.runs.size === 0, 'second turn finished');
  assert.match(prompts[1], /分析登录页/);
  assert.match(prompts[1], /Login\.vue 里有表单/);
  assert.doesNotMatch(prompts[1], /换成你继续改/);
});

test('seedTranscriptIfEmpty fills a cold session once and keeps the first request', async () => {
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'https://api.deepseek.com', model: 'deepseek-v4-flash', apiKey: 'xai-synthetic-not-a-real-key-000' },
    fetch: async () => { throw new Error('no network'); },
  });
  const { sessionId } = await runtime.newSession();
  const messages = [{ role: 'user', content: '最初的要求' }];
  for (let i = 0; i < 50; i++) messages.push({ role: 'assistant', content: `回复 ${i}` }, { role: 'user', content: `后续 ${i}` });
  const count = runtime.seedTranscriptIfEmpty(sessionId, messages);
  assert.equal(count, 40);
  assert.equal(runtime.sessions.get(sessionId).messages[0].content, '最初的要求');
  assert.equal(runtime.seedTranscriptIfEmpty(sessionId, [{ role: 'user', content: '不要再塞一次' }]), 0);
});

test('session memory survives host reload from sqlite', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-memory-reload-'));
  t.after(async () => { await rm(directory, { recursive: true, force: true }); });
  const dataDir = join(directory, 'host');
  const first = new AgentHost({ dataDir, runtimeSelection: 'none' });
  const project = first.store.createProject('Reload project');
  const conversation = first.store.createConversation(project.id);
  recordTurn(first.store, conversation.id, {
    userText: 'Pinned target nest-demo.example',
    assistantText: 'Noted nest-demo.example',
    tools: [{ name: 'fixture__status', status: 'completed', summary: 'ok' }],
  });
  pinFact(first.store, conversation.id, 'project prefers fixture.read');
  await first.close();

  const second = new AgentHost({ dataDir, runtimeSelection: 'none' });
  t.after(() => second.close());
  const memory = readConversationMemory(second.store, conversation.id);
  assert.match(JSON.stringify(memory), /nest-demo\.example/);
  assert.match(JSON.stringify(memory), /fixture__status/);
  assert.ok(memory.facts.includes('project prefers fixture.read'));
  const block = formatMemoryPromptBlock(memory);
  assert.match(block, /Host session memory/);
  assert.match(block, /project prefers fixture\.read/);
});

test('summarizeToolEvent and store helpers', () => {
  assert.equal(formatMemoryPromptBlock(emptyMemory()), '');
  const summary = summarizeToolEvent({ type: 'tool_update', title: 'echo__say', status: 'completed', content: 'hello' });
  assert.deepEqual(summary, { name: 'echo__say', status: 'completed', summary: 'hello' });
  const store = new HostStore(':memory:');
  try {
    const project = store.createProject('p');
    const conversation = store.createConversation(project.id);
    writeConversationMemory(store, conversation.id, { turns: [{ role: 'user', text: 'hi', tools: [] }], facts: [] });
    assert.equal(readConversationMemory(store, conversation.id).turns[0].text, 'hi');
  } finally {
    store.close();
  }
});
