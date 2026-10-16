import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { buildDiscoverySummary } from '../src/discovery.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));

async function hostFixture(t, options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-coupling-'));
  const host = new AgentHost({ dataDir: join(directory, 'host'), ...options });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Coupling project');
  return { host, directory, project };
}

async function installExample(host, directory, project, name, permissions, runtimeIdentity) {
  const sourceDirectory = join(root, 'examples', name);
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const path = (await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(directory, `${name}-package`) })).directory;
  const installed = await host.apps.install(path);
  await host.apps.setEnabled(installed.id, true);
  if (permissions?.length) host.apps.setGrants(installed.id, project.id, permissions, undefined, runtimeIdentity);
  return installed;
}

function mcpScope(host, conversation, pid = 55) {
  const run = { id: 'run-coupling', conversationId: conversation.id, sessionId: 'coupling-session', state: 'running', text: '', cancelled: false, skillScope: null };
  host.runs.set(conversation.id, run);
  const token = 'f'.repeat(64);
  host.mcpScopes.set(token, { conversationId: conversation.id, runtimePid: pid });
  return token;
}

const runtime = {
  evidence: 'synthetic-or-custom', status: { pid: 55 },
  async start() {}, async newSession() { return { sessionId: 'coupling-session' }; }, async loadSession() {},
  async prompt() { return { stopReason: 'end_turn', metrics: {} }; }, async cancel() { return true; }, async close() {},
};

test('R1/R2: ungated permission stays out of tools/list and direct call is 403 without retry', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const installed = await installExample(host, directory, project, 'fixture-authority', ['fixture.read'], host.runtimeIdentity());
  const conversation = host.store.createConversation(project.id);
  const token = mcpScope(host, conversation);
  const listed = await host.mcp(token, conversation.id, 'tools/list');
  assert.deepEqual(listed.tools.map(tool => tool.name).sort(), [`${installed.id}__status`]);
  assert.ok(listed.tools.every(tool => typeof tool.description === 'string' && tool.description.length > 0));
  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__ping`, arguments: { note: 'x' } }), error => {
    assert.equal(error.code, 'host_tool_permission_denied');
    assert.equal(error.status, 403);
    assert.equal(error.retryable, false);
    return true;
  });
});

test('R3: grantRevision change surfaces host_tool_authority_changed and must not be retried', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const installed = await installExample(host, directory, project, 'fixture-authority', ['fixture.write'], host.runtimeIdentity());
  const conversation = host.store.createConversation(project.id);
  const token = mcpScope(host, conversation);
  const appsCall = host.apps.call.bind(host.apps);
  let calls = 0;
  host.apps.call = async (...args) => {
    calls += 1;
    const pending = appsCall(...args);
    host.apps.setGrants(installed.id, project.id, [], undefined, host.runtimeIdentity());
    return pending;
  };
  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${installed.id}__ping`, arguments: { note: 'mid' } }), error => {
    assert.equal(error.code, 'host_tool_authority_changed');
    assert.equal(error.retryable, false);
    return true;
  });
  assert.equal(calls, 1);
  host.apps.call = appsCall;
  // Client must refresh: tools/list is now empty for write tools
  const listed = await host.mcp(token, conversation.id, 'tools/list');
  assert.deepEqual(listed.tools.filter(tool => tool.name.startsWith(installed.id)), []);
});

test('R4: disabled app disappears from tools/list and discovery', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const installed = await installExample(host, directory, project, 'fixture-authority', ['fixture.read'], host.runtimeIdentity());
  const conversation = host.store.createConversation(project.id);
  const token = mcpScope(host, conversation);
  await host.apps.setEnabled(installed.id, false);
  const listed = await host.mcp(token, conversation.id, 'tools/list');
  assert.deepEqual(listed.tools, []);
  const discovery = buildDiscoverySummary({ apps: host.apps, projectId: project.id, runtimeIdentity: host.runtimeIdentity() });
  assert.equal(discovery.summary.apps.some(app => app.id === installed.id), false);
});

test('R5: builtin uninstallable constraint remains (mark + reject)', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const installed = await installExample(host, directory, project, 'echo-app', ['echo.read'], host.runtimeIdentity());
  host.store.db.prepare('UPDATE applications SET builtin=1,uninstallable=0 WHERE id=?').run(installed.id);
  await assert.rejects(host.apps.uninstall(installed.id), error => error.code === 'host_app_builtin');
  assert.equal(host.store.application(installed.id).builtin, true);
  assert.equal(host.store.application(installed.id).uninstallable, false);
});

test('R6: discovery only exposes installed, enabled, and authorized apps', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const a = await installExample(host, directory, project, 'fixture-authority', ['fixture.read'], host.runtimeIdentity());
  const b = await installExample(host, directory, project, 'echo-app', [], host.runtimeIdentity());
  await host.apps.setEnabled(b.id, false);
  const discovery = buildDiscoverySummary({ apps: host.apps, projectId: project.id, runtimeIdentity: host.runtimeIdentity() });
  const ids = discovery.summary.apps.map(app => app.id);
  assert.deepEqual(ids, [a.id]);
  assert.ok(discovery.summary.apps[0].tools.every(tool => tool.purpose));
});

test('R7: app A cannot invoke app B tools except via host apps.call with B grant', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const a = await installExample(host, directory, project, 'fixture-authority', ['fixture.read', 'fixture.write'], host.runtimeIdentity());
  const b = await installExample(host, directory, project, 'echo-app', ['echo.read'], host.runtimeIdentity());
  // Wrong tool name on A
  await assert.rejects(host.apps.call(a.id, 'history', project.id, {}), /host_tool_permission_denied/);
  // MCP name for B without listing through A namespace
  const conversation = host.store.createConversation(project.id);
  const token = mcpScope(host, conversation);
  const listed = await host.mcp(token, conversation.id, 'tools/list');
  assert.ok(listed.tools.some(tool => tool.name === `${a.id}__status`));
  assert.ok(listed.tools.some(tool => tool.name === `${b.id}__history`));
  // Revoke B only — A still works, B denied
  host.apps.setGrants(b.id, project.id, [], undefined, host.runtimeIdentity());
  await assert.rejects(host.mcp(token, conversation.id, 'tools/call', { name: `${b.id}__history`, arguments: {} }), error => {
    assert.equal(error.code, 'host_tool_permission_denied');
    assert.equal(error.retryable, false);
    return true;
  });
  const ok = await host.mcp(token, conversation.id, 'tools/call', { name: `${a.id}__status`, arguments: {} });
  assert.equal(typeof ok.structuredContent.total, 'number');
  assert.equal(ok.structuredContent.projectId, project.id);
});

test('R8: disclosure_runtime mismatch hides capabilities until grant matches runtimeIdentity', async t => {
  const { host, directory, project } = await hostFixture(t, { runtime });
  const installed = await installExample(host, directory, project, 'fixture-authority', ['fixture.read'], 'stale-runtime:0');
  const conversation = host.store.createConversation(project.id);
  const token = mcpScope(host, conversation);
  const listed = await host.mcp(token, conversation.id, 'tools/list');
  assert.deepEqual(listed.tools.filter(tool => tool.name.startsWith(installed.id)), []);
  host.apps.setGrants(installed.id, project.id, ['fixture.read'], undefined, host.runtimeIdentity());
  const restored = await host.mcp(token, conversation.id, 'tools/list');
  assert.ok(restored.tools.some(tool => tool.name === `${installed.id}__status`));
});
