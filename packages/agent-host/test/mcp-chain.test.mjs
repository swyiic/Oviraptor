import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const agent = fileURLToPath(new URL('./fixtures/acp-mcp-agent.mjs', import.meta.url));
const appId = 'org.oviraptor.echo';
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-mcp-chain-')), events = [], agentProcesses = []; let host;
  const runtime = new GrokAcpRuntime({ cwd: directory, requestTimeoutMs: 4000, promptTimeoutMs: 5000,
    spawn(_command, _args, options) { const child = spawn(process.execPath, [agent], options); agentProcesses.push(child); return child; },
    onEvent: event => host?.runtimeEvent(event) });
  host = new AgentHost({ dataDir: directory, runtime, onEvent: event => events.push(event) });
  const web = await serveHost(host);
  t.after(async () => { await host.close(); await web.close(); await rm(directory, { recursive: true, force: true }); });
  const p1 = host.store.createProject('Synthetic MCP project one'), p2 = host.store.createProject('Synthetic MCP project two');
  const sourceDirectory = join(root, 'examples', 'echo-app'), spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const packaged = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(directory, 'package') });
  await host.apps.install(packaged.directory); await host.apps.setEnabled(appId, true);
  async function grant(projectId, permissions) {
    const response = await fetch(`${web.origin}/api/apps/${appId}/grants`, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify({ projectId, permissions, disclosureAccepted: true }) });
    assert.equal(response.status, 200); return response.json();
  }
  async function turn(conversation, action) {
    await host.send(conversation.id, { projectId: conversation.projectId, text: 'CHAIN:' + JSON.stringify(action) });
    await host.runs.get(conversation.id).completion;
    const message = host.store.messages(conversation.id).filter(message => message.role === 'assistant').at(-1);
    assert.equal(message.status, 'completed', JSON.stringify(events.filter(event => event.type === 'runtime.error')));
    return JSON.parse(message.content);
  }
  return { host, runtime, events, agentProcesses, p1, p2, grant, turn, directory };
}

test('actual ACP → MCP bridge → Echo worker chain admits only explicit project grants and reuses a live bridge on later turns', async t => {
  const { host, runtime, events, agentProcesses, p1, p2, grant, turn } = await fixture(t);
  const conversation = host.store.createConversation(p1.id);
  const empty = await turn(conversation, { action: 'list' }); assert.deepEqual(empty.tools, []);
  await grant(p1.id, ['echo.write', 'echo.read']);
  const first = await turn(conversation, { action: 'echo', text: 'Synthetic three-process value one' });
  assert(first.tools.some(name => name.endsWith('__echo'))); assert.equal(first.call.structuredContent.projectId, p1.id);
  assert.equal(first.call.structuredContent.text, 'Synthetic three-process value one');
  const second = await turn(conversation, { action: 'echo', text: 'Synthetic three-process value two' });
  assert.equal(second.agentPid, first.agentPid); assert.equal(second.bridgePid, first.bridgePid); assert.equal(second.agentPid, runtime.status.pid);
  assert.notEqual(second.bridgePid, second.agentPid); assert.notEqual(second.agentPid, process.pid); assert.equal(agentProcesses.length, 1);
  const workerPid = host.apps.workers.get(appId).child.pid;
  assert.equal(new Set([process.pid, second.agentPid, second.bridgePid, workerPid]).size, 4, 'ACP peer, MCP bridge and Echo worker are three distinct real child processes');
  assert.equal(host.mcpScopes.size, 1, 'The conversation-scoped MCP credential remains usable during the second turn');
  assert.equal(second.call.structuredContent.projectId, p1.id); assert(events.some(event => event.type === 'message.delta' && event.delta.includes('Synthetic three-process value two')));
  assert(events.some(event => event.type === 'runtime.tool_call')); assert(events.some(event => event.type === 'runtime.tool_update' && event.update.status === 'completed'));
  await grant(p2.id, ['echo.read']); const other = host.store.createConversation(p2.id);
  const scoped = await turn(other, { action: 'echo', text: 'Must not write to another project' });
  assert(!scoped.tools.some(name => name.endsWith('__echo'))); assert(scoped.tools.some(name => name.endsWith('__history'))); assert.equal(scoped.unavailable, true);
});

test('MCP rejects model-supplied projectId and never writes a foreign-project record', async t => {
  const { host, p1, p2, grant, turn, directory } = await fixture(t); await grant(p1.id, ['echo.write']);
  const conversation = host.store.createConversation(p1.id), output = await turn(conversation, { action: 'extra-project', projectId: p2.id, text: 'Must be rejected by the schema' });
  assert.equal(output.error.code, -32000); assert.equal(output.call, undefined);
  const state = JSON.parse(await readFile(join(directory, 'applications', 'data', appId, 'echo-state.json'), 'utf8'));
  assert.deepEqual(state.records, []);
});

test('revocation and application disable immediately remove listing and reject a remembered tool name over the existing bridge', async t => {
  const { host, p1, grant, turn, directory } = await fixture(t); await grant(p1.id, ['echo.write']);
  const conversation = host.store.createConversation(p1.id), before = await turn(conversation, { action: 'echo', text: 'Only accepted record' });
  await grant(p1.id, []); const revoked = await turn(conversation, { action: 'call-known', text: 'Revoked write' });
  assert.deepEqual(revoked.tools, []); assert.equal(revoked.error.code, -32000); assert.equal(revoked.bridgePid, before.bridgePid);
  await grant(p1.id, ['echo.write']); await host.apps.setEnabled(appId, false);
  const disabled = await turn(conversation, { action: 'call-known', text: 'Disabled write' });
  assert.deepEqual(disabled.tools, []); assert.equal(disabled.error.code, -32000); assert.equal(disabled.bridgePid, before.bridgePid);
  const state = JSON.parse(await readFile(join(directory, 'applications', 'data', appId, 'echo-state.json'), 'utf8'));
  assert.equal(state.records.length, 1); assert.equal(state.records[0].text, 'Only accepted record');
});

test('a cancelled run denies a new MCP operation from its real retained bridge', async t => {
  const { host, p1, grant, events } = await fixture(t); await grant(p1.id, ['echo.write']);
  const conversation = host.store.createConversation(p1.id);
  await host.send(conversation.id, { projectId: p1.id, text: 'CHAIN:' + JSON.stringify({ action: 'wait-cancel' }) });
  const run = host.runs.get(conversation.id);
  await new Promise((resolve, reject) => { const deadline = Date.now() + 3000; const check = () => { if (events.some(event => event.type === 'message.delta' && event.delta === 'chain-wait-ready\n')) return resolve(); if (Date.now() > deadline) return reject(Error('Synthetic chain did not become ready')); setTimeout(check, 5); }; check(); });
  await host.cancel(conversation.id); await run.completion;
  const message = host.store.messages(conversation.id).filter(message => message.role === 'assistant').at(-1);
  assert.equal(message.status, 'cancelled'); assert(message.content.includes('"cancelledMcpDenied":true'));
});
