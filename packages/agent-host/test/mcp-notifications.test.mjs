import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { mcpHttpFixture } from './fixtures/mcp-http.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const bridgePath = fileURLToPath(new URL('../src/mcp-bridge.mjs', import.meta.url));
const waitFor = predicate => new Promise((resolve, reject) => { const deadline = Date.now() + 3000; const check = () => { if (predicate()) return resolve(); if (Date.now() > deadline) return reject(Error('Synthetic bridge observation timed out')); setTimeout(check, 5); }; check(); });
function bridge(t, endpoint, script = bridgePath) {
  const child = spawn(process.execPath, [script], { env: { PATH: process.env.PATH ?? '', OVIRAPTOR_HOST_ORIGIN: endpoint.origin, OVIRAPTOR_MCP_TOKEN: endpoint.token, OVIRAPTOR_CONVERSATION: endpoint.conversation }, stdio: ['pipe', 'pipe', 'pipe'] });
  const messages = []; let bytes = '', stderr = '', exit;
  const completion = new Promise(resolve => child.once('exit', (code, signal) => { exit = { code, signal }; resolve(exit); }));
  child.stdout.setEncoding('utf8'); child.stdout.on('data', chunk => { bytes += chunk; for (;;) { const index = bytes.indexOf('\n'); if (index < 0) break; messages.push(JSON.parse(bytes.slice(0, index))); bytes = bytes.slice(index + 1); } });
  child.stderr.setEncoding('utf8'); child.stderr.on('data', chunk => { stderr += chunk; }); child.stdin.on('error', () => {});
  t.after(async () => { if (!exit) { child.kill('SIGTERM'); await completion; } assert(!stderr.includes(endpoint.token), 'Scope token never appears in diagnostics'); assert(!stderr.includes('Unhandled'), 'Pipe errors do not escape'); });
  return { child, messages, completion, get stderr() { return stderr; }, send(message) { child.stdin.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n'); } };
}
const init = { id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'synthetic', version: '1' } } };

test('real bridge subscribes once after initialize and forwards initial/later scoped tool changes without a model poll', async t => {
  const endpoint = await mcpHttpFixture(t), peer = bridge(t, endpoint);
  peer.send(init); await waitFor(() => peer.messages.length >= 2);
  assert.equal(peer.messages[0].id, 1); assert.equal(peer.messages[0].result.capabilities.tools.listChanged, true);
  assert.deepEqual(peer.messages[1], { jsonrpc: '2.0', method: 'notifications/tools/list_changed' });
  assert.equal(endpoint.state.opened, 1); assert.deepEqual(endpoint.state.eventHeaders, [{ accept: 'text/event-stream', scoped: true }]);
  peer.send({ method: 'notifications/initialized' }); peer.send({ method: 'notifications/initialized' });
  endpoint.event({ type: 'tools.changed' }); endpoint.event({ type: 'unrelated.metadata' });
  await waitFor(() => peer.messages.filter(message => message.method === 'notifications/tools/list_changed').length === 2);
  assert.equal(endpoint.state.opened, 1); assert.equal(endpoint.state.posts.length, 1, 'Notifications do not poll tools/list or a provider');
  peer.child.stdin.end(); assert.equal((await peer.completion).code, 0); await waitFor(() => endpoint.state.closed === 1);
});

test('SSE UTF-8/CRLF/multiline events are parsed incrementally and are aborted when stdio ends', async t => {
  const endpoint = await mcpHttpFixture(t, { initialEvent: false }), peer = bridge(t, endpoint);
  peer.send(init); await waitFor(() => endpoint.state.opened === 1);
  const event = Buffer.from('data: {"type":\r\ndata: "tools.changed","label":"合成"}\r\n\r\n'), split = event.indexOf(Buffer.from('合')) + 1;
  endpoint.raw(event.subarray(0, split)); endpoint.raw(event.subarray(split));
  await waitFor(() => peer.messages.some(message => message.method === 'notifications/tools/list_changed'));
  peer.child.stdin.end(); assert.equal((await peer.completion).code, 0); await waitFor(() => endpoint.state.closed === 1);
});

test('unterminated input exceeding 64 KiB exits immediately before stdin EOF or any HTTP call', async t => {
  const endpoint = await mcpHttpFixture(t), peer = bridge(t, endpoint);
  peer.child.stdin.write(Buffer.alloc(65_537, 120));
  assert.equal((await peer.completion).code, 2); assert.equal(endpoint.state.posts.length, 0); assert.deepEqual(peer.messages, []);
});

test('exactly 64 KiB and split multibyte input remain valid JSON-RPC frames', async t => {
  const endpoint = await mcpHttpFixture(t), peer = bridge(t, endpoint);
  const request = { jsonrpc: '2.0', id: 'bounded', method: 'tools/list', params: { text: '你' } };
  request.params.text += 'x'.repeat(65_536 - Buffer.byteLength(JSON.stringify(request)));
  const frame = Buffer.from(JSON.stringify(request)), split = frame.indexOf(Buffer.from('你')) + 1;
  assert.equal(frame.length, 65_536); peer.child.stdin.write(frame.subarray(0, split)); peer.child.stdin.write(Buffer.concat([frame.subarray(split), Buffer.from('\n')]));
  await waitFor(() => peer.messages.length === 1); assert.equal(peer.messages[0].id, 'bounded'); assert.equal(endpoint.state.posts[0].params.text, request.params.text);
});

test('concurrent HTTP admission stops at sixteen and queued requests resume after slots are released', async t => {
  const held = [], endpoint = await mcpHttpFixture(t, { onPost(_request, complete) { held.push(complete); } }), peer = bridge(t, endpoint);
  for (let id = 1; id <= 32; id++) peer.send({ id, method: 'tools/list', params: {} });
  await waitFor(() => held.length === 16); assert.equal(endpoint.state.peakPosts, 16); assert.equal(endpoint.state.posts.length, 16);
  for (const complete of held.splice(0)) complete({ tools: [] });
  await waitFor(() => held.length === 16); for (const complete of held.splice(0)) complete({ tools: [] });
  await waitFor(() => peer.messages.length === 32); assert.equal(endpoint.state.peakPosts, 16); assert.equal(new Set(peer.messages.map(message => message.id)).size, 32);
});

test('malformed envelopes, invalid UTF-8 and incomplete final frames fail closed without HTTP forwarding', async t => {
  const endpoint = await mcpHttpFixture(t);
  for (const input of ['null\n', '[]\n', '{"jsonrpc":"1.0","id":1,"method":"tools/list"}\n', '{"jsonrpc":"2.0","id":null,"method":"tools/list"}\n', '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":[]}\n', '{"jsonrpc":"2.0","id":1,"result":{}}\n', Buffer.from([0xff, 10])]) {
    const peer = bridge(t, endpoint); peer.child.stdin.write(input); assert.equal((await peer.completion).code, 2);
  }
  const peer = bridge(t, endpoint); peer.child.stdin.end('{"jsonrpc":"2.0"'); assert.equal((await peer.completion).code, 2); assert.equal(endpoint.state.posts.length, 0);
});

test('closing the peer stdout contains EPIPE and aborts its SSE connection without scope logs', async t => {
  const endpoint = await mcpHttpFixture(t), peer = bridge(t, endpoint); peer.send(init); await waitFor(() => endpoint.state.opened === 1);
  peer.child.stdout.destroy(); endpoint.event({ type: 'tools.changed' });
  assert.equal((await peer.completion).code, 2); await waitFor(() => endpoint.state.closed === 1); assert.equal(peer.stderr, '');
});

test('an unauthorized metadata stream fails closed without retries or credential-bearing diagnostics', async t => {
  const endpoint = await mcpHttpFixture(t, { eventStatus: 403 }), peer = bridge(t, endpoint); peer.send(init);
  assert.equal((await peer.completion).code, 2); assert.equal(endpoint.state.eventHeaders.length, 1); assert.equal(peer.stderr, '');
});

test('an imported real bridge contains stdin errors without printing exception or scope data', async t => {
  const endpoint = await mcpHttpFixture(t), peer = bridge(t, endpoint, fileURLToPath(new URL('./fixtures/mcp-stdin-error.mjs', import.meta.url)));
  assert.equal((await peer.completion).code, 2); assert.equal(peer.stderr, ''); assert.deepEqual(endpoint.state.posts, []);
});

test('an oversized unterminated SSE line is bounded and aborts the metadata connection', async t => {
  const endpoint = await mcpHttpFixture(t, { initialEvent: false }), peer = bridge(t, endpoint); peer.send(init);
  await waitFor(() => endpoint.state.opened === 1); endpoint.raw('data: ' + 'x'.repeat(65_537));
  assert.equal((await peer.completion).code, 2); await waitFor(() => endpoint.state.closed === 1); assert.equal(peer.messages.length, 1);
});

test('HTTP redirects are rejected instead of forwarding a scoped credential to another route', async t => {
  let endpoint;
  endpoint = await mcpHttpFixture(t, { onPost(_input, _complete, response) { response.writeHead(302, { Location: `${endpoint.origin}/must-not-follow` }).end(); } });
  const peer = bridge(t, endpoint); peer.send({ id: 1, method: 'tools/list', params: {} }); await waitFor(() => peer.messages.length === 1);
  assert.equal(peer.messages[0].error.code, -32000); assert.deepEqual(endpoint.state.urls, [`/mcp/${endpoint.conversation}`]);
});

test('production Host app grants/state and next-turn hooks reach a retained bridge between turns while tools stay active-run scoped', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-live-mcp-notify-')); let host;
  const runtime = new GrokAcpRuntime({ cwd: directory, command: process.execPath, args: [fileURLToPath(new URL('./fixtures/mcp-wait-runtime.mjs', import.meta.url))], onEvent: event => host?.runtimeEvent(event) });
  host = new AgentHost({ dataDir: directory, runtime }); const web = await serveHost(host);
  t.after(async () => { await host.close(); await web.close(); await rm(directory, { recursive: true, force: true }); });
  const first = host.store.createProject('Synthetic notification project'), other = host.store.createProject('Other synthetic project'), conversation = host.store.createConversation(first.id);
  async function post(path, value) {
    const response = await fetch(web.origin + path, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify(value) });
    assert.equal(response.status, 200, await response.clone().text()); return response.json();
  }
  await host.send(conversation.id, { projectId: first.id, text: 'Synthetic initial turn' });
  await waitFor(() => runtime.status.activeTurns === 1);
  const scopeToken = [...host.mcpScopes].find(([, scope]) => scope.conversationId === conversation.id)[0];
  const peer = bridge(t, { origin: web.origin, token: scopeToken, conversation: conversation.id });
  peer.send(init); await waitFor(() => peer.messages.some(message => message.method === 'notifications/tools/list_changed'));
  const notifications = () => peer.messages.filter(message => message.method === 'notifications/tools/list_changed').length;
  const run = host.runs.get(conversation.id); await host.cancel(conversation.id); await run.completion;
  assert.equal(host.runs.size, 0);
  const sourceDirectory = fileURLToPath(new URL('../../../examples/echo-app/', import.meta.url));
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const packaged = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(directory, 'package') });
  let count = notifications(); await post('/api/apps', { path: packaged.directory }); await waitFor(() => notifications() === count + 1);
  count = notifications(); await post('/api/apps/org.oviraptor.echo/state', { enabled: true }); await waitFor(() => notifications() === count + 1);
  count = notifications(); await post('/api/apps/org.oviraptor.echo/grants', { projectId: first.id, permissions: ['echo.write'], disclosureAccepted: true, expectedRuntimeIdentity: host.runtimeIdentity() }); await waitFor(() => notifications() === count + 1);
  peer.send({ id: 2, method: 'tools/list' }); await waitFor(() => peer.messages.some(message => message.id === 2)); assert.equal(peer.messages.find(message => message.id === 2).error.code, -32000, 'Idle metadata subscription does not admit tools');
  count = notifications(); await post('/api/apps/org.oviraptor.echo/grants', { projectId: other.id, permissions: ['echo.read'], disclosureAccepted: true, expectedRuntimeIdentity: host.runtimeIdentity() });
  // A subsequent same-project event is a wire-order barrier on the same SSE stream.
  await post('/api/apps/org.oviraptor.echo/grants', { projectId: first.id, permissions: ['echo.write'], disclosureAccepted: true, expectedRuntimeIdentity: host.runtimeIdentity() }); await waitFor(() => notifications() >= count + 1);
  await new Promise(resolve => setTimeout(resolve, 20)); assert.equal(notifications(), count + 1, 'Foreign project grants do not notify this scope');
  count = notifications(); await host.send(conversation.id, { projectId: first.id, text: 'Synthetic next turn' }); await waitFor(() => notifications() === count + 1 && runtime.status.activeTurns === 1);
  peer.send({ id: 3, method: 'tools/list' }); await waitFor(() => peer.messages.some(message => message.id === 3)); assert(peer.messages.find(message => message.id === 3).result.tools.some(tool => tool.name.endsWith('__echo')));
  count = notifications(); await post('/api/apps/org.oviraptor.echo/grants', { projectId: first.id, permissions: [], disclosureAccepted: true, expectedRuntimeIdentity: host.runtimeIdentity() }); await waitFor(() => notifications() === count + 1);
  peer.send({ id: 4, method: 'tools/list' }); await waitFor(() => peer.messages.some(message => message.id === 4)); assert.deepEqual(peer.messages.find(message => message.id === 4).result.tools, []);
  count = notifications(); await post('/api/apps/org.oviraptor.echo/state', { enabled: false }); await waitFor(() => notifications() === count + 1);
  peer.child.stdin.end(); assert.equal((await peer.completion).code, 0);
});
