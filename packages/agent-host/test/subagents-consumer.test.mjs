import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, realpath, mkdir, writeFile, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { HostStore } from '../src/store.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { absorbToolResults, emptyAgentState } from '../src/agent-state.mjs';
import { compressSessionMessages, summarizeOlderTurns } from '../src/memory.mjs';

const response = delta => ({ ok: true, status: 200, body: (async function* () { yield `data: ${JSON.stringify({ choices: [{ delta, finish_reason: delta.tool_calls ? 'tool_calls' : 'stop' }] })}\n\n`; })() });
const calls = entries => ({ tool_calls: entries.map((entry, index) => ({ index, id: entry.id, function: { name: entry.name, arguments: JSON.stringify(entry.args ?? {}) } })) });
const unpack = request => { const message = request.messages.filter(item => item.role === 'tool').at(-1); return JSON.parse(JSON.parse(message.content).content[0].text); };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { resolve, promise }; };
async function makeHost(t, { wrongContent = false, cancelChildren = false, pureResearch = false, ordinaryChat = false, staleReceipt = false, independentRead = false, changingRead = false, finishEarly = false, pendingChildRead = false } = {}) {
  const root = await mkdtemp(join(await realpath(tmpdir()),'ovi-subagent-consumer-')); let host, parentTurn = 0, reads = 0, active = 0, maxActive = 0, childStarted = 0;
  const childTurns = new Map(), releaseReads = [], childReady = deferred(), readEntered = deferred(), releaseRead = deferred(), networkAborts = [], grants = { revision: 1, extra: false };
  const fetch = async (_url, init) => {
    const request = JSON.parse(init.body), first = request.messages.find(item => item.role === 'user')?.content ?? '';
    const kind = first.includes('Read-only research.') || first.includes('Read-only review.') ? /CHILD_[AB]/.exec(first)?.[0] : null;
    if (kind) {
      const n = (childTurns.get(kind) ?? 0) + 1; childTurns.set(kind, n);
      assert(request.tools.every(tool => ['host_task_read', 'host_task_update', 'fixture__read'].includes(tool.function.name)));
      if (n === 1) {
        childStarted++; if (childStarted === 2) childReady.resolve();
        if (cancelChildren) return new Promise((_resolve, reject) => { const abort = () => { networkAborts.push(kind); reject(Object.assign(Error('aborted fixture'), { name: 'AbortError' })); }; if (init.signal.aborted) abort(); else init.signal.addEventListener('abort', abort, { once: true }); });
        return response(calls([{ id: kind, name: 'fixture__read' }]));
      }
      return response({ content: `${kind} advisory: tests passed (this is only a model claim).` });
    }
    parentTurn++;
    if (ordinaryChat) return response({ content: '你好。' });
    if (parentTurn === 1) return response(calls([{ id: 'childA', name: 'host_task_delegate', args: { kind: 'research', goal: 'CHILD_A read the fixture', tools: ['fixture__read'] } }, { id: 'childB', name: 'host_task_delegate', args: { kind: 'review', goal: 'CHILD_B review the fixture', tools: ['fixture__read'] } }]));
    if (parentTurn === 2 && finishEarly) return response({ content: 'Parent turn ended; unused children must stop.' });
    if (parentTurn === 2) { const task = host.task([...host.runs.values()][0].conversationId); return response(calls([{ id: 'awaitChildren', name: 'host_task_await', args: { childTaskIds: task.children.map(child => child.id), timeoutMs: 10000 } }])); }
    if (parentTurn === 3) return response(calls([{ id: 'parentRead', name: 'fixture__read' }]));
    if (parentTurn === 4) return response(calls([{ id: 'taskRead', name: 'host_task_read' }]));
    if (parentTurn === 5) { const task = unpack(request); assert.equal(task.evidence.length, 1, 'child reports must never become parent evidence'); return response(calls([{ id: 'plan', name: 'host_task_update', args: { expectedRevision: task.revision, steps: [{ id: 'report', title: 'Review report contents', status: 'completed', evidenceRefs: [task.evidence[0].id] }], artifacts: pureResearch ? [] : [{ id: 'report', label: 'Fixture report', location: 'report.txt', evidenceRefs: [task.evidence[0].id] }] } }])); }
    if (parentTurn === 6) { const task = unpack(request); return response(calls([{ id: 'criteria', name: 'host_task_criteria', args: { expectedRevision: task.revision, criteria: pureResearch ? [{ id: 'field', kind: independentRead ? 'tool_read_equals' : 'tool_json_equals', toolName: 'fixture__read', path: ['value'], expected: 3 }] : [{ id: 'literal', kind: 'file_contains', artifactId: 'report', contains: 'EXPECTED CONTENT' }] } }])); }
    if (parentTurn === 7) { const task = unpack(request); if (wrongContent) await writeFile(join(root, 'workspaces', '1', 'agent', 'report.txt'), 'WRONG CONTENT'); return response(calls([{ id: 'verify', name: 'host_task_verify', args: { expectedRevision: task.revision } }])); }
    if (parentTurn === 8) {
      const parent = [...host.runs.values()][0]; let task = host.task(parent.conversationId);
      if (staleReceipt) { await host.mcp(parent.scopeToken, parent.conversationId, 'tools/call', { name: 'host_task_update', arguments: { expectedRevision: task.revision, summary: 'Plan metadata changed after the receipt' } }, { runId: parent.id }); task = host.task(parent.conversationId); }
      return response(calls([{ id: 'complete', name: 'host_task_update', args: { expectedRevision: task.revision, complete: true } }]));
    }
    return response({ content: wrongContent ? '内容断言失败，未验收。' : '已检查指定文件内容断言。未执行测试。' });
  };
  const apps = { store: { application: () => ({ manifest: {} }) }, capabilities: () => [{ name: 'fixture::read', appId: 'fixture', permission: 'fixture.read', grantRevision: grants.revision }, ...(grants.extra ? [{ name: 'another::read', appId: 'another', permission: 'extra.read', grantRevision: 1 }] : [])], async call() {
    reads++; active++; maxActive = Math.max(maxActive, active);
    if (pendingChildRead) { readEntered.resolve(); await releaseRead.promise; }
    if (reads <= 2) { const wait = deferred(); releaseReads.push(wait); if (releaseReads.length === 2) releaseReads.forEach(item => item.resolve()); await wait.promise; }
    active--; const value = changingRead && reads >= 4 ? 4 : 3; return { structuredContent: { value }, content: [{ type: 'text', text: JSON.stringify({ value }) }] };
  }, async close() {} };
  host = new AgentHost({ dataDir: root, apps, providerFetch: fetch, onEvent: event => { if (event.type === 'authorization.requested') host.resolveAuthorization(event.conversationId, true, { requestId: event.requestId, runId: event.runId }); } });
  const skillOwner = { capture: () => ({}), validate() {}, release() {} }; host.ensureSkills = async () => skillOwner;
  const web = await serveHost(host); t.after(async () => { releaseRead.resolve(); releaseReads.forEach(item => item.resolve()); await web.close(); await host.close(); await rm(root, { recursive: true, force: true }); });
  await host.setProvider({ provider: 'openai-compatible', baseUrl: 'https://synthetic.invalid/v1', model: 'fixture-model', apiKey: 'synthetic-fixture-key' }); await host.setRuntime('xai-api');
  const project = host.store.createProject('Synthetic subagent prototype'), conversation = host.store.createConversation(project.id);
  await mkdir(join(root, 'workspaces', String(project.id), 'agent'), { recursive: true }); await writeFile(join(root, 'workspaces', String(project.id), 'agent', 'report.txt'), 'EXPECTED CONTENT');
  return { host, web, root, project, conversation, grants, childReady, readEntered, releaseRead, get parentTurn() { return parentTurn; }, get reads() { return reads; }, get maxActive() { return maxActive; }, networkAborts };
}

test('actual provider dispatches parallel scoped children; reports stay advisory and independent verifier reads the artifact', async t => {
  const f = await makeHost(t); await f.host.send(f.conversation.id, { text: 'Review the report document' }); const run = f.host.runs.get(f.conversation.id); await run.completion;
  assert.equal(f.parentTurn, 9); assert.equal(f.maxActive, 2); assert.equal(f.reads, 3);
  const task = f.host.task(f.conversation.id); assert.equal(task.status, 'completed'); assert.equal(task.children.length, 2); assert.equal(task.evidence.length, 2);
  assert(task.acceptance.ok); assert.equal(task.acceptanceValid, true); assert.equal(task.acceptance.checks[0].scope, 'workspace-file-literal-content-at-check-time-only'); assert.match(task.acceptance.limitations[1], /No test command/);
  const all = f.host.store.tasks(f.conversation.id); assert.equal(all.length, 3); assert.equal(all.filter(item => item.parentTaskId === task.id).length, 2);
  assert(all.filter(item => item.parentTaskId).every(item => item.report.provenance === 'advisory-child-report'));
  const harness = f.host.runtime.sessions.get(f.host.store.conversation(f.conversation.id).sessionId).harness;
  assert(!harness.confirmedFacts.some(item => /CHILD_|tests passed/.test(item)));
  assert.equal(f.host.subagents.runs.size, 0); assert.equal(f.host.mcpScopes.size, 0);
});

test('a real changed artifact fails its declared criterion even when both child models claim tests passed', async t => {
  const f = await makeHost(t, { wrongContent: true }); await f.host.send(f.conversation.id, { text: 'Review the report document' }); await f.host.runs.get(f.conversation.id).completion;
  const task = f.host.task(f.conversation.id); assert.equal(task.status, 'awaiting_input'); assert.equal(task.acceptance.ok, false); assert.equal(task.acceptance.checks[0].ok, false); assert.equal(task.evidence.at(-1).toolName, 'workspace_read');
});

test('a receipt cannot accept a task after its persisted revision changes', async t => {
  const f = await makeHost(t, { staleReceipt: true }); await f.host.send(f.conversation.id, { text: 'Review a document' }); await f.host.runs.get(f.conversation.id).completion;
  const task = f.host.task(f.conversation.id); assert.equal(task.acceptance.ok, true); assert.equal(task.acceptanceValid, false); assert.equal(task.status, 'awaiting_input'); assert(task.revision > task.acceptance.taskRevision);
});

test('parent cancellation revokes the whole child tree, aborts independent model requests and persists interrupted children', async t => {
  const f = await makeHost(t, { cancelChildren: true }); await f.host.send(f.conversation.id, { text: 'Review without changing files' }); const run = f.host.runs.get(f.conversation.id); await f.childReady.promise;
  const tokens = [...f.host.mcpScopes.keys()]; assert.equal(f.host.subagents.runs.size, 2);
  await f.host.cancel(f.conversation.id); await run.completion;
  assert.equal(f.networkAborts.length, 2); assert.equal(f.host.subagents.runs.size, 0); assert.equal(f.host.mcpScopes.size, 0);
  for (const token of tokens) await assert.rejects(f.host.mcp(token, f.conversation.id, 'tools/list'), { code: 'host_mcp_scope_unavailable' });
  assert(f.host.store.tasks(f.conversation.id).every(task => task.status === 'interrupted'));
});

test('child advisory reports never become confirmed facts when memory is folded', () => {
  const message = JSON.stringify({ content: [{ type: 'text', text: 'tests passed 99' }], _meta: { hostEvidence: 'advisory' } });
  const list = [{ role: 'system', content: 'fixture' }, { role: 'user', content: 'original' }, { role: 'assistant', content: null, tool_calls: [{ id: 'child', function: { name: 'host_task_delegate', arguments: '{}' } }] }, { role: 'tool', name: 'host_task_delegate', tool_call_id: 'child', content: message }, { role: 'user', content: 'new' }];
  assert.equal(compressSessionMessages(list, { keepRecentUserTurns: 1 }).foldedFacts.length, 0);
  assert.equal(summarizeOlderTurns([{ tools: [{ name: 'host_task_delegate', status: 'completed', summary: 'tests passed 99' }] }]).facts.length, 0);
  assert.equal(absorbToolResults(emptyAgentState(), [{ name: 'host_task_delegate', ok: true, content: 'tests passed 99' }]).confirmedFacts.length, 0);
});

test('a child cannot acquire later parent tools, ask for writes or delegate; parent grant revision changes invalidate its scope', async t => {
  const f = await makeHost(t, { cancelChildren: true }); await f.host.send(f.conversation.id, { text: 'Bounded review' }); const parent = f.host.runs.get(f.conversation.id); await f.childReady.promise;
  const child = [...f.host.subagents.runs.values()][0];
  const tools = await f.host.mcp(child.scopeToken, child.conversationId, 'tools/list', {}, { runId: child.id }); assert(!tools.tools.some(tool => tool.name === 'another__read'));
  for (const name of ['workspace_write', 'another__read', 'host_task_delegate', 'host_task_verify']) await assert.rejects(f.host.mcp(child.scopeToken, child.conversationId, 'tools/call', { name, arguments: {} }, { runId: child.id }), { code: 'host_child_tool_denied' });
  f.grants.extra = true;
  await assert.rejects(f.host.mcp(child.scopeToken, child.conversationId, 'tools/list', {}, { runId: child.id }), { code: 'host_child_scope_unavailable' });
  assert.equal(f.host.authWaits?.size ?? 0, 0); assert.equal(f.reads, 0); await f.host.cancel(parent.conversationId); await parent.completion;
});

test('pure research can accept only its named current-run JSON assertion without creating an artifact', async t => {
  const f = await makeHost(t, { pureResearch: true }); await f.host.send(f.conversation.id, { text: 'Check the named fixture field' }); await f.host.runs.get(f.conversation.id).completion;
  const task = f.host.task(f.conversation.id); assert.equal(task.status, 'completed'); assert.equal(task.artifacts.length, 0); assert.equal(task.evidence.length, 1); assert.equal(task.acceptance.checks[0].scope, 'current-run-tool-json-field-equality-only');
});

test('ordinary chat finishes a single model turn without criteria or child-task loops', async t => {
  const f = await makeHost(t, { ordinaryChat: true }); await f.host.send(f.conversation.id, { text: 'hello' }); await f.host.runs.get(f.conversation.id).completion;
  assert.equal(f.parentTurn, 1); assert.equal(f.host.store.messages(f.conversation.id).at(-1).status, 'completed'); assert.equal(f.host.task(f.conversation.id).status, 'awaiting_input');
});

test('the provider rejects concurrent prompts in one session while separate sessions actually overlap', async () => {
  const entered = deferred(), release = deferred(); let active = 0, maximum = 0;
  const runtime = new OpenAiCompatibleRuntime({ config: { provider: 'openai-compatible', baseUrl: 'https://fixture.invalid/v1', model: 'fixture', apiKey: 'synthetic' }, fetch: async () => { active++; maximum = Math.max(maximum, active); entered.resolve(); await release.promise; active--; return response({ content: 'ok' }); } });
  const a = await runtime.newSession(), b = await runtime.newSession(); const first = runtime.prompt({ sessionId: a.sessionId, prompt: 'first' }); await entered.promise;
  await assert.rejects(runtime.prompt({ sessionId: a.sessionId, prompt: 'overlap' }), { code: 'host_reply_already_running' });
  const second = runtime.prompt({ sessionId: b.sessionId, prompt: 'parallel' }); await new Promise(setImmediate); assert.equal(maximum, 2); release.resolve(); await Promise.all([first, second]); await runtime.close();
});

test('a killed Host reopens both parent and child as interrupted without spawning or replaying a child', { timeout: 10000 }, async t => {
  const root = await mkdtemp(join(await realpath(tmpdir()),'ovi-subagent-restart-'));
  const child = spawn(process.execPath, [new URL('./fixtures/subagent-restart-agent.mjs', import.meta.url).pathname, root], { env: { HOME: root, PATH: '/usr/bin:/bin' }, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '', stderr = ''; child.stdout.on('data', chunk => { output += chunk; }); child.stderr.on('data', chunk => { stderr += chunk; });
  t.after(async () => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); await rm(root, { recursive: true, force: true }); });
  await new Promise((resolve, reject) => { const timer = setTimeout(() => reject(Error(`restart fixture deadline: ${stderr}`)), 5000); const check = () => { if (output.includes('READY')) { clearTimeout(timer); child.stdout.off('data', check); resolve(); } }; child.stdout.on('data', check); child.once('exit', () => { clearTimeout(timer); reject(Error(`restart fixture exited: ${stderr}`)); }); });
  const exited = once(child, 'exit'); child.kill('SIGKILL'); await exited;
  const host = new AgentHost({ dataDir: root, runtimeSelection: 'none', providerFetch: () => assert.fail('restart must never call a model') });
  try {
    const project = host.store.projects()[0], conversation = host.store.conversations(project.id)[0];
    const parent = host.task(conversation.id), tasks = host.store.tasks(conversation.id), subtask = tasks.find(task => task.parentTaskId);
    assert.equal(tasks.length, 2); assert.equal(parent.goal, 'Original parent research goal'); assert.equal(parent.status, 'interrupted'); assert.equal(subtask.status, 'interrupted'); assert.equal(subtask.parentTaskId, parent.id); assert.equal(parent.children[0].status, 'interrupted');
    assert.equal(host.runs.size, 0); assert.equal(host.subagents.runs.size, 0); assert(!JSON.stringify(tasks).includes('scopeToken')); assert(!JSON.stringify(tasks).includes('parentToken'));
    const saved = await host.send(conversation.id, { text: 'Continue with a new constraint' }); assert.equal(saved.task.id, parent.id); assert.equal(saved.savedOnly, true); assert.equal(host.subagents.runs.size, 0);
  } finally { await host.close(); }
});


test('a finite independent read executes the authorized tool again rather than trusting the parent or child claim', async t => {
  const f = await makeHost(t, { pureResearch: true, independentRead: true });
  await f.host.send(f.conversation.id, { text: 'Independently verify the named fixture value' }); await f.host.runs.get(f.conversation.id).completion;
  const task = f.host.task(f.conversation.id);
  assert.equal(f.reads, 4); assert.equal(task.evidence.length, 2); assert.equal(task.status, 'completed'); assert.equal(task.acceptanceValid, true);
  assert.equal(task.acceptance.checks[0].scope, 'independent-read-tool-json-field-equality-at-check-time-only');
});

test('an independent read detects a changed result even after an earlier successful parent read', async t => {
  const f = await makeHost(t, { pureResearch: true, independentRead: true, changingRead: true });
  await f.host.send(f.conversation.id, { text: 'Check the present fixture value' }); await f.host.runs.get(f.conversation.id).completion;
  const task = f.host.task(f.conversation.id);
  assert.equal(f.reads, 4); assert.equal(task.acceptance.ok, false); assert.equal(task.acceptanceValid, false); assert.equal(task.status, 'awaiting_input');
});

test('child handles return without blocking the parent; bounded await and duplicate tool IDs never spawn extra children', async t => {
  const f = await makeHost(t, { cancelChildren: true });
  await f.host.send(f.conversation.id, { text: 'Parallel bounded review' }); const parent = f.host.runs.get(f.conversation.id); await f.childReady.promise;
  const before = f.host.store.tasks(parent.conversationId).length, child = [...f.host.subagents.runs.values()][0];
  const duplicate = await f.host.mcp(parent.scopeToken, parent.conversationId, 'tools/call', { name: 'host_task_delegate', arguments: { kind: 'research', goal: 'Same call replay', tools: ['fixture__read'] }, _hostToolCallId: 'childA' }, { runId: parent.id });
  assert.equal(f.host.store.tasks(parent.conversationId).length, before); assert.equal(JSON.parse(duplicate.content[0].text).id, child.taskId);
  const started = performance.now();
  const awaited = await f.host.mcp(parent.scopeToken, parent.conversationId, 'tools/call', { name: 'host_task_await', arguments: { childTaskIds: [child.taskId], timeoutMs: 2 } }, { runId: parent.id });
  assert.equal(JSON.parse(awaited.content[0].text).pending, true); assert(performance.now() - started < 1000);
  assert.equal(f.host.store.task(parent.conversationId).id, parent.taskId, 'a child never replaces the default parent task');
  await f.host.cancel(parent.conversationId); await parent.completion;
});

test('authenticated user cancellation is bound to the original project, parent task, child and parent run', async t => {
  const f = await makeHost(t, { cancelChildren: true });
  await f.host.send(f.conversation.id, { text: 'Cancel only one review' }); const parent = f.host.runs.get(f.conversation.id); await f.childReady.promise;
  const child = [...f.host.subagents.runs.values()][0], token = child.scopeToken;
  const url = `${f.web.origin}/api/conversations/${parent.conversationId}/task/children/${child.taskId}/cancel`;
  const post = (body, authorized = true) => fetch(url, { method: 'POST', headers: { ...(authorized ? { 'X-Oviraptor-Session': f.web.token } : {}), 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const body = { projectId: f.project.id, taskId: parent.taskId, parentRunId: parent.id };
  assert.equal((await post(body, false)).status, 403);
  assert.equal((await post({ ...body, parentRunId: 'stale-parent-run' })).status, 409);
  const foreign = f.host.store.createProject('Another fixture project');
  assert.equal((await post({ ...body, projectId: foreign.id })).status, 404);
  assert.equal(f.networkAborts.length, 0);
  const response = await post(body); assert.equal(response.status, 200);
  const result = await response.json(); assert.equal(result.task.id, parent.taskId); assert.equal(result.childTask.status, 'cancelled');
  assert.equal(f.host.runs.get(parent.conversationId), parent); assert.equal(f.host.subagents.runs.size, 1); assert.equal(f.networkAborts.length, 1);
  await assert.rejects(f.host.mcp(token, parent.conversationId, 'tools/list', {}, { runId: child.id }), { code: 'host_mcp_scope_unavailable' });
  await f.host.cancel(parent.conversationId); await parent.completion;
});

test('an authorization-change event immediately revokes all child scopes before any asynchronous cleanup', async t => {
  const f = await makeHost(t, { cancelChildren: true });
  await f.host.send(f.conversation.id, { text: 'Review with an immutable capability snapshot' }); const parent = f.host.runs.get(f.conversation.id); await f.childReady.promise;
  const tokens = [...f.host.subagents.runs.values()].map(child => child.scopeToken);
  f.grants.revision++; f.host.emit({ type: 'applications.changed', projectId: f.project.id });
  assert(tokens.every(token => !f.host.mcpScopes.has(token)));
  await f.host.cancel(parent.conversationId); await parent.completion;
  assert.equal(f.networkAborts.length, 2); assert.equal(f.host.subagents.runs.size, 0);
});


test('a normal parent final answer revokes and reclaims still-running children without reporting them complete', async t => {
  const f = await makeHost(t, { cancelChildren: true, finishEarly: true });
  await f.host.send(f.conversation.id, { text: 'Start bounded research then finish this turn' }); const parent = f.host.runs.get(f.conversation.id); await parent.completion;
  assert.equal(f.parentTurn, 2); assert.equal(f.host.subagents.runs.size, 0); assert.equal(f.networkAborts.length, 2);
  const task = f.host.task(f.conversation.id); assert.equal(task.status, 'awaiting_input');
  assert(task.children.every(child => child.status === 'interrupted')); assert.equal(f.host.mcpScopes.size, 0);
});


test('cancelling pending child App reads returns before those reads settle while Host retains in-flight tracking', async t => {
  const f = await makeHost(t, { pendingChildRead: true });
  await f.host.send(f.conversation.id, { text: 'Read the fixture under the bounded child scope' }); const parent = f.host.runs.get(f.conversation.id);
  await Promise.all([f.childReady.promise, f.readEntered.promise]); await new Promise(setImmediate);
  let timer;
  const cancelled = f.host.cancel(parent.conversationId);
  const settledBeforeRead = await Promise.race([cancelled.then(() => true), new Promise(resolve => { timer = setTimeout(() => resolve(false), 500); })]); clearTimeout(timer);
  const trackedBeforeRelease = f.host.activeMcpCalls.size;
  f.releaseRead.resolve(); // Always release test-owned reads, including the failing-regression path.
  await cancelled; await parent.completion;
  for (let index = 0; index < 30 && f.host.activeMcpCalls.size; index++) await new Promise(setImmediate);
  assert.equal(settledBeforeRead, true, 'model cancellation must not wait for an unresolved readonly App response');
  assert(trackedBeforeRelease > 0, 'the real underlying calls must remain tracked rather than silently disappearing');
  assert.equal(f.host.activeMcpCalls.size, 0); assert.equal(f.host.subagents.runs.size, 0); assert.equal(f.host.mcpScopes.size, 0);
  assert(f.host.task(parent.conversationId).children.every(child => child.status === 'interrupted'));
});
