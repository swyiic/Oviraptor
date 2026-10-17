import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';

const KEY = 'xai-synthetic-not-a-real-key-000';

test('host without --runtime override restores saved xai-api (Grok login survives sidecar restart)', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ovi-wiring-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const first = new AgentHost({ dataDir: dir });
  first.providerStore.save({ provider: 'xai', apiKey: KEY });
  await first.setRuntime('xai-api');
  await first.close();
  const restarted = new AgentHost({ dataDir: dir }); // what the desktop sidecar now does
  t.after(() => restarted.close());
  assert.equal(restarted.settings().runtime.id, 'xai-api');
  const forcedNone = new AgentHost({ dataDir: dir, runtimeSelection: 'none' }); // old sidecar behaviour
  t.after(() => forcedNone.close());
  assert.equal(forcedNone.settings().runtime.id, 'none');
});

test('desktop sidecar no longer forces --runtime none', async () => {
  const rs = await readFile(fileURLToPath(new URL('../../../src-tauri/src/agent_host_sidecar.rs', import.meta.url)), 'utf8');
  assert.doesNotMatch(rs, /\.arg\("--runtime"\)\s*\.arg\("none"\)/);
});

test('desktop session page routes to agent-host runtime and labels it from host settings', async () => {
  const client = await readFile(fileURLToPath(new URL('../../../src/features/agentHost/chatClient.ts', import.meta.url)), 'utf8');
  assert.match(client, /runtime\?\.id === "xai-api"/);
  assert.match(client, /xAI Grok \(OAuth\)/);
  const workspace = await readFile(fileURLToPath(new URL('../../../src/features/conversations/ConversationWorkspace.vue', import.meta.url)), 'utf8');
  assert.match(workspace, /<AgentHostChat v-else-if="useAgentHostChat/);
  assert.match(workspace, /oviraptor:agent-host-settings/);
});
