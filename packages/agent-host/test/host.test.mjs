import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
async function fixture(t, options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-host-test-'));
  const host = new AgentHost({ dataDir: join(directory, 'host'), ...options });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const p1 = host.store.createProject('Synthetic project one'), p2 = host.store.createProject('Synthetic project two');
  return { host, directory, p1, p2 };
}
async function echoPackage(directory) {
  const sourceDirectory = join(root, 'examples', 'echo-app');
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  return (await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(directory, 'echo-package') })).directory;
}

test('zero-app core owns only host tables and saves plain chat without a runtime', async t => {
  const { host, p1, p2 } = await fixture(t);
  assert.deepEqual(host.apps.list(), []);
  const conversation = host.store.createConversation(p1.id);
  const result = await host.send(conversation.id, { projectId: p1.id, text: 'Synthetic offline message' });
  assert.equal(result.savedOnly, true);
  assert.equal(host.store.messages(conversation.id, p1.id)[0].content, 'Synthetic offline message');
  assert.throws(() => host.store.messages(conversation.id, p2.id), /host_conversation_unavailable/);
  const tables = host.store.db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all().map(row => row.name);
  assert(!tables.some(name => /assets|sentinel|scanning|findings/.test(name)));
});

test('real package install, process, scope, disable, uninstall and retained data', async t => {
  const { host, directory, p1, p2 } = await fixture(t);
  const source = await echoPackage(directory); const id = 'org.oviraptor.echo';
  const installed = await host.apps.install(source);
  assert.equal(installed.enabled, false);
  assert.deepEqual(host.apps.capabilities(p1.id), []);
  await host.apps.setEnabled(id, true);
  assert.deepEqual(host.apps.capabilities(p1.id), []);
  host.apps.setGrants(id, p1.id, ['echo.write', 'echo.read']);
  host.apps.setGrants(id, p2.id, ['echo.read']);
  const result = await host.apps.call(id, 'echo', p1.id, { text: 'Synthetic stored value' }, 1, 'synthetic-operation');
  assert.equal(result.structuredContent.projectId, p1.id);
  assert.equal((await host.apps.call(id, 'history', p2.id, {})).structuredContent.total, 0);
  await assert.rejects(host.apps.call(id, 'echo', p2.id, { text: 'denied' }), /host_tool_permission_denied/);
  await assert.rejects(host.apps.call(id, 'echo', p1.id, { text: 'bad', projectId: p2.id }), /additional|schema|argument/i);
  await host.apps.setEnabled(id, false);
  assert.deepEqual(host.apps.capabilities(p1.id), []);
  await assert.rejects(host.apps.call(id, 'history', p1.id, {}), /host_tool_permission_denied/);
  await host.apps.setEnabled(id, true);
  assert.deepEqual(host.apps.capabilities(p1.id), []);
  assert.equal((await host.apps.uninstall(id)).dataRetained, true);
  const saved = JSON.parse(await readFile(join(directory, 'host', 'applications', 'data', id, 'echo-state.json'), 'utf8'));
  assert.equal(saved.records.length, 1);
  await host.apps.install(source); await host.apps.setEnabled(id, true);
  assert.deepEqual(host.apps.capabilities(p1.id), []);
  host.apps.setGrants(id, p1.id, ['echo.read']);
  assert.equal((await host.apps.call(id, 'history', p1.id, {})).structuredContent.total, 1);
});

test('foreign and opaque origins, absent token and implicit disclosure are refused', async t => {
  const { host, p1 } = await fixture(t);
  const web = await serveHost(host); t.after(() => web.close());
  const get = headers => fetch(`${web.origin}/api/projects`, { headers });
  assert.equal((await get({})).status, 403);
  assert.equal((await get({ 'X-Oviraptor-Session': web.token, Origin: 'null' })).status, 403);
  assert.equal((await get({ 'X-Oviraptor-Session': web.token, Origin: 'https://foreign.invalid' })).status, 403);
  assert.equal((await get({ 'X-Oviraptor-Session': web.token, Origin: web.origin })).status, 200);
  const result = await fetch(`${web.origin}/api/apps/org.oviraptor.echo/grants`, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify({ projectId: p1.id, permissions: ['echo.write'] }) });
  assert.equal(result.status, 403);
  assert.equal((await result.json()).error, 'host_runtime_disclosure_required');
});

test('installed package mutation is rejected before registration', async t => {
  const { host, directory } = await fixture(t); const source = await echoPackage(directory);
  await writeFile(join(source, 'page.html'), 'synthetic tamper');
  await assert.rejects(host.apps.install(source));
  assert.deepEqual(host.apps.list(), []);
});

test('actual ACP child streams two turns through one retained session', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-host-acp-'));
  const events = []; let host;
  const runtime = new GrokAcpRuntime({ cwd: directory, command: process.execPath, args: [join(root, 'packages/runtime-acp/tests/fixtures/agent.mjs')], onEvent: event => host?.runtimeEvent(event) });
  host = new AgentHost({ dataDir: directory, runtime, onEvent: event => events.push(event) });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Synthetic ACP project'), conversation = host.store.createConversation(project.id);
  await host.send(conversation.id, { projectId: project.id, text: 'First synthetic turn' });
  await host.runs.get(conversation.id).completion;
  const sessionId = host.store.conversation(conversation.id).sessionId;
  const pid = runtime.status.pid;
  await host.send(conversation.id, { projectId: project.id, text: 'Second synthetic turn' });
  await host.runs.get(conversation.id).completion;
  assert.equal(host.store.conversation(conversation.id).sessionId, sessionId);
  assert.equal(runtime.status.pid, pid);
  assert.equal(host.store.messages(conversation.id).filter(row => row.role === 'assistant' && row.status === 'completed').length, 2);
  assert(events.some(event => event.type === 'message.delta' && event.delta.includes('你好')));
  assert.equal(events.filter(event => event.type === 'message.completed').length, 2);
  const metrics = host.store.db.prepare('SELECT metrics FROM runtime_metrics').all().map(row => JSON.parse(row.metrics));
  assert.equal(metrics.length, 2); assert(metrics.every(value => value.totalMs >= 0 && value.firstTextMs >= 0));
});
