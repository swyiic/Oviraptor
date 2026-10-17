import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { proxyForUrl, noProxyMatches, createProxyFetch, networkFetch } from '../src/net-proxy.mjs';
import { requestDeviceCode } from '../src/xai-oauth.mjs';

const listen = server => new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));

test('proxyForUrl reads HTTPS_PROXY / HTTP_PROXY / ALL_PROXY (case-insensitive)', () => {
  assert.equal(proxyForUrl('https://auth.x.ai/x', { HTTPS_PROXY: 'http://127.0.0.1:7890' }), 'http://127.0.0.1:7890/');
  assert.equal(proxyForUrl('https://auth.x.ai/x', { https_proxy: '127.0.0.1:7890' }), 'http://127.0.0.1:7890/');
  assert.equal(proxyForUrl('http://example.com/', { HTTP_PROXY: 'http://p:1' }), 'http://p:1/');
  assert.equal(proxyForUrl('https://api.x.ai/v1', { ALL_PROXY: 'http://p:2' }), 'http://p:2/');
  assert.equal(proxyForUrl('https://api.x.ai/v1', { ALL_PROXY: 'socks5://p:2' }), null);
  assert.equal(proxyForUrl('https://api.x.ai/v1', {}), null);
});

test('NO_PROXY and loopback bypass the proxy', () => {
  const env = { HTTPS_PROXY: 'http://127.0.0.1:7890', HTTP_PROXY: 'http://127.0.0.1:7890', NO_PROXY: '.internal.dev,api.x.ai' };
  for (const url of ['http://127.0.0.1:4100/api', 'http://localhost:3000/', 'http://[::1]:80/', 'https://foo.internal.dev/', 'https://api.x.ai/v1']) {
    assert.equal(proxyForUrl(url, env), null, url);
  }
  assert.equal(noProxyMatches('http://127.0.0.1:1/', {}), true);
  assert.equal(proxyForUrl('https://auth.x.ai/', env), 'http://127.0.0.1:7890/');
});

test('proxy fetch routes through HTTP_PROXY and bypasses loopback', async () => {
  const seen = [];
  const proxy = http.createServer((req, res) => { seen.push(req.url); res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"via":"proxy"}'); });
  const proxyPort = await listen(proxy);
  const origin = http.createServer((req, res) => res.end('{"via":"direct"}'));
  const originPort = await listen(origin);
  try {
    const f = createProxyFetch({ env: { HTTP_PROXY: `http://127.0.0.1:${proxyPort}` } });
    const viaProxy = await f('http://upstream.example/token?x=1', { method: 'POST', body: 'a=b' });
    assert.deepEqual(await viaProxy.json(), { via: 'proxy' });
    assert.deepEqual(seen, ['http://upstream.example/token?x=1']);
    const direct = await f(`http://127.0.0.1:${originPort}/`);
    assert.deepEqual(await direct.json(), { via: 'direct' });
    assert.equal(seen.length, 1);
  } finally { proxy.close(); origin.close(); }
});

test('HTTPS_PROXY is used via CONNECT; failure maps to host_provider_network_unreachable', async () => {
  const connects = [];
  const proxy = http.createServer();
  proxy.on('connect', (req, socket) => { connects.push(req.url); socket.end('HTTP/1.1 403 Forbidden\r\n\r\n'); });
  const port = await listen(proxy);
  try {
    const f = createProxyFetch({ env: { HTTPS_PROXY: `http://127.0.0.1:${port}` } });
    await assert.rejects(requestDeviceCode({ fetch: f }), err => err.code === 'host_provider_network_unreachable' && err.status === 502);
    assert.deepEqual(connects, ['auth.x.ai:443']);
  } finally { proxy.close(); }
});

test('plain fetch failure (TLS/syscall) maps to host_provider_network_unreachable', async () => {
  const failing = async () => { throw new TypeError('fetch failed', { cause: Object.assign(new Error('Client network socket disconnected before secure TLS connection was established'), { code: 'ECONNRESET' }) }); };
  await assert.rejects(networkFetch(failing, 'https://auth.x.ai/oauth2/device/code', {}), err => err.code === 'host_provider_network_unreachable' && err.details.host === 'auth.x.ai');
});
