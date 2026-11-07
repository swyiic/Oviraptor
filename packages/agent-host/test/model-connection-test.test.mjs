import test from 'node:test';
import assert from 'node:assert/strict';
import { AgentHost } from '../src/service.mjs';

// testProvider() only touches this.providerStore + this.providerFetch, so a tiny
// fake `this` exercises every branch without constructing a full host.
function ctx({ config, bearer, fetchImpl }) {
  return { providerStore: { read: () => config, bearer: () => bearer }, providerFetch: fetchImpl };
}
const run = fake => AgentHost.prototype.testProvider.call(fake);

test('testProvider: not configured / missing credential', async () => {
  assert.deepEqual(await run(ctx({ config: null })), { ok: false, reason: 'not_configured' });
  const r = await run(ctx({ config: { baseUrl: 'https://api.deepseek.com', model: 'deepseek-chat' }, bearer: '' }));
  assert.equal(r.ok, false); assert.equal(r.reason, 'no_credential'); assert.equal(r.model, 'deepseek-chat');
});

test('testProvider: 200 /models → ok + modelListed; credential never leaked; base trimmed', async () => {
  let seenUrl = ''; let seenAuth = '';
  const fetchImpl = async (url, opts) => {
    seenUrl = url; seenAuth = opts.headers.Authorization;
    return { ok: true, status: 200, json: async () => ({ data: [{ id: 'deepseek-chat' }, { id: 'deepseek-flash' }] }) };
  };
  const r = await run(ctx({ config: { baseUrl: 'https://api.deepseek.com/', model: 'deepseek-chat' }, bearer: 'sk-secret', fetchImpl }));
  assert.equal(r.ok, true); assert.equal(r.status, 200); assert.equal(r.modelListed, true);
  assert.equal(r.baseUrl, 'https://api.deepseek.com'); // trailing slash trimmed
  assert.match(seenUrl, /^https:\/\/api\.deepseek\.com\/models$/);
  assert.equal(seenAuth, 'Bearer sk-secret');
  assert.ok(!JSON.stringify(r).includes('sk-secret')); // value never returned to the client
});

test('testProvider: configured model absent from catalog → ok but modelListed false', async () => {
  const fetchImpl = async () => ({ ok: true, status: 200, json: async () => ({ data: [{ id: 'other-model' }] }) });
  const r = await run(ctx({ config: { baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7' }, bearer: 'k', fetchImpl }));
  assert.equal(r.ok, true); assert.equal(r.modelListed, false);
});

test('testProvider: HTTP error surfaces status + detail, not ok', async () => {
  const fetchImpl = async () => ({ ok: false, status: 401, text: async () => 'unauthorized' });
  const r = await run(ctx({ config: { baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7' }, bearer: 'bad', fetchImpl }));
  assert.equal(r.ok, false); assert.equal(r.reason, 'http_error'); assert.equal(r.status, 401);
  assert.match(r.error, /unauthorized/);
});

test('testProvider: fetch throws → unreachable with message', async () => {
  const fetchImpl = async () => { throw new Error('ECONNREFUSED'); };
  const r = await run(ctx({ config: { baseUrl: 'https://nope.example/v1', model: 'm' }, bearer: 'k', fetchImpl }));
  assert.equal(r.ok, false); assert.equal(r.reason, 'unreachable'); assert.match(r.error, /ECONNREFUSED/);
});
