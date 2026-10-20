import { AgentHost } from '../../src/service.mjs';
import { serveHost } from '../../src/server.mjs';
const root = process.argv[2];
const response = delta => ({ ok: true, status: 200, body: (async function* () { yield `data: ${JSON.stringify({ choices: [{ delta, finish_reason: 'tool_calls' }] })}\n\n`; })() });
const apps = { store: { application: () => ({ manifest: {} }) }, capabilities: () => [{ name: 'fixture::read', appId: 'fixture', permission: 'fixture.read', grantRevision: 1 }], async call() { throw Error('fixture must remain at model boundary'); }, async close() {} };
const host = new AgentHost({ dataDir: root, apps, providerFetch: async (_url, init) => {
  const request = JSON.parse(init.body), text = request.messages.find(item => item.role === 'user').content;
  if (text.includes('Read-only research.')) { process.stdout.write('READY\n'); await new Promise(() => {}); }
  if (host.subagents.runs.size) await new Promise(() => {});
  return response({ tool_calls: [{ index: 0, id: 'child', function: { name: 'host_task_delegate', arguments: JSON.stringify({ goal: 'Child read-only research', kind: 'research', tools: ['fixture__read'] }) } }] });
} });
host.ensureSkills = async () => ({ capture: () => ({}), validate() {}, release() {} });
await serveHost(host);
await host.setProvider({ provider: 'openai-compatible', baseUrl: 'https://fixture.invalid/v1', model: 'fixture', apiKey: 'synthetic-key' }); await host.setRuntime('xai-api');
const p = host.store.createProject('Synthetic restart'), c = host.store.createConversation(p.id);
await host.send(c.id, { text: 'Original parent research goal' });
