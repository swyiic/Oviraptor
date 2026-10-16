import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { readFileSync } from 'node:fs';
import { GrokAcpRuntime } from '/workspace/Oviraptor-agent-first-20261016/packages/runtime-acp/index.mjs';

// Synthetic transport replays the separately recorded official initialize
// response. No official executable, account, network, auth or prompt is used.
const actual = JSON.parse(readFileSync(new URL('./initialize-only-02/initialize-response.json', import.meta.url))).result;
function fixture(init = actual, authError = null) {
  const sent = [], events = [];
  const child = new EventEmitter(); child.stdout = new PassThrough(); child.stderr = new PassThrough(); child.pid = 123;
  child.stdin = new EventEmitter(); child.kill = () => true;
  child.stdin.write = (wire, callback) => {
    const request = JSON.parse(wire); sent.push(request);
    queueMicrotask(() => {
      const response = request.method === 'initialize' ? { result: init }
        : authError ? { error: authError } : { result: {} };
      child.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, ...response }) + '\n');
      callback?.();
    });
    return true;
  };
  const runtime = new GrokAcpRuntime({ cwd: '/work', command: 'synthetic-only', spawn: () => child,
    requestTimeoutMs: 1000, onEvent: event => events.push(event) });
  return { runtime, sent, events };
}

test('recorded official blank-profile advertisement never becomes authenticated readiness or a login call', async () => {
  const f = fixture();
  await assert.rejects(f.runtime.start(), { code: 'ACP_CACHED_AUTH_UNAVAILABLE' });
  assert.deepEqual(f.sent.map(item => item.method), ['initialize']);
  assert.equal(f.runtime.status.state, 'failed');
  assert(!f.events.some(item => item.type === 'runtime_ready'));
});

test('unadvertised cached auth never falls back to grok.com oidc or api-key from the Host adapter', async () => {
  for (const id of ['grok.com', 'oidc', 'xai.api_key']) {
    const f = fixture({ ...actual, authMethods: [{ id, name: 'Synthetic alternative only' }] });
    await assert.rejects(f.runtime.start(), { code: 'ACP_CACHED_AUTH_UNAVAILABLE' });
    assert.deepEqual(f.sent.map(item => item.method), ['initialize']);
  }
});

test('synthetic cached-auth remote rejection never produces ready session or prompt', async () => {
  const f = fixture({ ...actual, authMethods: [{ id: 'cached_token', name: 'Synthetic only' }] },
    { code: -32000, message: 'Synthetic rejection, not an observed official authentication result' });
  await assert.rejects(f.runtime.start(), { code: 'ACP_REMOTE_ERROR' });
  assert.deepEqual(f.sent.map(item => item.method), ['initialize', 'authenticate']);
  assert.equal(f.sent[1].params.methodId, 'cached_token');
  assert.equal(f.sent[1].params._meta.headless, true);
  assert(!f.events.some(item => item.type === 'runtime_ready'));
});

test('PROPOSED contract: pre-work missing authentication is classified explicitly with no unknown outcome', async () => {
  const f = fixture();
  await assert.rejects(f.runtime.start(), error => {
    assert.equal(error.details.authenticationRequired, true);
    assert.equal(error.details.outcomeUnknown, false);
    return true;
  });
});
