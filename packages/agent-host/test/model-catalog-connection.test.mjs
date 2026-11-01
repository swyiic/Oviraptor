import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { normalizeProviderModel, validateProviderConfig, ProviderStore } from '../src/provider.mjs';

const connectionRef = { kind: 'models', policyId: 'shared-models', revision: 7, modelProfileId: 'model-1' };
const bound = input => ({ provider: 'openai-compatible', baseUrl: 'https://gateway.example/v1', apiKey: 'synthetic-secret', connectionRef, ...input });

test('catalog wire models match Rust: only the exact openai prefix is removed', () => {
  for (const [model, wire] of [
    ['openai/gpt-4.1', 'gpt-4.1'], ['openai/deepseek-flash', 'deepseek-flash'],
    ['deepseek/deepseek-flash', 'deepseek/deepseek-flash'], ['xai/grok-custom', 'xai/grok-custom'],
    ['org/custom-model', 'org/custom-model'], ['OpenAI/custom', 'OpenAI/custom'],
  ]) assert.equal(validateProviderConfig(bound({ model })).model, wire);
});

test('catalog model names never change an explicit provider endpoint', () => {
  for (const [model, baseUrl] of [
    ['deepseek-flash', 'https://api.x.ai/v1'], ['grok-4.7', 'https://api.deepseek.com/v1'],
    ['openai/deepseek-flash', 'https://proxy.example/tenant/v1'],
  ]) {
    const value = validateProviderConfig(bound({ model, baseUrl }));
    assert.equal(value.provider, 'openai-compatible'); assert.equal(value.baseUrl, baseUrl);
  }
});

test('catalog accepts explicitly configured LAN endpoints and short credentials', () => {
  const value = validateProviderConfig(bound({ model: 'local/custom', baseUrl: 'http://192.0.2.20:9000/v1', apiKey: 'test' }));
  assert.equal(value.baseUrl, 'http://192.0.2.20:9000/v1'); assert.equal(value.apiKey, 'test');
  for (const apiKey of ['', 'key\nheader', 'key\u007f']) assert.throws(() => validateProviderConfig(bound({ model: 'm', apiKey })), /host_provider_api_key_invalid/);
});

test('catalog endpoint validation retains scheme and embedded credential restrictions', () => {
  for (const baseUrl of ['file:///tmp/model', 'https://user:password@gateway.example/v1', 'https://gateway.example/v1?token=secret', 'https://gateway.example/v1#fragment']) {
    assert.throws(() => validateProviderConfig(bound({ model: 'm', baseUrl })), /host_provider_base_url_invalid/);
  }
  assert.throws(() => validateProviderConfig(bound({ model: 'm', connectionRef: { ...connectionRef, revision: 0 } })), /host_provider_connection_ref_invalid/);
});

test('a saved catalog connection keeps its endpoint, exact model and key on reload', t => {
  const dir = mkdtempSync(join(tmpdir(), 'ovi-bound-provider-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = new ProviderStore(dir);
  store.save(bound({ model: 'grok-custom', baseUrl: 'https://api.deepseek.com/v1', apiKey: 'short', authType: 'api-key' }));
  const reloaded = new ProviderStore(dir);
  assert.equal(reloaded.read().model, 'grok-custom'); assert.equal(reloaded.read().baseUrl, 'https://api.deepseek.com/v1');
  assert.equal(reloaded.read().apiKey, 'short'); assert.deepEqual(reloaded.publicView().connectionRef, connectionRef);
});

test('legacy unbound presets retain their existing inference and validation', () => {
  assert.equal(normalizeProviderModel({ provider: 'xai', baseUrl: 'https://api.x.ai/v1', model: 'openai/deepseek-chat' }).baseUrl, 'https://api.deepseek.com');
  assert.equal(normalizeProviderModel({ provider: 'openai-compatible', baseUrl: 'https://api.deepseek.com', model: 'xai/grok-4.7' }).model, 'grok-4.7');
  assert.throws(() => validateProviderConfig({ provider: 'openai-compatible', model: 'm', baseUrl: 'http://192.0.2.20:9000/v1', apiKey: 'synthetic-secret' }), /host_provider_base_url_invalid/);
  assert.throws(() => validateProviderConfig({ provider: 'openai-compatible', model: 'm', baseUrl: 'https://gateway.example/v1', apiKey: 'short' }), /host_provider_api_key_invalid/);
});

test('catalog readback never normalizes an already resolved wire model twice', t => {
  const dir = mkdtempSync(join(tmpdir(), 'ovi-bound-wire-model-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = new ProviderStore(dir);
  store.save(bound({ model: 'openai/openai/custom', authType: 'api-key' }));
  assert.equal(store.publicView().model, 'openai/custom');
  assert.equal(new ProviderStore(dir).read().model, 'openai/custom');
  store.save(bound({ model: 'openai/openai/local', baseUrl: 'http://127.0.0.1:9000/v1', apiKey: undefined, authType: 'none', clearApiKey: true }));
  assert.equal(store.publicView().model, 'openai/local');
  assert.equal(new ProviderStore(dir).read().model, 'openai/local');
});
