import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { AppManager } from '../src/apps.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';
import { AppProcess } from '../src/app-process.mjs';

const appId = 'org.example.review';
const content = { content: [{ type: 'text', text: 'Synthetic authorized result' }] };
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };

async function inFlightMcp(t) {
  const store = new HostStore(':memory:');
  t.after(() => store.close());
  const project = store.createProject('Synthetic review project');
  const conversation = store.createConversation(project.id);
  const entered = deferred(), result = deferred();
  const apps = {
    capabilities: () => [{ appId, name: `${appId}::read`, permission: 'read', grantRevision: 1 }],
    async call() { entered.resolve(); return result.promise; },
  };
  const runtime = { status: { pid: 101 }, async cancel() {} };
  const host = new AgentHost({ dataDir: '/tmp/unused-agent-host-review', store, apps, runtime });
  const task = store.createTask(conversation.id, 'Read synthetic authorized content');
  const run = { id: 'synthetic-run', taskId: task.id, conversationId: conversation.id, sessionId: 'synthetic-session', state: 'running' };
  const scope = { conversationId: conversation.id, runtimePid: 101, runId: run.id };
  host.runs.set(conversation.id, run);
  host.mcpScopes.set('synthetic-token', scope);
  const pending = host.mcp('synthetic-token', conversation.id, 'tools/call', { name: `${appId}__read`, arguments: {} });
  // Attach immediately so rejection remains observed before changing authority.
  const outcome = pending.then(value => ({ value }), error => ({ error }));
  await entered.promise;
  return { host, runtime, run, scope, conversation, result, outcome };
}

for (const [name, change] of [
  ['cancel', f => f.host.cancel(f.conversation.id)],
  ['run identity replacement', f => f.host.runs.set(f.conversation.id, { ...f.run, id: 'next-run' })],
  ['runtime process replacement', f => { f.runtime.status.pid = 202; }],
  ['MCP scope identity replacement', f => f.host.mcpScopes.set('synthetic-token', { ...f.scope })],
  ['run removal', f => f.host.runs.delete(f.conversation.id)],
]) {
  test(`MCP withholds an already executing read after ${name}`, async t => {
    const f = await inFlightMcp(t);
    await change(f);
    f.result.resolve(content);
    const outcome = await f.outcome;
    assert.equal(outcome.value, undefined);
    assert.equal(outcome.error?.code, 'host_mcp_scope_unavailable');
  });
}

test('MCP returns an executing read when its captured authority remains unchanged', async t => {
  const f = await inFlightMcp(t);
  f.result.resolve(content);
  const outcome = await f.outcome;
  assert.equal(outcome.error, undefined);
  const task = f.host.store.task(f.conversation.id, f.run.taskId);
  assert.equal(task.evidence.length, 1);
  const evidence = task.evidence[0];
  assert.equal(evidence.runId, f.run.id);
  assert.equal(evidence.toolName, `${appId}__read`);
  assert.equal(evidence.status, 'completed');
  assert.equal(evidence.readOnly, true);
  assert.deepEqual(outcome.value, { ...content, _meta: { hostEvidenceId: evidence.id } });
});

function processFixture(t, toolReply) {
  const app = { id: appId, version: '1.0.0', path: '/tmp/synthetic-reviewed-package', manifest: { entry: 'worker.mjs', tools: [], pages: [], dataSchemaVersion: 1 } };
  const child = new EventEmitter();
  child.stdout = new PassThrough(); child.stderr = new PassThrough(); child.stdin = new EventEmitter();
  child.exited = false;
  child.kill = () => {
    if (child.exited) return false;
    child.exited = true; queueMicrotask(() => child.emit('exit', 0, null)); return true;
  };
  child.stdin.write = (line, callback) => {
    const request = JSON.parse(line);
    let response;
    if (request.method === 'tool/call') response = toolReply(request);
    else {
      const result = request.method === 'describe'
        ? { protocolVersion: 1, appId, version: app.version, tools: [], pages: [] }
        : request.method === 'lifecycle/start' ? { dataSchemaVersion: 1 } : { busy: false };
      response = { jsonrpc: '2.0', id: request.id, result };
    }
    queueMicrotask(() => child.stdout.write(JSON.stringify(response) + '\n'));
    callback?.(); return true;
  };
  const worker = new AppProcess(app, '/tmp/synthetic-reviewed-data', { spawn: () => child });
  t.after(() => worker.stop());
  return worker;
}

for (const [name, result] of [
  ['missing content', { arbitrary: 'not MCP content' }],
  ['null result', null],
  ['array result', []],
  ['nonarray content', { content: {} }],
  ['unknown result key', { ...content, injectedAuthority: true }],
  ['unsupported content kind', { content: [{ type: 'image', text: 'synthetic' }] }],
  ['nonstring text', { content: [{ type: 'text', text: 7 }] }],
  ['unknown text block key', { content: [{ type: 'text', text: 'synthetic', permission: 'granted' }] }],
  ['nonboolean isError', { ...content, isError: 'false' }],
  ['array structured content', { ...content, structuredContent: [] }],
  ['null structured content', { ...content, structuredContent: null }],
  ['more than32 text blocks', { content: Array.from({ length: 33 }, () => ({ type: 'text', text: '' })) }],
]) {
  test(`Host rejects worker result with ${name} independently of SDK worker validation`, async t => {
    const worker = processFixture(t, request => ({ jsonrpc: '2.0', id: request.id, result }));
    await assert.rejects(worker.call('read', {}, {}), error => error.code === 'host_app_result_invalid');
  });
}

test('Host preserves valid bounded MCP text, structured content and error indication', async t => {
  const result = { ...content, structuredContent: { exact: 3, unknown: null }, isError: false };
  const worker = processFixture(t, request => ({ jsonrpc: '2.0', id: request.id, result }));
  assert.deepEqual(await worker.call('read', {}, {}), result);
});

test('Host rejects actual serialized worker output beyond64KiB', async t => {
  const worker = processFixture(t, request => ({ jsonrpc: '2.0', id: request.id, result: { content: [{ type: 'text', text: '界'.repeat(23000) }] } }));
  await assert.rejects(worker.call('read', {}, {}), error => error.code === 'host_app_result_too_large');
});

for (const [name, fields] of [
  ['both result and error', { result: content, error: { code: -32603, message: 'synthetic failure' } }],
  ['neither result nor error', {}],
  ['unknown response field', { result: content, extra: true }],
]) {
  test(`Host rejects JSON-RPC response with ${name}`, async t => {
    const worker = processFixture(t, request => ({ jsonrpc: '2.0', id: request.id, ...fields }));
    await assert.rejects(worker.call('read', {}, {}), error => error.code === 'host_app_protocol_invalid');
  });
}

for (const [name, error] of [
  ['null error', null],
  ['false error', false],
  ['malformed error object', { code: 'invalid', message: 7 }],
]) {
  test(`Host rejects JSON-RPC response with ${name} as a protocol error`, async t => {
    const worker = processFixture(t, request => ({ jsonrpc: '2.0', id: request.id, error }));
    await assert.rejects(worker.call('read', {}, {}), cause => cause.code === 'host_app_protocol_invalid');
  });
}

test('Host contains a valid worker JSON-RPC error without exposing its raw message', async t => {
  const worker = processFixture(t, request => ({ jsonrpc: '2.0', id: request.id, error: { code: -32603, message: 'Synthetic private worker diagnostic', data: { code: 'synthetic_detail' } } }));
  await assert.rejects(worker.call('read', {}, {}), cause => cause.code === 'host_app_tool_rejected' && !cause.message.includes('private'));
});


test('AppManager close prevents a validated package install that was still pending on directory reads', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-review-close-install-'));
  const store = new HostStore(':memory:');
  const apps = new AppManager(store, join(directory, 'applications'));
  t.after(async () => { await apps.close(); store.close(); await rm(directory, { recursive: true, force: true }); });
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const sourceDirectory = join(root, 'examples/echo-app');
  const specification = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const source = (await createDirectoryPackage({ ...specification, sourceDirectory, outputDirectory: join(directory, 'package') })).directory;
  const install = apps.install(source);
  const outcome = install.then(value => ({ value }), error => ({ error }));
  await apps.close();
  const actual = await outcome;
  assert.equal(actual.value, undefined);
  assert.equal(actual.error?.code, 'host_app_manager_closing');
  assert.deepEqual(store.applications(), []);
});
