import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { GrokAcpRuntime } from '../index.mjs';

const peer = fileURLToPath(new URL('./fixtures/authentication.mjs', import.meta.url));
async function fixture(t, mode) {
  const root = await mkdtemp(join(tmpdir(), 'acp-auth-classification-')), journal = join(root, 'sent.ndjson');
  const events = [];
  const runtime = new GrokAcpRuntime({ cwd: root, requestTimeoutMs: 1000,
    env: { PATH: '/usr/bin:/bin', LANG: 'C.UTF-8' },
    spawn(_command, _args, options) { return spawn(process.execPath, [peer, mode, journal], options); },
    onEvent: event => events.push(event) });
  t.after(async () => { await runtime.close(); await rm(root, { recursive: true, force: true }); });
  return { runtime, events, async requests() { return (await readFile(journal, 'utf8')).trim().split('\n').map(line => JSON.parse(line)); } };
}

for (const mode of ['grok-com', 'oidc', 'mixed']) test(`only advertised login methods (${mode}) require authentication without a second RPC`, async t => {
  const f = await fixture(t, mode);
  await assert.rejects(f.runtime.start(), error => {
    assert.equal(error.code, 'ACP_CACHED_AUTH_UNAVAILABLE');
    assert.deepEqual(error.details, { authenticationRequired: true, outcomeUnknown: false });
    return true;
  });
  assert.equal(f.runtime.status.authenticationRequired, true);
  assert.equal(f.runtime.status.state, 'failed'); // Transport remains closed and unusable.
  assert.deepEqual((await f.requests()).map(request => request.method), ['initialize']);
  assert.deepEqual(f.events.filter(event => event.type === 'runtime_error').map(({ code, authenticationRequired, outcomeUnknown }) => ({ code, authenticationRequired, outcomeUnknown })),
    [{ code: 'ACP_CACHED_AUTH_UNAVAILABLE', authenticationRequired: true, outcomeUnknown: false }]);
  assert(!f.events.some(event => event.type === 'runtime_ready'));
});

for (const mode of ['empty', 'unknown', 'api-key']) test(`unsupported advertisement (${mode}) preserves the prior explicit failure and never chooses another method`, async t => {
  const f = await fixture(t, mode);
  await assert.rejects(f.runtime.start(), error => {
    assert.equal(error.code, 'ACP_CACHED_AUTH_UNAVAILABLE');
    assert.equal(error.details.authenticationRequired, undefined);
    return true;
  });
  assert.equal(f.runtime.status.authenticationRequired, undefined);
  assert.deepEqual((await f.requests()).map(request => request.method), ['initialize']);
  assert.equal(f.events.find(event => event.type === 'runtime_error').outcomeUnknown, true);
});

test('existing cached-auth success still requires its actual acknowledgement before ready', async t => {
  const f = await fixture(t, 'cached'); await f.runtime.start();
  const requests = await f.requests();
  assert.deepEqual(requests.map(request => request.method), ['initialize', 'authenticate']);
  assert.deepEqual(requests[1].params, { methodId: 'cached_token', _meta: { headless: true } });
  assert.equal(f.runtime.status.state, 'ready'); assert.equal(f.runtime.status.authenticationRequired, undefined);
  assert.equal(f.events.filter(event => event.type === 'runtime_ready').length, 1);
});

test('rejected cached authentication preserves remote failure and unknown-outcome handling', async t => {
  const f = await fixture(t, 'auth-rejected');
  await assert.rejects(f.runtime.start(), { code: 'ACP_REMOTE_ERROR' });
  assert.deepEqual((await f.requests()).map(request => request.method), ['initialize', 'authenticate']);
  assert.equal(f.runtime.status.authenticationRequired, undefined);
  assert.equal(f.events.find(event => event.type === 'runtime_error').outcomeUnknown, true);
  assert(!f.events.some(event => event.type === 'runtime_ready'));
  assert(!JSON.stringify(f.events).includes('Synthetic private diagnostic'));
});
