import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { buildDiscoverySummary, formatDiscoveryPromptBlock, toolsFingerprint } from '../src/discovery.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { validateManifest } from '../../app-sdk/src/manifest.mjs';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';
import { SkillLibrary } from '../../skills-library/src/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const bridgePath = fileURLToPath(new URL('../src/mcp-bridge.mjs', import.meta.url));
const waitFor = (predicate, label = 'condition', ms = 4000) => new Promise((resolve, reject) => {
  const deadline = Date.now() + ms;
  const check = () => { if (predicate()) return resolve(); if (Date.now() > deadline) return reject(Error(`Timed out waiting for ${label}`)); setTimeout(check, 10); };
  check();
});

async function hostFixture(t, options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-smartness-'));
  const host = new AgentHost({ dataDir: join(directory, 'host'), ...options });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Synthetic smartness project');
  return { host, directory, project };
}

async function packageFixture(directory, name = 'fixture-authority') {
  const sourceDirectory = join(root, 'examples', name);
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  return (await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(directory, `${name}-package`) })).directory;
}

async function installGranted(host, directory, project, permissions = ['fixture.write', 'fixture.read']) {
  const path = await packageFixture(directory);
  const installed = await host.apps.install(path);
  await host.apps.setEnabled(installed.id, true);
  host.apps.setGrants(installed.id, project.id, permissions, undefined, host.runtimeIdentity());
  return installed;
}

test('P0.1 discovery summary injects authorized apps tools and selected skills into the prompt', async t => {
  const prompts = [];
  const runtime = {
    evidence: 'synthetic-or-custom',
    status: { pid: 4242 },
    async start() {},
    async newSession() { return { sessionId: 'synthetic-session' }; },
    async loadSession() {},
    async prompt({ prompt }) { prompts.push(prompt); return { stopReason: 'end_turn', metrics: {} }; },
    async cancel() { return true; },
    async close() {},
    async resolvePermission() { return true; },
  };
  const { host, directory, project } = await hostFixture(t, { runtime });
  await installGranted(host, directory, project);
  const conversation = host.store.createConversation(project.id);
  const access = await host.ensureSkills();
  const skillDir = join(directory, 'skill-pkg');
  await mkdir(skillDir, { recursive: true });
  await writeFile(join(skillDir, 'SKILL.md'), '---\nname: fixture-guide\ndescription: Synthetic fixture guidance\n---\n# Fixture guide\nRead before calling ping.\n');
  const preview = await host.library.previewDirectory(skillDir);
  await host.library.importDirectory(skillDir, preview.digest);
  const guide = host.library.listSkills().find(skill => skill.name === 'fixture-guide');
  assert.ok(guide, 'fixture-guide skill imported');
  const current = access.get(conversation.id);
  access.set(conversation.id, {
    selectedSkillIds: [guide.id],
    expectedRevision: current.revision,
    disclosureAccepted: true,
    expectedRuntimeIdentity: host.runtimeIdentity(),
    expectedSkillRevision: current.skillRevision,
  });
  await host.send(conversation.id, { projectId: project.id, text: 'Synthetic discovery turn' });
  await waitFor(() => prompts.length === 1, 'prompt');
  assert.match(prompts[0], /Host capability summary \(literal untrusted data/);
  assert.match(prompts[0], /org\.oviraptor\.fixture\.authority/);
  assert.match(prompts[0], /fixture\.authority__ping|org\.oviraptor\.fixture\.authority__ping/);
  assert.match(prompts[0], /untrusted/);
  assert.match(prompts[0], /fixture-guide/);
  // Unauthorized tool of another permission should not appear after grant narrowing
  host.apps.setGrants('org.oviraptor.fixture.authority', project.id, ['fixture.read'], undefined, host.runtimeIdentity());
  const conversation2 = host.store.createConversation(project.id);
  await host.send(conversation2.id, { projectId: project.id, text: 'Synthetic narrowed turn' });
  await waitFor(() => prompts.length === 2, 'second prompt');
  assert.match(prompts[1], /__status/);
  assert.doesNotMatch(prompts[1], /__ping/);
});

test('P0.1 toolsFingerprint changes when capabilities change after list_changed', async t => {
  const { host, directory, project } = await hostFixture(t);
  await installGranted(host, directory, project, ['fixture.read']);
  const first = buildDiscoverySummary({ apps: host.apps, projectId: project.id, runtimeIdentity: host.runtimeIdentity() });
  host.apps.setGrants('org.oviraptor.fixture.authority', project.id, ['fixture.write', 'fixture.read'], undefined, host.runtimeIdentity());
  const second = buildDiscoverySummary({ apps: host.apps, projectId: project.id, runtimeIdentity: host.runtimeIdentity() });
  assert.notEqual(first.fingerprint, second.fingerprint);
  assert.match(formatDiscoveryPromptBlock(second), /Before calling/);
});

test('P0.2 mcp-bridge returns structured error codes and does not collapse to a single message', async t => {
  const { createServer } = await import('node:http');
  let posts = 0;
  const server = createServer((request, response) => {
    if (request.method === 'POST') {
      posts += 1;
      let body = '';
      request.on('data', chunk => { body += chunk; });
      request.on('end', () => {
        const input = JSON.parse(body || '{}');
        if (input.method === 'tools/call') {
          response.writeHead(403, { 'Content-Type': 'application/json' });
          response.end(JSON.stringify({ error: 'host_tool_permission_denied', retryable: false }));
          return;
        }
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ tools: [] }));
      });
      return;
    }
    response.writeHead(404).end();
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const token = 'a'.repeat(64);
  const conversation = 'synthetic-conversation';
  const child = spawn(process.execPath, [bridgePath], {
    env: { PATH: process.env.PATH ?? '', OVIRAPTOR_HOST_ORIGIN: origin, OVIRAPTOR_MCP_TOKEN: token, OVIRAPTOR_CONVERSATION: conversation },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  const messages = []; let bytes = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    bytes += chunk;
    for (;;) {
      const index = bytes.indexOf('\n'); if (index < 0) break;
      messages.push(JSON.parse(bytes.slice(0, index))); bytes = bytes.slice(index + 1);
    }
  });
  t.after(async () => { if (!child.killed) child.kill('SIGTERM'); await new Promise(resolve => child.once('exit', resolve)); });
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'x__y', arguments: {} } }) + '\n');
  await waitFor(() => messages.some(message => message.id === 1), 'bridge error');
  const error = messages.find(message => message.id === 1).error;
  assert.equal(error.message, 'host_tool_permission_denied');
  assert.equal(error.data.code, 'host_tool_permission_denied');
  assert.equal(error.data.retryable, false);
  assert.notEqual(error.message, 'Scoped application capability unavailable');
  assert.equal(posts, 1, 'permission denied must not retry');
});

test('P0.2 revision conflict is retryable at most twice then surfaces structured code', async t => {
  const { createServer } = await import('node:http');
  let posts = 0;
  const server = createServer((request, response) => {
    if (request.method === 'POST') {
      posts += 1;
      response.writeHead(409, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ error: 'host_grant_revision_conflict', retryable: true }));
      return;
    }
    response.writeHead(404).end();
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const child = spawn(process.execPath, [bridgePath], {
    env: { PATH: process.env.PATH ?? '', OVIRAPTOR_HOST_ORIGIN: origin, OVIRAPTOR_MCP_TOKEN: 'b'.repeat(64), OVIRAPTOR_CONVERSATION: 'rev-conflict' },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  const messages = []; let bytes = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    bytes += chunk;
    for (;;) {
      const index = bytes.indexOf('\n'); if (index < 0) break;
      messages.push(JSON.parse(bytes.slice(0, index))); bytes = bytes.slice(index + 1);
    }
  });
  t.after(async () => { if (!child.killed) child.kill('SIGTERM'); await new Promise(resolve => child.once('exit', resolve)); });
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 7, method: 'tools/call', params: { name: 'a__b', arguments: {} } }) + '\n');
  await waitFor(() => messages.some(message => message.id === 7), 'revision error', 5000);
  const error = messages.find(message => message.id === 7).error;
  assert.equal(error.data.code, 'host_grant_revision_conflict');
  assert.equal(posts, 3, 'initial attempt + 2 retries');
});

test('P0.2 AgentHost.mcp retries grant revision conflict and never retries permission denied', async t => {
  const { host, directory, project } = await hostFixture(t, {
    runtime: {
      evidence: 'synthetic-or-custom', status: { pid: 99 },
      async start() {}, async newSession() { return { sessionId: 's' }; }, async loadSession() {},
      async prompt() { return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {},
    },
  });
  const installed = await installGranted(host, directory, project);
  const conversation = host.store.createConversation(project.id);
  const run = { id: 'run', conversationId: conversation.id, sessionId: 's', state: 'running', text: '', cancelled: false, skillScope: null };
  host.runs.set(conversation.id, run);
  const token = 'c'.repeat(64);
  host.mcpScopes.set(token, { conversationId: conversation.id, runtimePid: 99 });
  host.setOrigin('http://127.0.0.1:9');
  host.apps.setGrants(installed.id, project.id, [], undefined, host.runtimeIdentity());
  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__ping`, arguments: { note: 'nope' } }), error => {
    assert.equal(error.code, 'host_tool_permission_denied');
    assert.equal(error.retryable, false);
    return true;
  });
  // Restore grant and verify success path works
  host.apps.setGrants(installed.id, project.id, ['fixture.write'], undefined, host.runtimeIdentity());
  const ok = await host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__ping`, arguments: { note: 'yes' } });
  assert.equal(ok.structuredContent.ok, true);
});

test('P0.3 paired_skills validated in manifest and appear as before-call hints', () => {
  const base = {
    schemaVersion: 1, id: 'org.oviraptor.fixture.paired', version: '1.0.0', hostApi: '^1.0.0', entry: 'worker.mjs', dataSchemaVersion: 1,
    pages: [{ id: 'home', title: 'Paired', path: 'page.html' }],
    permissions: [{ id: 'p.write', description: 'write' }],
    tools: [{ name: 'act', description: 'Act', inputSchema: { type: 'object', properties: {}, additionalProperties: false }, permission: 'p.write' }],
    files: [{ path: 'worker.mjs', sha256: 'a'.repeat(64) }, { path: 'page.html', sha256: 'b'.repeat(64) }],
    paired_skills: [{ name: 'fixture-guide', source: 'bundled', required: false }],
  };
  const validated = validateManifest(base);
  assert.equal(validated.paired_skills[0].name, 'fixture-guide');
  assert.throws(() => validateManifest({ ...base, paired_skills: [{ name: 'bad', source: 'bin/run.sh' }] }), { code: 'app_manifest_invalid' });
  assert.throws(() => validateManifest({ ...base, paired_skills: [{ name: 'bad', source: 'tools/hack.mjs' }] }), { code: 'app_manifest_invalid' });
});

test('P0.4 runtime tool_call/tool_update normalize to runtime.plan and runtime.step', async t => {
  const events = [];
  const { host, project } = await hostFixture(t, {
    runtime: {
      evidence: 'synthetic-or-custom', status: { pid: 7 },
      async start() {}, async newSession() { return { sessionId: 'plan-session' }; }, async loadSession() {},
      async prompt() { return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {},
    },
    onEvent: event => events.push(event),
  });
  const conversation = host.store.createConversation(project.id);
  const run = { id: 'run-plan', conversationId: conversation.id, sessionId: 'plan-session', state: 'running', text: '', message: { id: 'msg-1' } };
  host.runs.set(conversation.id, run);
  host.runtimeEvent({ type: 'tool_call', sessionId: 'plan-session', toolCallId: 't1', title: 'Step one', status: 'in_progress' });
  host.runtimeEvent({ type: 'tool_call', sessionId: 'plan-session', toolCallId: 't2', title: 'Step two', status: 'in_progress' });
  host.runtimeEvent({ type: 'tool_update', sessionId: 'plan-session', toolCallId: 't1', status: 'completed' });
  assert(events.some(event => event.type === 'runtime.plan' && event.steps.length >= 1));
  assert(events.some(event => event.type === 'runtime.step' && event.id === 't1' && event.status === 'completed'));
  assert(events.some(event => event.type === 'runtime.step' && event.id === 't2'));
});

test('P0.5 unauthorized and disabled tools stay out of tools/list; direct call is 403 without retry; revoke ends with authority_changed', async t => {
  let callCount = 0;
  const originalCall = null;
  const { host, directory, project } = await hostFixture(t, {
    runtime: {
      evidence: 'synthetic-or-custom', status: { pid: 55 },
      async start() {}, async newSession() { return { sessionId: 'auth-session' }; }, async loadSession() {},
      async prompt() { return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {},
    },
  });
  const installed = await installGranted(host, directory, project, ['fixture.read']);
  const conversation = host.store.createConversation(project.id);
  const run = { id: 'run-auth', conversationId: conversation.id, sessionId: 'auth-session', state: 'running', text: '', cancelled: false, skillScope: null };
  host.runs.set(conversation.id, run);
  const token = 'd'.repeat(64);
  host.mcpScopes.set(token, { conversationId: conversation.id, runtimePid: 55 });

  const listed = await host.mcp(token, conversation.id, 'tools/list');
  assert.deepEqual(listed.tools.map(tool => tool.name).sort(), [`${installed.id}__status`, 'workspace_list', 'workspace_read', 'workspace_write', 'workspace_run', 'host_task_read', 'host_task_update', 'host_task_criteria', 'host_task_verify'].sort());
  assert(!listed.tools.some(tool => tool.name.endsWith('__ping')));

  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__ping`, arguments: { note: 'denied' } }), error => {
    assert.equal(error.code, 'host_tool_permission_denied');
    assert.equal(error.status, 403);
    assert.equal(error.retryable, false);
    return true;
  });

  await host.apps.setEnabled(installed.id, false);
  const listedDisabled = await host.mcp(token, conversation.id, 'tools/list');
  assert.deepEqual(listedDisabled.tools.map(tool => tool.name), ['workspace_list', 'workspace_read', 'workspace_write', 'workspace_run', 'host_task_read', 'host_task_update', 'host_task_criteria', 'host_task_verify']);
  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__status`, arguments: {} }), /host_tool_permission_denied/);

  await host.apps.setEnabled(installed.id, true);
  host.apps.setGrants(installed.id, project.id, ['fixture.write'], undefined, host.runtimeIdentity());
  // Start a slow call then revoke mid-flight by wrapping apps.call
  const appsCall = host.apps.call.bind(host.apps);
  let releaseGate;
  const gate = new Promise(resolve => { releaseGate = resolve; });
  host.apps.call = async (...args) => {
    callCount += 1;
    // Revoke after the call has been admitted but before completion check
    const pending = appsCall(...args);
    host.apps.setGrants(installed.id, project.id, [], undefined, host.runtimeIdentity());
    releaseGate();
    return pending;
  };
  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__ping`, arguments: { note: 'mid' } }), error => {
    assert.equal(error.code, 'host_tool_authority_changed');
    assert.equal(error.retryable, false);
    return true;
  });
  await gate;
  assert.equal(callCount, 1, 'authority_changed path must not retry');
  host.apps.call = appsCall;
});

test('P0.5 HTTP MCP surface returns structured 403 without Scoped application capability unavailable', async t => {
  const { host, directory, project } = await hostFixture(t, {
    runtime: {
      evidence: 'synthetic-or-custom', status: { pid: 77 },
      async start() {}, async newSession() { return { sessionId: 'http-session' }; }, async loadSession() {},
      async prompt() { return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {},
    },
  });
  await installGranted(host, directory, project, ['fixture.read']);
  const conversation = host.store.createConversation(project.id);
  const run = { id: 'run-http', conversationId: conversation.id, sessionId: 'http-session', state: 'running', text: '', cancelled: false, skillScope: null };
  host.runs.set(conversation.id, run);
  const token = 'e'.repeat(64);
  host.mcpScopes.set(token, { conversationId: conversation.id, runtimePid: 77 });
  const web = await serveHost(host);
  t.after(() => web.close());
  const response = await fetch(`${web.origin}/mcp/${conversation.id}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Oviraptor-Mcp': token },
    body: JSON.stringify({ method: 'tools/call', params: { name: 'org.oviraptor.fixture.authority__ping', arguments: { note: 'x' } } }),
  });
  assert.equal(response.status, 403);
  const body = await response.json();
  assert.equal(body.error, 'host_tool_permission_denied');
  assert.notEqual(body.retryable, true);
  assert.notEqual(body.error, 'Scoped application capability unavailable');
});
