import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { ProviderStore } from '../src/provider.mjs';
import {
  XAI_OAUTH, requestDeviceCode, pollDeviceCodeOnce, pollDeviceCodeUntilDone,
  normalizeOAuthTokens, refreshAccessToken, createPkcePair, buildAuthorizeUrl,
  accessTokenIsExpiring, publicDevicePending,
} from '../src/xai-oauth.mjs';

const tmp = async t => {
  const d = await mkdtemp(join(tmpdir(), 'ovi-oauth-'));
  t.after(() => rm(d, { recursive: true, force: true }));
  return d;
};

function mockFetch(handler) {
  return async (url, init = {}) => handler(String(url), init);
}

test('requestDeviceCode posts form body with public client_id and referrer oviraptor', async () => {
  let captured = '';
  const device = await requestDeviceCode({
    fetch: mockFetch(async (_url, init) => {
      assert.equal(init.method, 'POST');
      assert.match(init.headers['Content-Type'], /application\/x-www-form-urlencoded/);
      captured = init.body;
      return Response.json({
        device_code: 'DC-1',
        user_code: 'ABCD-EFGH',
        verification_uri: 'https://auth.x.ai/device',
        verification_uri_complete: 'https://auth.x.ai/device?user_code=ABCD-EFGH',
        expires_in: 300,
        interval: 5,
      });
    }),
  });
  const parsed = new URLSearchParams(captured);
  assert.equal(parsed.get('client_id'), XAI_OAUTH.CLIENT_ID);
  assert.match(parsed.get('scope'), /offline_access/);
  assert.match(parsed.get('scope'), /grok-cli:access/);
  assert.match(parsed.get('scope'), /api:access/);
  assert.equal(parsed.get('referrer'), 'oviraptor');
  assert.equal(device.user_code, 'ABCD-EFGH');
  assert.ok(device.verification_uri_complete);
});

test('pollDeviceCodeOnce: pending / slow_down / authorized / denied', async () => {
  const pending = await pollDeviceCodeOnce('DC', {
    fetch: mockFetch(async () => Response.json({ error: 'authorization_pending' }, { status: 400 })),
  });
  assert.equal(pending.status, 'pending');

  const slow = await pollDeviceCodeOnce('DC', {
    fetch: mockFetch(async () => Response.json({ error: 'slow_down' }, { status: 400 })),
  });
  assert.equal(slow.status, 'pending');
  assert.equal(slow.slowDown, true);

  const ok = await pollDeviceCodeOnce('DC', {
    fetch: mockFetch(async (_url, init) => {
      const body = new URLSearchParams(init.body);
      assert.equal(body.get('grant_type'), XAI_OAUTH.DEVICE_CODE_GRANT);
      assert.equal(body.get('client_id'), XAI_OAUTH.CLIENT_ID);
      assert.equal(body.get('device_code'), 'DC');
      return Response.json({ access_token: 'AT-mock', refresh_token: 'RT-mock', expires_in: 3600 });
    }),
  });
  assert.equal(ok.status, 'authorized');
  assert.equal(ok.tokens.access_token, 'AT-mock');
  assert.equal(ok.tokens.refresh_token, 'RT-mock');

  const denied = await pollDeviceCodeOnce('DC', {
    fetch: mockFetch(async () => Response.json({ error: 'access_denied' }, { status: 400 })),
  });
  assert.equal(denied.status, 'denied');
});

test('pollDeviceCodeUntilDone resolves after pending then authorized; respects slow_down', async () => {
  let n = 0;
  const sleeps = [];
  const tokens = await pollDeviceCodeUntilDone(
    { device_code: 'DC', expires_in: 60, interval: 1 },
    {
      sleep: async ms => { sleeps.push(ms); },
      now: (() => { let t = 0; return () => (t += 10); })(),
      fetch: mockFetch(async () => {
        n += 1;
        if (n === 1) return Response.json({ error: 'authorization_pending' }, { status: 400 });
        if (n === 2) return Response.json({ error: 'slow_down' }, { status: 400 });
        return Response.json({ access_token: 'AT', refresh_token: 'RT', expires_in: 120 });
      }),
    },
  );
  assert.equal(tokens.access_token, 'AT');
  assert.ok(sleeps.length >= 2);
  assert.ok(sleeps[1] > sleeps[0]); // slow_down bumped interval
});

test('PKCE helpers produce authorize URL with plan=generic', () => {
  const pkce = createPkcePair();
  assert.ok(pkce.verifier.length > 20);
  assert.ok(pkce.challenge.length > 20);
  const url = buildAuthorizeUrl({
    redirectUri: `http://127.0.0.1:${XAI_OAUTH.LOOPBACK_PORT}/callback`,
    challenge: pkce.challenge,
    state: pkce.state,
  });
  const parsed = new URL(url);
  assert.equal(parsed.origin + parsed.pathname, 'https://auth.x.ai/oauth2/authorize');
  assert.equal(parsed.searchParams.get('client_id'), XAI_OAUTH.CLIENT_ID);
  assert.equal(parsed.searchParams.get('code_challenge_method'), 'S256');
  assert.equal(parsed.searchParams.get('plan'), 'generic');
  assert.equal(parsed.searchParams.get('referrer'), 'oviraptor');
});

test('normalizeOAuthTokens + ProviderStore.saveOAuth stores 0600 and never leaks in publicView', async t => {
  const dir = await tmp(t);
  const store = new ProviderStore(dir);
  const normalized = normalizeOAuthTokens({ access_token: 'AT-secret-value-xxxx', refresh_token: 'RT-secret', expires_in: 100 });
  const view = store.saveOAuth(normalized);
  assert.equal(view.oauthConfigured, true);
  assert.equal(view.oauthAvailable, true);
  assert.equal(view.authentication, 'oauth');
  assert.equal(view.apiKeyConfigured, false);
  assert.equal(view.configured, true);
  assert.ok(!JSON.stringify(view).includes('AT-secret'));
  assert.ok(!JSON.stringify(view).includes('RT-secret'));
  assert.equal((await stat(store.file)).mode & 0o777, 0o600);
  assert.equal(store.bearer(), 'AT-secret-value-xxxx');
});

test('publicDevicePending never includes device_code', () => {
  const pub = publicDevicePending({
    device_code: 'SECRET-DC',
    user_code: 'ABCD',
    verification_uri: 'https://auth.x.ai/device',
    status: 'pending',
    expires_at: 1,
  });
  assert.equal(pub.userCode, 'ABCD');
  assert.ok(!JSON.stringify(pub).includes('SECRET-DC'));
  assert.ok(!('device_code' in pub));
});

test('HTTP oauth start → poll → authorized switches runtime to xai-api; logout clears', async t => {
  const dir = await tmp(t);
  let phase = 'device';
  const oauthFetch = mockFetch(async (url, init) => {
    if (url.includes('/device/code')) {
      return Response.json({
        device_code: 'DC-HTTP',
        user_code: 'WXYZ-1234',
        verification_uri: 'https://auth.x.ai/device',
        verification_uri_complete: 'https://auth.x.ai/device?user_code=WXYZ-1234',
        expires_in: 120,
        interval: 1,
      });
    }
    if (url.includes('/token')) {
      const body = new URLSearchParams(init.body);
      assert.equal(body.get('device_code'), 'DC-HTTP');
      if (phase === 'pending') return Response.json({ error: 'authorization_pending' }, { status: 400 });
      return Response.json({ access_token: 'AT-http-mock-token', refresh_token: 'RT-http-mock', expires_in: 3600 });
    }
    throw new Error(`unexpected url ${url}`);
  });

  const host = new AgentHost({ dataDir: dir });
  host.oauthFetch = oauthFetch;
  // Disable background poll racing by cancelling after start and using manual poll.
  const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); });

  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  phase = 'pending';
  const start = await fetch(`${web.origin}/api/settings/provider/oauth/start`, {
    method: 'POST', headers, body: JSON.stringify({ background: false }),
  });
  const startBody = await start.json();
  assert.equal(start.status, 200);
  assert.equal(startBody.status, 'pending');
  assert.equal(startBody.userCode, 'WXYZ-1234');
  assert.ok(startBody.verificationUriComplete);
  assert.ok(!JSON.stringify(startBody).includes('DC-HTTP'));
  assert.ok(!JSON.stringify(startBody).includes('AT-http'));

  phase = 'pending';
  const pendingPoll = await fetch(`${web.origin}/api/settings/provider/oauth/poll`, { method: 'POST', headers, body: '{}' });
  assert.equal((await pendingPoll.json()).status, 'pending');

  phase = 'ok';
  const done = await fetch(`${web.origin}/api/settings/provider/oauth/poll`, { method: 'POST', headers, body: '{}' });
  const doneBody = await done.json();
  assert.equal(done.status, 200);
  assert.equal(doneBody.status, 'authorized');
  assert.equal(doneBody.apiProvider.oauthConfigured, true);
  assert.equal(doneBody.runtime.id, 'xai-api');
  assert.ok(!JSON.stringify(doneBody).includes('AT-http-mock-token'));
  assert.ok(!JSON.stringify(doneBody).includes('RT-http-mock'));

  const logout = await fetch(`${web.origin}/api/settings/provider/oauth/logout`, { method: 'POST', headers, body: '{}' });
  const logoutBody = await logout.json();
  assert.equal(logout.status, 200);
  assert.equal(logoutBody.apiProvider.oauthConfigured, false);
  assert.equal(logoutBody.runtime.id, 'none');
});

test('refreshAccessToken posts refresh_token grant', async () => {
  const tokens = await refreshAccessToken('RT-old', {
    fetch: mockFetch(async (_url, init) => {
      const body = new URLSearchParams(init.body);
      assert.equal(body.get('grant_type'), 'refresh_token');
      assert.equal(body.get('refresh_token'), 'RT-old');
      assert.equal(body.get('client_id'), XAI_OAUTH.CLIENT_ID);
      return Response.json({ access_token: 'AT-new', refresh_token: 'RT-new', expires_in: 60 });
    }),
  });
  assert.equal(tokens.access_token, 'AT-new');
  assert.equal(tokens.refresh_token, 'RT-new');
});

test('accessTokenIsExpiring reads JWT exp without verifying signature', () => {
  const header = Buffer.from(JSON.stringify({ alg: 'none' })).toString('base64url');
  const expSoon = Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 30 })).toString('base64url');
  const expLater = Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 10_000 })).toString('base64url');
  assert.equal(accessTokenIsExpiring(`${header}.${expSoon}.x`, 120_000), true);
  assert.equal(accessTokenIsExpiring(`${header}.${expLater}.x`, 120_000), false);
  assert.equal(accessTokenIsExpiring('opaque-token'), false);
});
