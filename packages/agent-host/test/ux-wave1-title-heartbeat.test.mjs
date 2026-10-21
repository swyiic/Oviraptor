import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { HostStore, DEFAULT_CONVERSATION_TITLE, deriveConversationTitle } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('deriveConversationTitle truncates on grapheme boundary without a model call', () => {
  assert.equal(deriveConversationTitle('  hello world  '), 'hello world');
  assert.equal(deriveConversationTitle('a'.repeat(60)).endsWith('…'), true);
  assert.equal([...deriveConversationTitle('资产清单里有多少主机需要关注一下')].length <= 48, true);
  assert.equal(deriveConversationTitle(''), DEFAULT_CONVERSATION_TITLE);
});

test('first user message auto-titles default conversations and skips custom titles', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-ux-title-'));
  const store = new HostStore(':memory:');
  const events = [];
  const host = new AgentHost({ dataDir: directory, store, onEvent: event => events.push(event) });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = store.createProject('Synthetic title project');
  const fresh = store.createConversation(project.id);
  assert.equal(fresh.title, DEFAULT_CONVERSATION_TITLE);
  await host.send(fresh.id, { text: '统计当前项目资产数量' });
  assert.equal(store.conversation(fresh.id).title, '统计当前项目资产数量');
  assert.equal(events.some(event => event.type === 'conversation.updated' && event.title === '统计当前项目资产数量'), true);

  const localized = store.createConversation(project.id, '新对话');
  await host.send(localized.id, { text: '我们在哪个项目里' });
  assert.equal(store.conversation(localized.id).title, '我们在哪个项目里');

  const custom = store.createConversation(project.id, '手工标题');
  await host.send(custom.id, { text: '不应覆盖手工标题' });
  assert.equal(store.conversation(custom.id).title, '手工标题');
});

test('runtime.heartbeat emits while a tool/ACP turn is active and stops on completion', { timeout: 5000 }, async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-ux-heartbeat-'));
  const store = new HostStore(':memory:');
  const prompting = deferred();
  const response = deferred();
  const runtime = {
    status: { pid: 91 },
    async start() {},
    async newSession() { return { sessionId: 'synthetic-heartbeat-session' }; },
    prompt() { prompting.resolve(); return response.promise; },
    async cancel() { return true; },
    async close() {},
  };
  const events = [];
  const host = new AgentHost({
    dataDir: directory,
    store,
    runtime,
    heartbeatMs: 40,
    onEvent: event => events.push(event),
  });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = store.createProject('Synthetic heartbeat');
  const conversation = store.createConversation(project.id);
  await host.send(conversation.id, { text: 'long tool turn' });
  const run = host.runs.get(conversation.id);
  assert(run);
  await prompting.promise;
  await new Promise(resolve => setTimeout(resolve, 120));
  const beats = events.filter(event => event.type === 'runtime.heartbeat' && event.runId === run.id);
  assert.ok(beats.length >= 1, `expected heartbeat events, got ${beats.length}`);
  assert.equal(typeof beats[0].elapsedMs, 'number');
  assert.equal(typeof beats[0].lastEventAt, 'string');
  response.resolve({ stopReason: 'end_turn', metrics: { totalMs: 1 } });
  await run.completion;
  const after = events.filter(event => event.type === 'runtime.heartbeat').length;
  await new Promise(resolve => setTimeout(resolve, 80));
  assert.equal(events.filter(event => event.type === 'runtime.heartbeat').length, after);
});
