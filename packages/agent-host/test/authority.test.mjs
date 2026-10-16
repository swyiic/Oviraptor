import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { request as httpRequest } from 'node:http';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { AppProcess } from '../src/app-process.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const appId = 'org.oviraptor.echo';
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };

async function fixture(t, behavior = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-authority-'));
  const store = new HostStore(join(directory, 'host.sqlite'));
  const workers = [];
  const processFactory = (app, data) => {
    const worker = {
      app, data, active: 0, stopped: false, calls: [], stops: 0,
      async start() { await behavior.start?.(worker); },
      async call(name, args, context) {
        worker.active++; worker.calls.push({ name, args, context });
        try { return await behavior.call?.(worker) ?? { content: [{ type: 'text', text: 'Synthetic result' }] }; }
        finally { worker.active--; }
      },
      async stop() { assert.equal(worker.active, 0); worker.stops++; worker.stopped = true; },
    };
    workers.push(worker); return worker;
  };
  let apps = new AppManager(store, join(directory, 'applications'), { processFactory });
  const p1 = store.createProject('Synthetic one'), p2 = store.createProject('Synthetic two');
  const sourceDirectory = join(root, 'examples/echo-app');
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const source = (await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(directory, 'package') })).directory;
  await apps.install(source);
  t.after(async () => { await apps.close(); store.close(); await rm(directory, { recursive: true, force: true }); });
  return {
    get apps() { return apps; }, store, directory, source, p1, p2, workers, processFactory,
    async restart() {
      await apps.close();
      apps = new AppManager(store, join(directory, 'applications'), { processFactory });
      workers.length = 0;
    },
  };
}

test('project grants, declared schemas and grant revisions gate arguments before a worker call', async t => {
  const f = await fixture(t);
  await f.apps.setEnabled(appId, true);
  await assert.rejects(f.apps.call(appId, 'history', f.p1.id, {}), /host_tool_permission_denied/);
  f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  await assert.rejects(f.apps.call(appId, 'history', f.p2.id, {}), /host_tool_permission_denied/);
  await assert.rejects(f.apps.call(appId, 'history', f.p1.id, {}, 0), /host_grant_revision_conflict/);
  await assert.rejects(f.apps.call(appId, 'history', f.p1.id, { projectId: f.p2.id }), /additional|argument|schema/i);
  assert.equal(f.workers[0].calls.length, 0);
  await f.apps.call(appId, 'history', f.p1.id, {}, 1);
  assert.equal(f.workers[0].calls[0].context.hostBoundProjectId, f.p1.id);
});

test('revocation during an accepted call withholds its result and prevents later admission', async t => {
  const entered = deferred(), result = deferred();
  const f = await fixture(t, { call() { entered.resolve(); return result.promise; } });
  await f.apps.setEnabled(appId, true); f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  const pending = f.apps.call(appId, 'history', f.p1.id, {});
  const rejected = assert.rejects(pending, /host_tool_authority_changed/);
  await entered.promise;
  f.apps.setGrants(appId, f.p1.id, [], 1);
  assert.deepEqual(f.apps.capabilities(f.p1.id), []);
  await assert.rejects(f.apps.call(appId, 'history', f.p1.id, {}), /host_tool_permission_denied/);
  result.resolve({ content: [{ type: 'text', text: 'Withheld synthetic result' }] });
  await rejected;
  assert.equal(f.workers[0].calls.length, 1);
});

test('disable while busy revokes new calls, rejects uninstall and retains app data after a later uninstall', async t => {
  const entered = deferred(), result = deferred();
  const f = await fixture(t, { call() { entered.resolve(); return result.promise; } });
  await f.apps.setEnabled(appId, true); f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  const data = f.workers[0].data;
  await mkdir(data, { recursive: true }); await writeFile(join(data, 'synthetic.json'), '{"retained":true}');
  const rejected = assert.rejects(f.apps.call(appId, 'history', f.p1.id, {}), /host_tool_authority_changed/);
  await entered.promise;
  await f.apps.setEnabled(appId, false);
  assert.equal(f.workers[0].stops, 0);
  assert.deepEqual(f.store.grant(appId, f.p1.id).permissions, []);
  await assert.rejects(f.apps.uninstall(appId), /host_app_busy/);
  result.resolve({ content: [] }); await rejected;
  assert.equal((await f.apps.uninstall(appId)).dataRetained, true);
  assert.equal(await readFile(join(data, 'synthetic.json'), 'utf8'), '{"retained":true}');
  await f.apps.install(f.source);
  assert.deepEqual(f.store.grant(appId, f.p1.id).permissions, []);
  assert.equal(f.store.application(appId).enabled, false);
});

test('installed executable tampering is rejected before any worker is started', async t => {
  const f = await fixture(t);
  const app = f.store.application(appId);
  await writeFile(join(app.path, 'worker.mjs'), `${await readFile(join(app.path, 'worker.mjs'), 'utf8')}\n// synthetic post-install mutation\n`);
  await assert.rejects(f.apps.setEnabled(appId, true), /integrity|hash|package.*changed/i);
  assert.equal(f.workers.length, 0);
  assert.equal(f.store.application(appId).enabled, false);
});

test('concurrent first calls after host restart share exactly one app worker', async t => {
  const f = await fixture(t);
  await f.apps.setEnabled(appId, true); f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  await f.restart();
  await Promise.all([f.apps.call(appId, 'history', f.p1.id, {}), f.apps.call(appId, 'history', f.p1.id, {})]);
  assert.equal(f.workers.length, 1, 'A persisted enabled app must have one live worker, including concurrent cold starts');
  assert.equal(f.workers[0].calls.length, 2);
});

test('uninstall cannot delete an app while its first worker is still being created', async t => {
  const f = await fixture(t);
  await f.apps.setEnabled(appId, true); f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  await f.restart();
  const calling = f.apps.call(appId, 'history', f.p1.id, {});
  // Attach immediately so the pre-fix missing-app failure remains observed.
  const observed = calling.then(value => ({ value }), error => ({ error }));
  let uninstallError;
  try { await f.apps.uninstall(appId); } catch (error) { uninstallError = error; }
  const outcome = await observed;
  assert.match(uninstallError?.code ?? '', /host_app_busy/);
  assert.equal(f.store.application(appId).enabled, true);
  assert.equal(outcome.error, undefined);
  assert.equal(f.workers.length, 1);
});

test('host app close cannot finish while a cold worker creation is still pending', async t => {
  const f = await fixture(t);
  await f.apps.setEnabled(appId, true); f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  await f.restart();
  const calling = f.apps.call(appId, 'history', f.p1.id, {});
  const observed = calling.then(value => ({ value }), error => ({ error }));
  let closeError;
  try { await f.apps.close(); } catch (error) { closeError = error; }
  const outcome = await observed;
  assert.match(closeError?.code ?? '', /host_app_busy/);
  assert.equal(outcome.error, undefined);
});

test('MCP tokens retained by an older runtime process cannot access a current active turn', async t => {
  const f = await fixture(t);
  const runtime = { status: { pid: 222 } };
  const host = new AgentHost({ dataDir: f.directory, store: f.store, apps: f.apps, runtime });
  const conversation = f.store.createConversation(f.p1.id);
  host.runs.set(conversation.id, { id: 'synthetic-run', conversationId: conversation.id });
  host.mcpScopes.set('synthetic-old-token', { conversationId: conversation.id, runtimePid: 111 });
  await assert.rejects(host.mcp('synthetic-old-token', conversation.id, 'tools/list'), /host_mcp_scope_unavailable/);
});

test('a cancelled turn rejects new MCP tool admission before runtime cancellation finishes', async t => {
  const f = await fixture(t);
  await f.apps.setEnabled(appId, true); f.apps.setGrants(appId, f.p1.id, ['echo.read']);
  const runtime = { status: { pid: 222 } };
  const host = new AgentHost({ dataDir: f.directory, store: f.store, apps: f.apps, runtime });
  const conversation = f.store.createConversation(f.p1.id);
  host.runs.set(conversation.id, { id: 'synthetic-cancelled-run', conversationId: conversation.id, state: 'cancelling', cancelled: true });
  host.mcpScopes.set('synthetic-current-token', { conversationId: conversation.id, runtimePid: 222 });
  await assert.rejects(host.mcp('synthetic-current-token', conversation.id, 'tools/call', { name: `${appId}__history`, arguments: {} }), /host_mcp_scope_unavailable/);
  assert.equal(f.workers[0].calls.length, 0);
});

test('HTTP requires the exact origin and session token before a project write', async t => {
  const f = await fixture(t);
  const host = new AgentHost({ dataDir: f.directory, store: f.store, apps: f.apps });
  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  for (const patch of [{ 'X-Oviraptor-Session': 'synthetic-invalid' }, { Origin: 'null' }, { Origin: 'https://foreign.invalid' }]) {
    const response = await fetch(`${web.origin}/api/projects`, { method: 'POST', headers: { ...headers, ...patch }, body: JSON.stringify({ name: 'Must not exist' }) });
    assert.equal(response.status, 403);
  }
  // Node fetch normalizes Host; use the actual HTTP wire for this negative case.
  const foreignHostStatus = await new Promise((resolve, reject) => {
    const request = httpRequest(`${web.origin}/api/projects`, { headers: { ...headers, Host: 'foreign.invalid' } }, response => { response.resume(); resolve(response.statusCode); });
    request.on('error', reject); request.end();
  });
  assert.equal(foreignHostStatus, 403);
  assert.equal(f.store.projects().length, 2);
  const allowed = await fetch(`${web.origin}/api/projects`, { method: 'POST', headers: { ...headers, Origin: web.origin }, body: JSON.stringify({ name: 'Explicit local write' }) });
  assert.equal(allowed.status, 200);
  assert.equal(f.store.projects().length, 3);
});

test('a zero-application host database has exactly core tables and no application business schema', () => {
  const store = new HostStore(':memory:');
  try {
    const actual = store.db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all().map(row => row.name);
    assert.deepEqual(actual, ['application_grants', 'application_retained', 'application_versions', 'applications', 'conversations', 'host_settings', 'messages', 'projects', 'runtime_metrics']);
    assert.deepEqual(store.applications(), []);
  } finally { store.close(); }
});

function processFixture() {
  const app = { id: appId, version: '1.0.0', path: '/synthetic/package', manifest: { entry: 'worker.mjs', tools: [], pages: [], dataSchemaVersion: 1 } };
  const child = new EventEmitter(); child.stdout = new PassThrough(); child.stderr = new PassThrough(); child.stdin = new EventEmitter(); child.kills = 0;
  child.kill = () => { child.kills++; return true; };
  child.stdin.write = (line, callback) => {
    const request = JSON.parse(line);
    const result = request.method === 'describe' ? { protocolVersion: 1, appId: app.id, version: app.version, tools: [], pages: [] }
      : request.method === 'lifecycle/start' ? { dataSchemaVersion: 1 } : { busy: false };
    queueMicrotask(() => child.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\n'));
    callback?.(); return true;
  };
  return { process: new AppProcess(app, '/synthetic/data', { spawn: () => child }), child };
}

test('stopping an app does not report completion before its worker process exits', async t => {
  const { process, child } = processFixture();
  t.after(() => child.emit('exit', 0, null));
  await process.start();
  let finished = false;
  const stopping = process.stop().then(() => { finished = true; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(child.kills, 1);
  assert.equal(finished, false, 'Uninstall/reinstall must not race a still-live old process');
  child.emit('exit', 0, null); await stopping;
  assert.equal(finished, true);
});

test('an app stdin error is contained as worker failure instead of escaping the host event loop', async t => {
  const { process, child } = processFixture();
  t.after(() => child.emit('exit', 0, null));
  await process.start();
  assert.doesNotThrow(() => child.stdin.emit('error', new Error('synthetic closed pipe')));
  assert.equal(process.stopped, true);
  assert.equal(child.kills, 1);
});
