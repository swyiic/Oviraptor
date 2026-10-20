import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = new URL('../../../src/features/agentHost/', import.meta.url);
const read = name => readFile(new URL(name, root), 'utf8');

test('Wave4-B: friendly Chinese errors, no duplicate runtime.error banner', async () => {
  const errors = await read('errors.ts');
  assert.match(errors, /host_provider_network_unreachable: \{ title: \["[^"]*网络/);
  assert.match(errors, /host_provider_request_failed/);
  assert.match(errors, /host_provider_api_key_required/);
  assert.match(errors, /出错了/);
  const chat = await read('components/AgentHostChat.vue');
  assert.match(chat, /describeAgentHostError/);
  assert.doesNotMatch(chat, /error\.value = String\(event\.code/);
  assert.doesNotMatch(chat, /\{\{ message\.failureCode \|\|/);
});

test('Wave4-C: compact styled toolbar with model picker on the right, styled composer', async () => {
  const chat = await read('components/AgentHostChat.vue');
  assert.match(chat, /class="ahc-toolbar-right"[\s\S]*data-testid="agent-host-model-picker"/);
  assert.doesNotMatch(chat, /旧版对话/);
  assert.match(chat, /deleteConversation/);
  assert.match(chat, /data-testid="agent-host-session-switcher"/);
  const style = chat.slice(chat.indexOf('<style scoped>'));
  assert.match(style, /\.ahc-toolbar \{/);
  assert.match(style, /\.ahc-composer \{/);
  assert.match(chat, /client\.setModel\(/);
});

test('Wave4-D: copy, edit & resend, steer into the running turn', async () => {
  const chat = await read('components/AgentHostChat.vue');
  for (const id of ['agent-host-copy', 'agent-host-edit', 'agent-host-steer']) assert.match(chat, new RegExp(`data-testid="${id}"`));
  assert.match(chat, /navigator\.clipboard\.writeText/);
  assert.match(chat, /async function steer\(text: string\)/);
  assert.match(chat, /client\.steer\(/);
  assert.match(chat, /if \(running\.value\)/);
  assert.doesNotMatch(chat, /async function steer\(\) \{\s*await cancel\(\);/);
  assert.doesNotMatch(chat, /<textarea[^>]*:disabled="[^"]*running/);
  assert.match(chat, /<textarea[^>]*:disabled="loadingProject"/);
});

test('Wave4-E: title updates and heartbeat are rendered', async () => {
  const chat = await read('components/AgentHostChat.vue');
  assert.match(chat, /event\.type === "conversation\.updated"/);
  assert.match(chat, /event\.type === "runtime\.heartbeat"/);
  assert.match(chat, /data-testid="agent-host-heartbeat"/);
  assert.match(chat, /仍在运行/);
  assert.match(chat, /data-testid="agent-host-inner"/);
  assert.match(chat, /runtime\.inner/);
  assert.match(chat, /待验证/);
});
