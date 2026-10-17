import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, stat, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { validateProviderConfig, ProviderStore, truncateToolResult, truncateMcpResult, planToolWaves, runToolCalls, readChatStream, strategyHint, OpenAiCompatibleRuntime, PLANNING_GUIDANCE } from '../src/provider.mjs';

const KEY = 'xai-synthetic-not-a-real-key-000';
const tmp = async t => { const d = await mkdtemp(join(tmpdir(), 'ovi-provider-')); t.after(() => rm(d, { recursive: true, force: true })); return d; };
const sse = chunks => (async function* () { for (const c of chunks) yield Buffer.from(`data: ${typeof c === 'string' ? c : JSON.stringify(c)}\n\n`); })();

test('provider config validation: xAI defaults, https only, no credentials in URL, key bounds', () => {
  const v = validateProviderConfig({ provider: 'xai', apiKey: KEY });
  assert.equal(v.baseUrl, 'https://api.x.ai/v1'); assert.equal(v.model, 'grok-4.7');
  assert.throws(() => validateProviderConfig({ baseUrl: 'http://api.x.ai/v1' }), { code: 'host_provider_base_url_invalid' });
  assert.equal(validateProviderConfig({ provider: 'openai-compatible', baseUrl: 'http://127.0.0.1:8080/v1', model: 'm' }).baseUrl, 'http://127.0.0.1:8080/v1');
  assert.throws(() => validateProviderConfig({ baseUrl: 'https://u:p@api.x.ai/v1' }), { code: 'host_provider_base_url_invalid' });
  assert.throws(() => validateProviderConfig({ apiKey: 'short' }), { code: 'host_provider_api_key_invalid' });
  assert.throws(() => validateProviderConfig({ provider: 'oauth-fake' }), { code: 'host_provider_invalid' });
  assert.throws(() => validateProviderConfig({ model: 'bad model;' }), { code: 'host_provider_model_invalid' });
});

test('API key is stored 0600 and never returned by settings or HTTP', async t => {
  const dir = await tmp(t); const store = new ProviderStore(dir);
  assert.throws(() => store.save({ provider: 'xai' }), { code: 'host_provider_api_key_required' });
  const view = store.save({ provider: 'xai', apiKey: KEY });
  assert.equal(view.apiKeyConfigured, true); assert.equal(view.oauthAvailable, true); assert.ok(!JSON.stringify(view).includes(KEY));
  assert.equal((await stat(store.file)).mode & 0o777, 0o600);
  store.save({ provider: 'xai', model: 'grok-4.6' }); // keeps key
  assert.equal(store.read().apiKey, KEY);
  const host = new AgentHost({ dataDir: join(dir, 'h') }); const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); });
  const res = await fetch(`${web.origin}/api/settings/provider`, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify({ provider: 'xai', apiKey: KEY }) });
  const text = await res.text(); assert.equal(res.status, 200); assert.ok(!text.includes(KEY));
  const got = await (await fetch(`${web.origin}/api/settings`, { headers: { 'X-Oviraptor-Session': web.token } })).text();
  assert.ok(!got.includes(KEY)); assert.match(got, /"apiKeyConfigured":true/);
  const sel = await fetch(`${web.origin}/api/settings`, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify({ runtime: 'xai-api' }) });
  assert.equal(sel.status, 200); assert.equal(host.settings().runtime.id, 'xai-api'); assert.match(host.runtimeIdentity(), /^xai-api:/);
});

test('truncation keeps UTF-8 valid and tells the agent to re-query', () => {
  const r = truncateToolResult('中'.repeat(10000), 1000);
  assert.equal(r.truncated, true); assert.match(r.text, /truncated: showed \d+ of 30000 bytes/); assert.match(r.text, /narrower filters/);
  assert.ok(!r.text.includes('\uFFFD'));
  assert.equal(truncateToolResult('small').truncated, false);
  const m = truncateMcpResult({ content: [{ type: 'text', text: 'x'.repeat(50000) }], isError: false }, 2000);
  assert.equal(m._meta.truncated, true); assert.ok(Buffer.byteLength(m.content[0].text) < 2300); assert.equal(m.isError, false);
});

test('independent tool calls run in parallel; dependent calls wait', async () => {
  assert.deepEqual(planToolWaves([{ id: 'a' }, { id: 'b' }, { id: 'c', dependsOn: ['a'] }]).map(w => w.map(c => c.id)), [['a', 'b'], ['c']]);
  let active = 0, peak = 0; const order = [];
  const exec = async call => { active++; peak = Math.max(peak, active); await new Promise(r => setTimeout(r, 30)); active--; order.push(call.id); if (call.name === 'bad') throw Object.assign(Error('x'), { code: 'host_tool_permission_denied' }); return { ok: call.id }; };
  const started = Date.now();
  const results = await runToolCalls([{ id: 'a', name: 't' }, { id: 'b', name: 't' }, { id: 'c', name: 'bad' }, { id: 'd', name: 't', arguments: { from: 'a' } }], exec);
  assert.equal(peak, 3); assert.equal(order.at(-1), 'd'); assert.ok(Date.now() - started < 200);
  assert.equal(results[2].ok, false); assert.match(results[2].content, /do not retry/);
});

test('strategy hints differ by structured error code', () => {
  assert.match(strategyHint('host_tool_authority_changed'), /tools\/list/);
  assert.match(strategyHint('host_app_timeout'), /narrow/);
  assert.match(strategyHint('weird'), /do not repeat/);
});

test('SSE parser assembles streamed text and split tool-call arguments', async () => {
  const deltas = [];
  const out = await readChatStream(sse([
    { choices: [{ delta: { content: 'Hel' } }] }, { choices: [{ delta: { content: 'lo' } }] },
    { choices: [{ delta: { tool_calls: [{ index: 0, id: 'c1', function: { name: 'x__y', arguments: '{"a"' } }] } }] },
    { choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: ':1}' } }] }, finish_reason: 'tool_calls' }] }, '[DONE]']), d => deltas.push(d));
  assert.deepEqual(deltas, ['Hel', 'lo']); assert.equal(out.toolCalls[0].arguments, '{"a":1}'); assert.equal(out.finishReason, 'tool_calls');
});

test('mock provider agent loop: streams, runs parallel tools through host MCP, sends plan guidance, never leaks key into events', async () => {
  const requests = []; const events = []; const calls = [];
  const fetchMock = async (url, init) => {
    requests.push({ url, body: JSON.parse(init.body), auth: init.headers.Authorization });
    if (requests.length === 1) return { ok: true, status: 200, body: sse([{ choices: [{ delta: { content: 'Plan: 1. read' } }] },
      { choices: [{ delta: { tool_calls: [{ index: 0, id: 't1', function: { name: 'app__a', arguments: '{}' } }, { index: 1, id: 't2', function: { name: 'app__b', arguments: '{}' } }] }, finish_reason: 'tool_calls' }] }]) };
    return { ok: true, status: 200, body: sse([{ choices: [{ delta: { content: ' done' }, finish_reason: 'stop' }] }]) };
  };
  const runtime = new OpenAiCompatibleRuntime({ config: { baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7', apiKey: KEY }, fetch: fetchMock, onEvent: e => events.push(e) });
  runtime.bindHost({ async mcp(token, conv, method, params) {
    if (method === 'tools/list') return { tools: [{ name: 'app__a', inputSchema: { type: 'object' } }, { name: 'app__b' }] };
    calls.push(params.name); if (params.name === 'app__b') throw Object.assign(Error('x'), { code: 'host_app_timeout' });
    return { content: [{ type: 'text', text: 'y'.repeat(40000) }] };
  } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'tok' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'c' }] }] });
  const result = await runtime.prompt({ sessionId, prompt: 'hi' });
  assert.equal(result.stopReason, 'end_turn'); assert.ok(result.metrics.firstTextMs !== null);
  assert.equal(requests[0].url, 'https://api.x.ai/v1/chat/completions'); assert.equal(requests[0].auth, `Bearer ${KEY}`);
  assert.equal(requests[0].body.stream, true); assert.equal(requests[0].body.parallel_tool_calls, true);
  assert.ok(requests[0].body.messages.some(m => m.content === PLANNING_GUIDANCE));
  assert.deepEqual(calls.sort(), ['app__a', 'app__b']);
  const toolMsgs = requests[1].body.messages.filter(m => m.role === 'tool');
  assert.match(toolMsgs.find(m => m.tool_call_id === 't1').content, /truncated/);
  assert.match(toolMsgs.find(m => m.tool_call_id === 't2').content, /narrow/);
  assert.deepEqual(events.filter(e => e.type === 'text_delta').map(e => e.text), ['Plan: 1. read', ' done']);
  assert.ok(events.some(e => e.type === 'tool_update' && e.status === 'failed'));
  assert.ok(!JSON.stringify(events).includes(KEY));
});

test('host prompt includes plan→step guidance and provider 401 maps to safe code', async t => {
  const dir = await tmp(t); const prompts = [];
  const runtime = { evidence: 'synthetic-or-custom', status: { pid: 9 }, async start() {}, async newSession() { return { sessionId: 's' }; }, async loadSession() {}, async prompt({ prompt }) { prompts.push(prompt); return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {} };
  const host = new AgentHost({ dataDir: dir, runtime }); t.after(() => host.close());
  const p = host.store.createProject('p'); const c = host.store.createConversation(p.id);
  await host.send(c.id, { projectId: p.id, text: 'go' }); await host.runs.get(c.id)?.completion;
  assert.ok(prompts[0].includes(PLANNING_GUIDANCE));
  const rt = new OpenAiCompatibleRuntime({ config: { baseUrl: 'https://api.x.ai/v1', model: 'm', apiKey: KEY }, fetch: async () => ({ ok: false, status: 401, body: null }) });
  const { sessionId } = await rt.newSession();
  await assert.rejects(rt.prompt({ sessionId, prompt: 'x' }), e => e.code === 'host_provider_unauthorized' && !JSON.stringify(e).includes(KEY));
});
