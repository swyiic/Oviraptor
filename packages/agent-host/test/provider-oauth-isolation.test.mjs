import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, mkdir, writeFile, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { ProviderStore, PROVIDERS, DEEPSEEK_BASE_URL, validateProviderConfig, OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

const tokens = { accessToken: 'synthetic-grok-access', refreshToken: 'synthetic-grok-refresh', expiresAt: Date.now() + 3_600_000 };
const fixture = async t => { const directory = await mkdtemp('/private/tmp/ovi-provider-auth-'); t.after(() => rm(directory, { recursive: true, force: true })); return new ProviderStore(directory); };

test('explicit saved OAuth selection works at the same official endpoint and preserves its API key', async t => {
  const store = await fixture(t);
  assert.throws(() => store.selectOAuth(), { code: 'host_oauth_not_configured' });
  store.saveOAuth(tokens, { model: 'grok-4.6' });
  store.save({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: 'grok-4.7', apiKey: 'synthetic-official-api-key' });
  store.save({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: 'grok-4.6' });
  assert.equal(store.read().authentication, 'api-key', 'normal model edits keep the chosen authentication');
  const selected = store.selectOAuth();
  assert.equal(selected.authentication, 'oauth'); assert.equal(store.bearer(), tokens.accessToken);
  store.clearOAuth(); assert.equal(store.bearer(), 'synthetic-official-api-key');
});

test('saved OAuth HTTP selection requires the host session, refuses active work, and refreshes the actual runtime config', async t => {
  const store = await fixture(t); let requests = 0;
  const host = new AgentHost({ dataDir: join(store.dir, '..'), networkEnvironment: {}, providerFetch: async () => { requests++; throw Error('No external request allowed'); } });
  const web = await serveHost(host); t.after(async () => { await web.close(); await host.close(); });
  host.providerStore.saveOAuth(tokens);
  await host.setProvider({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: 'grok-4.7', apiKey: 'synthetic-official-api-key' });
  await host.setRuntime('xai-api');
  const post = authenticated => fetch(`${web.origin}/api/settings/provider/oauth/select`, { method: 'POST', headers: authenticated ? { 'X-Oviraptor-Session': web.token } : {} });
  assert.equal((await post(false)).status, 403);
  host.runs.set('fixture', {});
  assert.equal((await post(true)).status, 409); assert.equal(host.providerStore.read().authentication, 'api-key');
  host.runs.delete('fixture'); host.runtimeChanging = true;
  assert.equal((await post(true)).status, 409); host.runtimeChanging = false;
  host.oauthPending = { status: 'pending' };
  assert.equal((await post(true)).status, 409); host.oauthPending = null;
  const response = await post(true), text = await response.text();
  assert.equal(response.status, 200); assert.equal(JSON.parse(text).apiProvider.authentication, 'oauth');
  assert.equal(host.runtime.config.authentication, 'oauth'); assert.equal(host.runtime.config.apiKey, tokens.accessToken);
  for (const secret of [tokens.accessToken, tokens.refreshToken, 'synthetic-official-api-key']) assert(!text.includes(secret));
  assert.equal(requests, 0);
});

test('OAuth login after DeepSeek selects the xAI endpoint and preserves both provider credentials across switches', async t => {
  const store = await fixture(t);
  store.save({ model: 'deepseek-chat', apiKey: 'synthetic-deepseek-key' });
  store.saveOAuth(tokens);
  assert.equal(store.read().provider, 'xai'); assert.equal(store.read().baseUrl, PROVIDERS.xai.baseUrl); assert.equal(store.read().model, PROVIDERS.xai.model);
  assert.equal(store.bearer(), tokens.accessToken);
  store.save({ model: 'deepseek-chat' }); assert.equal(store.bearer(), 'synthetic-deepseek-key'); assert.equal(store.read().accessToken, undefined);
  store.save({ model: 'grok-4.7' }); assert.equal(store.bearer(), tokens.accessToken); assert.equal(store.read().baseUrl, PROVIDERS.xai.baseUrl);
  store.save({ model: 'grok-4.6' }); store.save({ model: 'deepseek-chat' }); assert.equal(store.bearer(), 'synthetic-deepseek-key');
  const publicText = JSON.stringify(store.publicView());
  for (const secret of ['synthetic-deepseek-key', tokens.accessToken, tokens.refreshToken]) assert(!publicText.includes(secret));
  assert.equal(store.read().keys, undefined); assert.equal(store.read().oauth, undefined); assert.equal((await stat(store.file)).mode & 0o777, 0o600);
});

test('OAuth cannot be reassigned to an explicit custom endpoint and an API save never carries OAuth tokens', async t => {
  const store = await fixture(t); store.saveOAuth(tokens);
  assert.throws(() => store.save({ provider: 'xai', baseUrl: 'https://custom.invalid/v1', model: 'grok-4.7' }), { code: 'host_provider_api_key_required' });
  assert.equal(store.bearer(), tokens.accessToken);
  assert.throws(() => store.saveOAuth(tokens, { baseUrl: 'https://api.x.ai.evil.invalid/v1' }), { code: 'host_oauth_provider_invalid' });
  assert.throws(() => store.saveOAuth(tokens, { model: 'deepseek-chat' }), { code: 'host_oauth_provider_invalid' });
  store.save({ provider: 'openai-compatible', baseUrl: 'https://custom.invalid/v1', model: 'grok-4.7', apiKey: 'synthetic-custom-key' });
  assert.equal(store.read().baseUrl, 'https://custom.invalid/v1'); assert.equal(store.bearer(), 'synthetic-custom-key'); assert.equal(store.read().accessToken, undefined);
  store.save({ model: 'deepseek-chat' }); assert.equal(store.read().baseUrl, 'https://custom.invalid/v1'); assert.equal(store.bearer(), 'synthetic-custom-key');
});

test('explicit custom endpoints retain provider identity regardless of model branding', () => {
  for (const model of ['deepseek-chat', 'grok-4.7']) {
    const value = validateProviderConfig({ provider: 'openai-compatible', baseUrl: 'https://deepseek.gateway.invalid/v1', model });
    assert.equal(value.provider, 'openai-compatible'); assert.equal(value.baseUrl, 'https://deepseek.gateway.invalid/v1');
  }
});

test('OAuth refresh and model changes retain API key slots without exposing inactive OAuth through read()', async t => {
  const store = await fixture(t);
  store.save({ model: 'grok-4.7', apiKey: 'synthetic-xai-key' });
  store.save({ model: 'deepseek-chat', apiKey: 'synthetic-deepseek-key' });
  store.saveOAuth(tokens); store.saveOAuth({ accessToken: 'synthetic-refreshed-access', expiresAt: Date.now() + 7_200_000 });
  assert.equal(store.read().refreshToken, tokens.refreshToken);
  store.save({ model: 'deepseek-chat' }); assert.equal(store.bearer(), 'synthetic-deepseek-key'); assert.equal(store.read().refreshToken, undefined);
  store.save({ model: 'grok-4.7' }); assert.equal(store.bearer(), 'synthetic-refreshed-access');
  store.save({ model: 'grok-4.7', apiKey: 'synthetic-new-xai-key' }); store.save({ model: 'grok-4.6' });
  assert.equal(store.bearer(), 'synthetic-new-xai-key'); assert.equal(store.read().authentication, 'api-key');
});

test('legacy OAuth state polluted with a DeepSeek endpoint is repaired without sending OAuth there', async t => {
  const store = await fixture(t); await mkdir(store.dir, { recursive: true });
  await writeFile(store.file, JSON.stringify({ provider: 'xai', baseUrl: DEEPSEEK_BASE_URL, model: 'deepseek-chat', authentication: 'oauth', ...tokens }));
  assert.equal(store.read().baseUrl, PROVIDERS.xai.baseUrl); assert.equal(store.read().model, PROVIDERS.xai.model); assert.equal(store.bearer(), tokens.accessToken);
});

test('signing out removes only OAuth and preserves active or previously saved API providers', async t => {
  const store = await fixture(t);
  store.save({ model: 'grok-4.7', apiKey: 'synthetic-xai-key' });
  store.save({ model: 'deepseek-chat', apiKey: 'synthetic-deepseek-key' }); store.saveOAuth(tokens);
  store.clearOAuth(); assert.equal(store.bearer(), 'synthetic-xai-key'); assert.equal(store.publicView().oauthConfigured, false);
  store.save({ model: 'deepseek-chat' }); assert.equal(store.bearer(), 'synthetic-deepseek-key');
  store.saveOAuth(tokens); store.save({ model: 'deepseek-chat' }); store.clearOAuth();
  assert.equal(store.bearer(), 'synthetic-deepseek-key');
  store.save({ model: 'grok-4.7' }); assert.equal(store.bearer(), 'synthetic-xai-key');
});

test('a custom endpoint 401 never refreshes or receives the host Grok OAuth credential', async () => {
  const requests = []; let refreshed = 0;
  const runtime = new OpenAiCompatibleRuntime({ config: { provider: 'openai-compatible', baseUrl: 'https://custom.invalid/v1', model: 'fixture', apiKey: 'synthetic-custom-key' }, fetch: async (url, init) => { requests.push({ url, authorization: init.headers.Authorization }); return { ok: false, status: 401 }; } });
  runtime.bindHost({ async ensureOAuthAccess() { refreshed++; return { provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, authentication: 'oauth', ...tokens }; } });
  const active = await runtime.newSession();
  await assert.rejects(runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' }), { code: 'host_provider_unauthorized' });
  assert.equal(refreshed, 0); assert.deepEqual(requests, [{ url: 'https://custom.invalid/v1/chat/completions', authorization: 'Bearer synthetic-custom-key' }]);
});

test('only an OAuth session at the official xAI endpoint retries 401 with matching refreshed tokens', async () => {
  const requests = []; let refreshed = 0;
  const config = { provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: 'grok-4.7', authentication: 'oauth', apiKey: tokens.accessToken };
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async (url, init) => {
    requests.push({ url, authorization: init.headers.Authorization });
    if (requests.length === 1) return { ok: false, status: 401 };
    return { ok: true, status: 200, body: (async function* () { yield 'data: {"type":"response.completed","response":{"id":"fixture","output":[]}}\n\n'; })() };
  } });
  runtime.bindHost({ async ensureOAuthAccess() { refreshed++; return { ...config, accessToken: 'synthetic-refreshed-access' }; } });
  const active = await runtime.newSession(); const result = await runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  assert.equal(result.stopReason, 'end_turn'); assert.equal(refreshed, 1); assert.equal(requests.length, 2);
  assert(requests.every(request => request.url.startsWith(`${PROVIDERS.xai.baseUrl}/`)));
  assert.equal(requests[1].authorization, 'Bearer synthetic-refreshed-access');
  assert.throws(() => new OpenAiCompatibleRuntime({ config: { ...config, baseUrl: 'https://custom.invalid/v1' } }), { code: 'host_oauth_provider_invalid' });
});
