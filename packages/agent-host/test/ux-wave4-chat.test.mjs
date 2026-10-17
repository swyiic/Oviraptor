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
