import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { gzipSync } from 'node:zlib';
import { mkdtemp, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostNetworkSettings } from '../src/network-settings.mjs';
import { fetchViaProxy, networkFetch } from '../src/net-proxy.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

const directory = async t => { const root = await realpath(await mkdtemp(join(tmpdir(), 'ovi-network-settings-'))); t.after(() => rm(root, { recursive: true, force: true })); return root; };
const listen = server => new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));

test('host network modes persist independently and redact saved and inherited proxy credentials', async t => {
  const root = await directory(t), environment = { HTTPS_PROXY: 'http://env-user:env-password@proxy.invalid:7000/path?secret=hidden#fragment', NO_PROXY: '.internal' };
  const owner = new HostNetworkSettings(root, { env: environment });
  environment.HTTPS_PROXY = 'http://changed.invalid:1';
  let view = owner.publicView({ targets: { oauth: 'https://auth.x.ai/oauth2/device/code?credential=hidden' } });
  assert.equal(view.routes.oauth.proxyEndpoint, 'http://proxy.invalid:7000'); assert.equal(view.routes.oauth.target, 'https://auth.x.ai');
  assert.equal(view.routes.oauth.environmentVariable, 'HTTPS_PROXY'); assert.equal(view.routes.oauth.source, 'inherited-environment');
  for (const secret of ['env-user', 'env-password', 'hidden', 'fragment']) assert(!JSON.stringify(view).includes(secret));
  owner.save({ mode: 'manual', proxyUrl: 'http://manual-user:manual-password@127.0.0.1:7890', noProxy: '.internal,example.test:443' });
  view = owner.publicView({ targets: { provider: 'https://example.test:443/api' } });
  assert.equal(view.routes.provider.source, 'manual-bypass'); assert.equal(view.manualProxyEndpoint, 'http://127.0.0.1:7890');
  assert.equal(view.manualProxyCredentialsConfigured, true); assert(!JSON.stringify(view).includes('manual-password'));
  assert.equal((await stat(owner.file)).mode & 0o777, 0o600);
  const restored = new HostNetworkSettings(root, { env: {} });
  assert.equal(restored.route('https://external.test/').proxyUrl, 'http://manual-user:manual-password@127.0.0.1:7890/');
  restored.save({ mode: 'direct' }); assert.equal(restored.route('https://external.test/').proxyUrl, null);
  restored.save({ mode: 'manual' }); assert.equal(restored.route('https://external.test/').publicRoute.source, 'manual-proxy');
  restored.save({ mode: 'system', clearProxy: true }); assert.equal(restored.publicView().manualProxyConfigured, false);
  assert.equal(environment.HTTPS_PROXY, 'http://changed.invalid:1', 'saving does not mutate inherited environment');
});

test('direct, manual bypass and loopback routing use only host policy; a manual failure never falls back', async t => {
  const owner = new HostNetworkSettings(await directory(t), { env: { HTTPS_PROXY: 'http://environment.invalid:1', NO_PROXY: '*' } });
  const calls = [];
  const fetch = owner.createFetch({ baseFetch: async (url, init) => { calls.push({ route: 'direct', url, signal: init.signal }); return new Response('direct'); }, viaProxy: async (url, init, proxy) => { calls.push({ route: 'proxy', proxy, signal: init.signal }); throw new TypeError('synthetic proxy failure'); } });
  owner.save({ mode: 'manual', proxyUrl: 'http://manual.invalid:2', noProxy: '.internal.test' });
  const abort = new AbortController();
  await assert.rejects(fetch('https://external.test/', { signal: abort.signal }));
  assert.equal(calls.length, 1); assert.equal(calls[0].route, 'proxy'); assert.equal(calls[0].signal, abort.signal);
  assert.equal(owner.publicView().lastRequest.outcome, 'error');
  await fetch('https://some.internal.test/'); await fetch('http://127.0.0.1:1234/');
  owner.save({ mode: 'direct' }); await fetch('https://external.test/');
  assert.deepEqual(calls.map(call => call.route), ['proxy', 'direct', 'direct', 'direct']);
});

test('manual validation rejects unsupported protocols and sensitive URL suffixes without echoing inputs', async t => {
  const owner = new HostNetworkSettings(await directory(t), { env: {} });
  for (const proxyUrl of ['socks5://secret:password@proxy.test:1', 'http://secret:password@proxy.test/path?token=secret', 'http://proxy.test/#secret', 'http://user%0A:secret@proxy.test/']) {
    assert.throws(() => owner.save({ mode: 'manual', proxyUrl }), error => error.code === 'host_network_proxy_invalid' && !JSON.stringify(error).includes('secret'));
  }
  assert.throws(() => owner.save({ mode: 'manual' }), { code: 'host_network_proxy_required' });
  assert.throws(() => owner.save({ mode: 'system', noProxy: 'user:password@proxy.test' }), { code: 'host_network_bypass_invalid' });
  owner.save({ mode: 'direct' }); await writeFile(owner.file, '{broken');
  assert.equal(owner.publicView().valid, false);
  await assert.rejects(owner.createFetch({ baseFetch: () => assert.fail('corrupt config cannot route') })('https://external.test/'));
  owner.save({ mode: 'system' }); assert.equal(owner.publicView().valid, true);
});

test('system mode reports unsupported inherited proxy values and actual routing errors never expose userinfo', async t => {
  const owner = new HostNetworkSettings(await directory(t), { env: { HTTPS_PROXY: 'socks5://secret:password@proxy.test:1' } });
  assert.equal(owner.route('https://example.test').publicRoute.reason, 'unsupported-environment-proxy');
  const error = new TypeError('fetch failed at http://secret:password@proxy.test/path?token=secret');
  await assert.rejects(networkFetch(async () => { throw error; }, 'https://auth.x.ai/', {}), value => value.code === 'host_provider_network_unreachable' && !JSON.stringify(value).includes('secret'));
});

test('network HTTP API requires the session, does not reveal proxy credentials, and preserves model authentication', async t => {
  const host = new AgentHost({ dataDir: await directory(t), networkEnvironment: {} });
  const web = await serveHost(host); t.after(async () => { await web.close(); await host.close(); });
  host.providerStore.save({ provider: 'xai', apiKey: 'synthetic-model-key' });
  const post = headers => fetch(`${web.origin}/api/settings/network`, { method: 'POST', headers: { 'Content-Type': 'application/json', ...headers }, body: JSON.stringify({ mode: 'manual', proxyUrl: 'http://network-user:network-password@127.0.0.1:7890' }) });
  assert.equal((await post({})).status, 403);
  const saved = await post({ 'X-Oviraptor-Session': web.token }); assert.equal(saved.status, 200);
  const content = await saved.text(); assert(!content.includes('network-password')); assert(!content.includes('network-user')); assert(!content.includes('synthetic-model-key'));
  const network = await (await fetch(`${web.origin}/api/settings/network`, { headers: { 'X-Oviraptor-Session': web.token } })).json();
  assert.equal(network.manualProxyEndpoint, 'http://127.0.0.1:7890'); assert.equal(host.providerStore.bearer(), 'synthetic-model-key');
  host.runs.set('synthetic', {}); assert.throws(() => host.setNetwork({ mode: 'direct' }), { code: 'host_runtime_busy' }); host.runs.delete('synthetic');
});

test('production OAuth and provider paths use the saved manual proxy without contacting the real services', async t => {
  const connects = [];
  const proxy = http.createServer(); proxy.on('connect', (request, socket) => { connects.push({ target: request.url, authorization: request.headers.authorization }); socket.end('HTTP/1.1 403 Forbidden\r\n\r\n'); });
  const port = await listen(proxy); t.after(() => proxy.close());
  const host = new AgentHost({ dataDir: await directory(t), networkEnvironment: {} });
  const web = await serveHost(host); t.after(async () => { await web.close(); await host.close(); });
  host.setNetwork({ mode: 'manual', proxyUrl: `http://proxy-user:proxy-password@127.0.0.1:${port}` });
  await assert.rejects(host.startProviderOAuth({ background: false }), { code: 'host_provider_network_unreachable' });
  await host.setProvider({ provider: 'xai', apiKey: 'synthetic-model-key' }); await host.setRuntime('xai-api');
  const project = host.store.createProject('synthetic'), conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: 'hello' }); await host.runs.get(conversation.id).completion;
  assert.deepEqual(connects.map(call => call.target), ['auth.x.ai:443', 'api.x.ai:443']);
  assert(connects.every(call => call.authorization === undefined), 'model credentials are not CONNECT headers');
  const view = host.networkSettings(); assert.equal(view.lastRequest.source, 'manual-proxy'); assert.equal(view.lastRequest.outcome, 'error');
  assert(!JSON.stringify(view).includes('proxy-password')); assert(!JSON.stringify(view).includes('proxy-user'));
});

test('HTTPS CONNECT transports TLS bytes through its tunnel and never opens a second direct socket', async t => {
  let connections = 0, tunnelBytes = 0; const sockets = new Set();
  const target = net.createServer(socket => { connections++; sockets.add(socket); socket.on('data', () => socket.destroy()); });
  const targetPort = await listen(target);
  const proxy = http.createServer(); proxy.on('connect', (request, socket) => {
    sockets.add(socket); socket.on('error', () => {});
    const upstream = net.connect(targetPort, '127.0.0.1', () => {
      socket.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      socket.on('data', bytes => { tunnelBytes += bytes.length; }); socket.pipe(upstream); upstream.pipe(socket);
    }); sockets.add(upstream); upstream.on('error', () => {});
  });
  const proxyPort = await listen(proxy);
  try {
    // The local endpoint closes on ClientHello. No trusted CA, real account or
    // external DNS/network is needed to prove that the TLS bytes use CONNECT.
    await assert.rejects(fetchViaProxy(`https://127.0.0.1:${targetPort}/`, { signal: AbortSignal.timeout(2000) }, `http://127.0.0.1:${proxyPort}`));
    assert.equal(connections, 1); assert(tunnelBytes > 0);
  } finally { for (const socket of sockets) socket.destroy(); proxy.close(); target.close(); }
});

test('cancelling a provider request closes its pending manual CONNECT and does not change other credentials', { timeout: 10_000 }, async t => {
  let enteredResolve, closedResolve; const entered = new Promise(resolve => { enteredResolve = resolve; }), closed = new Promise(resolve => { closedResolve = resolve; }), sockets = new Set();
  const proxy = http.createServer(); proxy.on('connect', (request, socket) => { sockets.add(socket); socket.on('error', () => {}); socket.on('end', () => socket.destroy()); socket.on('close', () => { sockets.delete(socket); closedResolve(); }); socket.resume(); enteredResolve(); });
  const port = await listen(proxy); t.after(() => { for (const socket of sockets) socket.destroy(); proxy.close(); });
  const host = new AgentHost({ dataDir: await directory(t), networkEnvironment: {} });
  const web = await serveHost(host); t.after(async () => { await web.close(); await host.close(); });
  host.setNetwork({ mode: 'manual', proxyUrl: `http://127.0.0.1:${port}` });
  await host.setProvider({ provider: 'xai', apiKey: 'synthetic-cancellation-key' }); await host.setRuntime('xai-api');
  const project = host.store.createProject('synthetic cancellation'), conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: 'hello' }); const run = host.runs.get(conversation.id);
  await entered; assert.throws(() => host.setNetwork({ mode: 'direct' }), { code: 'host_runtime_busy' });
  await host.cancel(conversation.id); await run.completion;
  await closed; assert.equal(sockets.size, 0);
  assert.equal(host.store.messages(conversation.id).at(-1).status, 'cancelled');
  assert.equal(host.networkSettings().lastRequest.outcome, 'cancelled'); assert.equal(host.providerStore.bearer(), 'synthetic-cancellation-key');
  host.setNetwork({ mode: 'direct' }); assert.equal(host.networkSettings().mode, 'direct');
});

test('direct mode bypasses Node global proxy settings in an isolated child process', async t => {
  let directRequests = 0, proxyRequests = 0;
  const origin = http.createServer((request, response) => { directRequests++; response.writeHead(200, { 'Content-Encoding': 'gzip' }); response.end(gzipSync('direct-origin')); });
  const proxy = http.createServer((request, response) => { proxyRequests++; response.end('unexpected-proxy'); });
  const originPort = await listen(origin), proxyPort = await listen(proxy), root = await directory(t);
  t.after(() => { origin.close(); proxy.close(); });
  const moduleUrl = new URL('../src/network-settings.mjs', import.meta.url).href;
  const script = `import {HostNetworkSettings} from ${JSON.stringify(moduleUrl)}; const owner=new HostNetworkSettings(${JSON.stringify(root)}); owner.save({mode:'direct'}); const response=await owner.createFetch()(${JSON.stringify(`http://127.0.0.1:${originPort}/`)}); process.stdout.write(await response.text());`;
  const result = await promisify(execFile)(process.execPath, ['--input-type=module', '--eval', script], { env: { NODE_USE_ENV_PROXY: '1', HTTP_PROXY: `http://127.0.0.1:${proxyPort}`, NO_PROXY: '' }, timeout: 5000 });
  assert.equal(result.stdout, 'direct-origin'); assert.equal(directRequests, 1); assert.equal(proxyRequests, 0);
});
