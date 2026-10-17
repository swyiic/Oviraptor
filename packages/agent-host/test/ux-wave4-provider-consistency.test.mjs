import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { validateProviderConfig, ProviderStore, DEEPSEEK_BASE_URL } from '../src/provider.mjs';

test('deepseek model never rides the xAI endpoint', () => {
  const v = validateProviderConfig({ provider: 'xai', model: 'openai/deepseek-v4-flash' });
  assert.deepEqual(v, { provider: 'openai-compatible', baseUrl: DEEPSEEK_BASE_URL, model: 'deepseek-v4-flash' });
  const g = validateProviderConfig({ provider: 'openai-compatible', baseUrl: DEEPSEEK_BASE_URL, model: 'grok-4.7' });
  assert.equal(g.provider, 'xai'); assert.equal(g.baseUrl, 'https://api.x.ai/v1');
});

test('keys are kept per endpoint so switching back and forth works', () => {
  const store = new ProviderStore(mkdtempSync(join(tmpdir(), 'w4-')));
  store.save({ provider: 'xai', model: 'grok-4.7', apiKey: 'xai-key-123456' });
  assert.throws(() => store.save({ model: 'deepseek-v4-flash' }), /host_provider_api_key_required/);
  store.save({ model: 'deepseek-v4-flash', apiKey: 'ds-key-123456' });
  assert.equal(store.bearer(), 'ds-key-123456');
  store.save({ model: 'grok-4.7' });
  assert.equal(store.bearer(), 'xai-key-123456');
  assert.equal(store.read().baseUrl, 'https://api.x.ai/v1');
  store.save({ model: 'openai/deepseek-chat' });
  assert.equal(store.bearer(), 'ds-key-123456');
  assert.equal(store.read().keys, undefined);
});

test('stale xai+deepseek file is repaired on read', () => {
  const dir = mkdtempSync(join(tmpdir(), 'w4-'));
  mkdirSync(join(dir, 'secrets'));
  writeFileSync(join(dir, 'secrets', 'model-provider.json'), JSON.stringify({ provider: 'xai', baseUrl: 'https://api.x.ai/v1', model: 'openai/deepseek-v4-flash', apiKey: 'xai-key-123456', authentication: 'api-key' }));
  const store = new ProviderStore(dir);
  const prev = process.env.DEEPSEEK_API_KEY; process.env.DEEPSEEK_API_KEY = 'env-ds-key-1234';
  try {
    const v = store.read();
    assert.equal(v.provider, 'openai-compatible'); assert.equal(v.baseUrl, DEEPSEEK_BASE_URL); assert.equal(v.model, 'deepseek-v4-flash');
    assert.equal(store.bearer(), 'env-ds-key-1234');
  } finally { if (prev === undefined) delete process.env.DEEPSEEK_API_KEY; else process.env.DEEPSEEK_API_KEY = prev; }
});

test('desktop clients derive provider from model and header follows provider', async () => {
  const { readFile } = await import('node:fs/promises');
  const root = new URL('../../../src/features/agentHost/', import.meta.url);
  const settings = await readFile(new URL('settingsClient.ts', root), 'utf8');
  assert.doesNotMatch(settings, /\{ provider: "xai" \}/);
  assert.match(settings, /resolveProviderForModel/);
  const chat = await readFile(new URL('chatClient.ts', root), 'utf8');
  assert.match(chat, /providerDisplayName/);
  assert.match(chat, /setModel/);
  const presets = await readFile(new URL('providerPresets.ts', root), 'utf8');
  assert.match(presets, /https:\/\/api\.deepseek\.com/);
});
