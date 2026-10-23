import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';

const peer = fileURLToPath(new URL('./fixtures/skills-host-acp-peer.mjs', import.meta.url));
const instruction = name => `---\nname: ${name}\ndescription: Synthetic integration reference\n---\n${name.toUpperCase()}-GUIDANCE-PRIVATE-TO-THIS-CONVERSATION\n`;
const reference = 'a'.repeat(4095) + '中😀complete UTF-8 reference';
const sha256 = text => createHash('sha256').update(text).digest('hex');
async function until(predicate, description) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() > deadline) throw Error(`Synthetic integration deadline: ${description}`);
    await new Promise(resolve => setTimeout(resolve, 5));
  }
}
function unpack(result) {
  assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 32768);
  assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
  assert.equal(result.structuredContent.untrusted, true);
  assert.equal(result.structuredContent.authority, 'reference_only_no_execution');
  return result.structuredContent;
}
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-skills-host-chain-')), events = [], runtimeEvents = []; let host;
  const runtime = new GrokAcpRuntime({ cwd: directory, command: process.execPath, args: [peer],
    requestTimeoutMs: 5000, promptTimeoutMs: 7000, cancelTimeoutMs: 1500,
    onEvent(event) { runtimeEvents.push(event); host?.runtimeEvent(event); } });
  host = new AgentHost({ dataDir: join(directory, 'host'), runtime, onEvent: event => events.push(event) });
  const web = await serveHost(host);
  t.after(async () => { await host.close(); await web.close(); await rm(directory, { recursive: true, force: true }); });
  async function api(path, input, method = input === undefined ? 'GET' : 'POST') {
    const response = await fetch(web.origin + path, { method, headers: { 'X-Oviraptor-Session': web.token, Origin: web.origin, 'Content-Type': 'application/json' },
      ...(input === undefined ? {} : { body: JSON.stringify(input) }) });
    const value = await response.json(); assert.equal(response.status, 200, `${path}: ${value.error ?? response.status}`); return value;
  }
  const project = await api('/api/projects', { name: 'Synthetic selected-Skills project' });
  const otherProject = await api('/api/projects', { name: 'Synthetic unrelated project' });
  const source = join(directory, 'source');
  const put = async (path, bytes) => { const full = join(source, path); await mkdir(dirname(full), { recursive: true }); await writeFile(full, bytes); };
  await put('skills/alpha/SKILL.md', instruction('alpha'));
  await put('skills/beta/SKILL.md', instruction('beta'));
  await put('skills/alpha/references/guide.txt', reference);
  await put('skills/alpha/scripts/inert.mjs', 'throw new Error("A Skill script must never execute");\n');
  await put('skills/beta/references/other.txt', 'Unselected sibling reference');
  await put('LICENSE', 'Synthetic shared license');
  const preview = await api('/api/skills/preview', { path: source });
  const pkg = await api('/api/skills/import', { path: source, expectedDigest: preview.digest });
  const alpha = pkg.skills.find(skill => skill.name === 'alpha'), beta = pkg.skills.find(skill => skill.name === 'beta');
  async function conversation(projectId = project.id) { return api('/api/conversations', { projectId, title: 'Synthetic Skills integration' }); }
  async function selection(id, ids) {
    const shown = await api(`/api/conversations/${id}/skills`);
    return api(`/api/conversations/${id}/skills`, { selectedSkillIds: ids, expectedRevision: shown.revision, disclosureAccepted: true,
      expectedRuntimeIdentity: shown.runtimeIdentity, expectedSkillRevision: shown.skillRevision });
  }
  async function start(conversation, action) {
    return api(`/api/conversations/${conversation.id}/messages`, { projectId: conversation.projectId, text: 'SKILLS-HOST-INTEGRATION:' + JSON.stringify(action) });
  }
  async function turn(conversation, action) {
    await start(conversation, action);
    const run = host.runs.get(conversation.id); if (run) await run.completion;
    await until(() => !host.runs.has(conversation.id), 'turn completion');
    const message = host.store.messages(conversation.id).filter(message => message.role === 'assistant').at(-1);
    assert.equal(message.status, 'completed', message.failureCode);
    return { output: JSON.parse(message.content), message };
  }
  return { host, runtime, web, directory, events, runtimeEvents, api, project, otherProject, pkg, alpha, beta, conversation, selection, start, turn };
}

test('real HTTP selection drives ACP→MCP bridge→Skills list/read/resource pages and retains the session without app authority', async t => {
  const f = await fixture(t), conversation = await f.conversation();
  const absent = await f.turn(conversation, { action: 'inspect', calls: [{ name: 'skills_read', arguments: { skillId: f.alpha.id } }] });
  assert.deepEqual(absent.output.tools.map(tool=>tool.name).sort(), ['host_task_criteria','host_task_read','host_task_update','host_task_verify','workspace_list','workspace_read','workspace_run','workspace_write']); assert.equal(absent.output.results[0].error.code, -32000);
  await f.selection(conversation.id, [f.alpha.id]);
  const { output } = await f.turn(conversation, { action: 'inspect', calls: [
    { name: 'skills_list', arguments: {} },
    { name: 'skills_read', arguments: { skillId: f.alpha.id } },
    { name: 'skills_list_resources', arguments: { skillId: f.alpha.id } },
    { name: 'skills_read_resource', arguments: { skillId: f.alpha.id, path: 'skills/alpha/references/guide.txt' } },
    { name: 'skills_read_resource', arguments: { skillId: f.alpha.id, path: 'skills/alpha/references/guide.txt', offset: 4095 } },
    { name: 'skills_read_resource', arguments: { skillId: f.alpha.id, path: 'skills/alpha/scripts/inert.mjs' } },
  ] });
  assert.deepEqual(output.tools.map(tool => tool.name).sort(), ['host_task_criteria', 'host_task_read', 'host_task_update', 'host_task_verify', 'skills_list', 'skills_list_resources', 'skills_read', 'skills_read_resource', 'workspace_list', 'workspace_read', 'workspace_run', 'workspace_write']);
  assert(output.tools.every(tool => tool.inputSchema.additionalProperties === false && !Object.hasOwn(tool.inputSchema.properties, 'projectId')));
  const [catalog, instructions, resources, first, second, script] = output.results.map(item => { assert.equal(item.error, undefined); return unpack(item.result); });
  assert.deepEqual(catalog.skills.map(skill => skill.id), [f.alpha.id]);
  assert.equal(instructions.content, instruction('alpha'));
  assert(resources.resources.some(resource => resource.kind === 'script'));
  assert(resources.resources.every(resource => !resource.path.startsWith('skills/beta/')));
  assert.equal(first.nextOffset, 4095); assert.equal(first.content + second.content, reference);
  assert.equal(second.hasMore, false); assert(script.content.includes('must never execute'));
  assert.deepEqual(f.host.apps.list(), []); assert.deepEqual(f.host.apps.capabilities(f.project.id), []);
  assert.equal(output.agentPid, absent.output.agentPid); assert.notEqual(output.bridgePid, absent.output.bridgePid); assert(output.historyCount > absent.output.historyCount);
  assert.equal(new Set([process.pid, output.agentPid, output.bridgePid]).size, 3);
  assert.equal(output.agentPid, f.runtime.status.pid);
  assert(f.events.some(event => event.type === 'runtime.tool_update' && event.update.status === 'completed'));
});

test('actual retained MCP bridges isolate selections between same-project and other-project conversations', async t => {
  const f = await fixture(t), selected = await f.conversation(), sameProject = await f.conversation(), other = await f.conversation(f.otherProject.id);
  await f.selection(selected.id, [f.alpha.id]);
  const accepted = await f.turn(selected, { action: 'inspect', calls: [{ name: 'skills_read', arguments: { skillId: f.alpha.id } }] });
  assert.equal(unpack(accepted.output.results[0].result).content, instruction('alpha'));
  const denied = await f.turn(sameProject, { action: 'inspect', calls: [{ name: 'skills_read', arguments: { skillId: f.alpha.id } }] });
  assert.deepEqual(denied.output.tools.map(tool=>tool.name).sort(), ['host_task_criteria','host_task_read','host_task_update','host_task_verify','workspace_list','workspace_read','workspace_run','workspace_write']); assert.equal(denied.output.results[0].error.code, -32000);
  await f.selection(other.id, [f.beta.id]);
  const scoped = await f.turn(other, { action: 'inspect', calls: [
    { name: 'skills_list', arguments: {} },
    { name: 'skills_read', arguments: { skillId: f.alpha.id } },
    { name: 'skills_read', arguments: { skillId: f.beta.id } },
  ] });
  assert.deepEqual(unpack(scoped.output.results[0].result).skills.map(skill => skill.id), [f.beta.id]);
  assert.equal(scoped.output.results[1].error.code, -32000);
  assert.equal(unpack(scoped.output.results[2].result).content, instruction('beta'));
  assert.notEqual(scoped.output.bridgePid, accepted.output.bridgePid);
});

test('MCP cannot turn Skill guidance into project overrides or undeclared application/directory tools', async t => {
  const f = await fixture(t), conversation = await f.conversation(); await f.selection(conversation.id, [f.alpha.id]);
  const { output } = await f.turn(conversation, { action: 'inspect', calls: [
    { name: 'skills_read', arguments: { skillId: f.alpha.id, projectId: f.otherProject.id } },
    { name: 'assets_get_stats', arguments: {} },
    { name: 'directory_read_file', arguments: { path: '/synthetic/outside' } },
    { name: 'skills_read_resource', arguments: { skillId: f.alpha.id, path: 'skills/beta/SKILL.md' } },
  ] });
  assert(output.results.every(item => item.error?.code === -32000 && item.result === undefined));
  assert.deepEqual(f.host.apps.list(), []);
  assert(!JSON.stringify(output).includes('BETA-GUIDANCE-PRIVATE-TO-THIS-CONVERSATION'));
});

test('HTTP in-flight revocation withholds the real peer final reply even after successful MCP content delivery', async t => {
  const f = await fixture(t), conversation = await f.conversation(); await f.selection(conversation.id, [f.alpha.id]);
  await f.start(conversation, { action: 'read-wait', calls: [{ name: 'skills_read', arguments: { skillId: f.alpha.id } }] });
  const run = f.host.runs.get(conversation.id); assert(run);
  await until(() => f.runtimeEvents.some(event => event.type === 'text_delta' && event.text.startsWith('SKILLS_READ_READY:')), 'real MCP read');
  const ready = JSON.parse(f.runtimeEvents.find(event => event.type === 'text_delta' && event.text.startsWith('SKILLS_READ_READY:')).text.slice('SKILLS_READ_READY:'.length));
  assert.equal(ready.contentSha256, sha256(instruction('alpha'))); assert.equal(ready.resultsReceived, 1);
  const shown = await f.api(`/api/conversations/${conversation.id}/skills`);
  const revoked = await f.api(`/api/conversations/${conversation.id}/skills`, { selectedSkillIds: shown.selectedSkillIds, expectedRevision: shown.revision, disclosureAccepted: false });
  assert.deepEqual(revoked.selectedSkillIds, [f.alpha.id]); assert.equal(revoked.disclosureAccepted, false);
  await run.completion;
  const attempted = f.runtimeEvents.find(event => event.type === 'text_delta' && event.text.includes('WITHHELD-SKILLS-FINAL'));
  assert(attempted, 'The actual peer must attempt its stale final answer for this negative test');
  assert.equal(JSON.parse(attempted.text).deniedAfterCancel, true);
  const message = f.host.store.messages(conversation.id).filter(message => message.role === 'assistant').at(-1);
  assert.equal(message.status, 'failed'); assert.equal(message.content, ''); assert.equal(message.failureCode, 'host_skill_access_changed');
  assert(f.events.some(event => event.type === 'message.completed' && event.conversationId === conversation.id && event.content === '' && event.status === 'failed'));
  assert(!f.events.some(event => event.type === 'message.delta' && event.delta.includes('WITHHELD-SKILLS-FINAL')));
  assert.equal(f.host.runs.has(conversation.id), false);
});

test('revocation during an actual host resource read withholds its bytes before the MCP peer receives a result', async t => {
  const f = await fixture(t), conversation = await f.conversation(); await f.selection(conversation.id, [f.alpha.id]);
  const read = f.host.library.readResource.bind(f.host.library); let changed = false;
  f.host.library.readResource = (...args) => {
    const result = read(...args);
    if (!changed && args[0] === f.alpha.id && args[1] === f.alpha.entryPath) { changed = true; f.host.library.setEnabled(f.alpha.id, false); }
    return result;
  };
  await f.start(conversation, { action: 'inspect', calls: [{ name: 'skills_read', arguments: { skillId: f.alpha.id } }] });
  const run = f.host.runs.get(conversation.id); if (run) await run.completion;
  await until(() => !f.host.runs.has(conversation.id), 'withheld read completion');
  assert.equal(changed, true);
  const wire = f.runtimeEvents.filter(event => event.type === 'text_delta' && event.text.startsWith('{')).map(event => JSON.parse(event.text));
  assert(wire.some(output => output.results?.[0]?.error?.code === -32000));
  assert(wire.every(output => output.results?.[0]?.result === undefined));
  assert(!JSON.stringify(wire).includes('ALPHA-GUIDANCE-PRIVATE-TO-THIS-CONVERSATION'));
  const message = f.host.store.messages(conversation.id).filter(message => message.role === 'assistant').at(-1);
  assert.equal(message.status, 'failed'); assert.equal(message.content, '');
});
