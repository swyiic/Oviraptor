import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ProviderStore } from '../src/provider.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

const reference = { kind: 'models', policyId: 'model-directory', revision: 2, modelProfileId: 'grok' };
const payload = { model: 'grok-4.7', messages: [{ role: 'user', content: 'fixture' }], stream: false };
const completion = { choices: [{ message: { role: 'assistant', content: 'ok' } }], usage: { prompt_tokens: 2, completion_tokens: 1 } };
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

async function setup(t, fetchImpl) {
  const dir = mkdtempSync(join(tmpdir(), 'ovi-oauth-bridge-'));
  const store = new ProviderStore(dir);
  const host = { providerStore: store, providerFetch: fetchImpl, oauthFetch: fetchImpl, runtimeId: 'none', apps: {}, setOrigin() {},
    ensureSavedOAuthAccess: AgentHost.prototype.ensureSavedOAuthAccess };
  const server = await serveHost(host);
  t.after(async () => { await server.close(); rmSync(dir, { recursive: true, force: true }); });
  return { store, host, server, call: (input = payload, headers = {}, path = '/api/model/oauth/chat/completions') => fetch(`${server.origin}${path}`, {
    method: input === null ? 'GET' : 'POST', headers: { Authorization: `Bearer ${server.token}`, 'Content-Type': 'application/json', ...headers }, ...(input === null ? {} : { body: JSON.stringify(input) }),
  }) };
}
const login = store => store.saveOAuth({ accessToken: 'oauth-fixture-access', refreshToken: 'oauth-fixture-refresh', expiresAt: Date.now() + 3_600_000 });
const useApi = store => store.save({ provider: 'openai-compatible', baseUrl: 'https://model.example/v1', model: 'fixture-model', apiKey: 'fixture-api-secret', authType: 'api-key' });

test('OAuth model binding survives selection and refresh without exporting secrets', async t => {
  const { store } = await setup(t, async () => json(completion)); login(store);
  store.selectOAuth({ model: 'grok-4.7-fast', connectionRef: reference });
  assert.deepEqual(store.publicView().connectionRef, reference);
  assert.equal(store.publicView().model, 'grok-4.7-fast');
  store.refreshOAuth({ accessToken: 'rotated-fixture-access' }, 'oauth-fixture-access');
  assert.deepEqual(store.publicView().connectionRef, reference);
  assert.ok(!JSON.stringify(store.publicView()).includes('rotated-fixture-access'));
  useApi(store);
  store.refreshOAuth({ accessToken: 'rotated-again-fixture' }, 'rotated-fixture-access');
  assert.equal(store.read().model, 'fixture-model');
  assert.equal(store.read().apiKey, 'fixture-api-secret');
  assert.equal(store.readOAuth().accessToken, 'rotated-again-fixture');
});

test('an in-flight refresh cannot recreate a removed login', async t => {
  const { store } = await setup(t, async () => json(completion)); login(store); useApi(store); store.clearOAuth();
  assert.throws(() => store.refreshOAuth({ accessToken: 'late' }, 'oauth-fixture-access'), /host_oauth_changed/);
  assert.equal(store.readOAuth(), null);
  assert.equal(store.read().apiKey, 'fixture-api-secret');
});

test('bound OAuth model IDs use the same exact prefix normalization as the Rust gateway', async t => {
  const { store } = await setup(t, async () => json(completion)); login(store);
  for (const [model, expected] of [['openai/grok-custom', 'grok-custom'], ['xai/grok-custom', 'xai/grok-custom'], ['openai/openai/grok-custom', 'openai/grok-custom']]) {
    store.selectOAuth({ model, connectionRef: reference });
    assert.equal(store.read().model, expected); assert.equal(store.publicView().model, expected);
    store.saveOAuth({ accessToken: 'relogin-fixture', refreshToken: 'relogin-refresh' }, { select: false });
    assert.equal(store.read().model, expected);
    assert.equal(store.read().baseUrl, 'https://api.x.ai/v1');
  }
});

test('saving a login without activation leaves the existing chat provider and binding intact', async t => {
  const { store } = await setup(t, async () => json(completion)); useApi(store);
  const before = store.read();
  store.saveOAuth({ accessToken: 'new-login', refreshToken: 'new-refresh' }, { select: false });
  assert.deepEqual(store.read(), before);
  assert.equal(store.publicView().oauthSaved, true);
  assert.equal(store.publicView().oauthConfigured, false);
  store.selectOAuth({ model: 'grok-4.7', connectionRef: reference });
  assert.equal(store.publicView().oauthConfigured, true);
  assert.deepEqual(store.publicView().connectionRef, reference);
});

test('device login with activate=false does not select a runtime or replace the existing provider', async t => {
  const dir = mkdtempSync(join(tmpdir(), 'ovi-login-selection-'));
  let requests = 0;
  const host = new AgentHost({ dataDir: dir, runtimeSelection: 'none', providerFetch: async () => ++requests === 1
    ? json({ device_code: 'fixture-device', user_code: 'FIXTURE', verification_uri: 'https://accounts.x.ai/fixture', expires_in: 300 })
    : json({ access_token: 'fixture-new-login', refresh_token: 'fixture-new-refresh', expires_in: 3600 }) });
  t.after(async () => { await host.close(); rmSync(dir, { recursive: true, force: true }); });
  useApi(host.providerStore); const before = host.providerStore.read();
  await host.startProviderOAuth({ background: false, activate: false });
  assert.equal((await host.pollProviderOAuth()).status, 'authorized');
  assert.deepEqual(host.providerStore.read(), before);
  assert.equal(host.runtimeId, 'none');
  assert.equal(host.providerStore.readOAuth().accessToken, 'fixture-new-login');
});

test('native JSON requests use only saved OAuth and preserve body/model/usage while chat stays API', async t => {
  const calls = [];
  const { store, call, server } = await setup(t, async (url, init) => { calls.push({ url, init }); return json(completion); });
  login(store); useApi(store);
  const input = { ...payload, response_format: { type: 'json_object' }, tools: [{ type: 'function', function: { name: 'fixture', parameters: { type: 'object' } } }] };
  const response = await call(input);
  assert.equal(response.status, 200); assert.deepEqual(await response.json(), completion);
  assert.equal(calls[0].url, 'https://api.x.ai/v1/chat/completions');
  assert.equal(calls[0].init.headers.Authorization, 'Bearer oauth-fixture-access');
  assert.deepEqual(JSON.parse(calls[0].init.body), input);
  assert.equal(calls[0].init.redirect, 'error');
  assert.ok(!JSON.stringify(calls).includes(server.token));
  assert.ok(!JSON.stringify(calls).includes('fixture-api-secret'));
  assert.equal(store.read().model, 'fixture-model');
});

test('bridge requires a session and preserves origin checks; bearer is not accepted on other routes', async t => {
  let calls = 0;
  const { store, call, server } = await setup(t, async () => { calls++; return json(completion); }); login(store);
  assert.equal((await call(payload, { Authorization: 'Bearer wrong' })).status, 403);
  assert.equal((await call(payload, { Origin: 'https://foreign.example' })).status, 403);
  const other = await fetch(`${server.origin}/api/settings`, { headers: { Authorization: `Bearer ${server.token}` } });
  assert.equal(other.status, 403); assert.equal(calls, 0);
});

test('preflight models route authenticates against the actual upstream provider', async t => {
  const calls = [];
  const { store, call } = await setup(t, async (url, init) => { calls.push({ url, init }); return json({ data: [{ id: 'grok-4.7' }] }); }); login(store);
  const response = await call(null, {}, '/api/model/oauth/models');
  assert.equal(response.status, 200); assert.deepEqual(await response.json(), { data: [{ id: 'grok-4.7' }] });
  assert.equal(calls[0].url, 'https://api.x.ai/v1/models'); assert.equal(calls[0].init.method, 'GET');
});

test('SSE completion and tool chunks pass through without buffering or rewriting', async t => {
  const wire = 'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1"}]}}]}\n\ndata: [DONE]\n\n';
  const { store, call } = await setup(t, async () => new Response(wire, { headers: { 'Content-Type': 'text/event-stream' } })); login(store);
  const response = await call({ ...payload, stream: true });
  assert.equal(response.headers.get('content-type'), 'text/event-stream'); assert.equal(await response.text(), wire);
});

test('401 refresh retries once with the renewed login and leaves the API selection untouched', async t => {
  const auth = []; let refreshes = 0;
  const { store, call } = await setup(t, async (url, init) => {
    if (!url.endsWith('/chat/completions')) { refreshes++; return json({ access_token: 'fresh-fixture-access', expires_in: 3600 }); }
    auth.push(init.headers.Authorization); return auth.length === 1 ? json({ error: 'expired' }, 401) : json(completion);
  }); login(store); useApi(store);
  const response = await call(); assert.equal(response.status, 200); await response.text();
  assert.deepEqual(auth, ['Bearer oauth-fixture-access', 'Bearer fresh-fixture-access']); assert.equal(refreshes, 1);
  assert.equal(store.read().model, 'fixture-model');
});

test('concurrent expiring-login callers share one refresh', async t => {
  let refreshes = 0;
  const { store, host } = await setup(t, async () => { refreshes++; await new Promise(resolve => setTimeout(resolve, 15)); return json({ access_token: 'shared-fixture-access', expires_in: 3600 }); });
  store.saveOAuth({ accessToken: 'old', refreshToken: 'refresh', expiresAt: Date.now() - 10 }); useApi(store);
  const result = await Promise.all([host.ensureSavedOAuthAccess(), host.ensureSavedOAuthAccess(), host.ensureSavedOAuthAccess()]);
  assert.equal(refreshes, 1); assert.ok(result.every(value => value.accessToken === 'shared-fixture-access'));
  assert.equal(store.read().model, 'fixture-model');
});

test('removed OAuth never falls back to a saved API key', async t => {
  let calls = 0;
  const { store, call } = await setup(t, async () => { calls++; return json(completion); }); login(store); useApi(store); store.clearOAuth();
  const response = await call(); assert.equal(response.status, 401); assert.equal(calls, 0);
  assert.equal((await response.json()).error, 'host_oauth_not_configured');
});

test('provider failures retain the actual status and never switch models or retry blindly', async t => {
  let calls = 0;
  const { store, call } = await setup(t, async () => { calls++; return json({ error: { message: 'quota' } }, 429); }); login(store);
  const response = await call(); assert.equal(response.status, 429); assert.equal(calls, 1); assert.deepEqual(await response.json(), { error: { message: 'quota' } });
});

test('invalid payloads are rejected before any provider request', async t => {
  let calls = 0;
  const { store, call } = await setup(t, async () => { calls++; return json(completion); }); login(store);
  for (const input of [{}, { ...payload, messages: [] }, { ...payload, model: 'invalid model' }, { ...payload, stream: 'true' }]) assert.equal((await call(input)).status, 400);
  assert.equal(calls, 0);
});

test('closing the native downstream cancels the upstream streaming request', async t => {
  let aborted;
  const settled = new Promise(resolve => { aborted = resolve; });
  const { store, server } = await setup(t, async (_url, init) => new Response(new ReadableStream({
    start(controller) { controller.enqueue(new TextEncoder().encode('data: first\n\n')); init.signal.addEventListener('abort', () => { controller.error(new Error('cancelled')); aborted(); }, { once: true }); },
  }), { headers: { 'Content-Type': 'text/event-stream' } })); login(store);
  const controller = new AbortController();
  const response = await fetch(`${server.origin}/api/model/oauth/chat/completions`, { method: 'POST', headers: { Authorization: `Bearer ${server.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ ...payload, stream: true }), signal: controller.signal });
  await response.body.getReader().read(); controller.abort();
  await Promise.race([settled, new Promise((_, reject) => { const timer = setTimeout(() => reject(new Error('upstream not cancelled')), 2000); timer.unref(); })]);
});
