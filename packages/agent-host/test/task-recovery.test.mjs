import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, realpath, rm, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { HostStore } from '../src/store.mjs';
import { serveHost } from '../src/server.mjs';
import { argumentDigest, createTask, startTaskRun, startTaskEvidence, finishTaskEvidence, updateTask } from '../src/tasks.mjs';
import { GrokAcpRuntime } from '../../runtime-acp/index.mjs';

const directory = async t => { const root = await realpath(await mkdtemp(join(tmpdir(), 'ovi-task-recovery-'))); t.after(() => rm(root, { recursive: true, force: true })); return root; };
const parse = result => JSON.parse(result.content[0].text);
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
function fixtureRuntime(handler, pid = 711) {
  return { status: { pid }, async start() {}, async newSession() { return { sessionId: 'fixture-task-session' }; }, async loadSession() {},
    bindHost(host) { this.host = host; }, async prompt(input) {
      const host = this.host, run = [...host.runs.values()].find(item => item.sessionId === input.sessionId);
      const token = [...host.mcpScopes].find(([, scope]) => scope.conversationId === run.conversationId)[0];
      const call = async (name, args = {}) => parse(await host.mcp(token, run.conversationId, 'tools/call', { name, arguments: args }));
      await handler({ host, run, input, call }); return { stopReason: 'end_turn', metrics: {} };
    }, async cancel() { return true; }, async close() {} };
}
function fixtureApps(call) {
  return { store: { application: () => ({ manifest: {} }) }, capabilities: () => ['read', 'write'].map(name => ({ name: `fixture::${name}`, permission: `fixture.${name}`, appId: 'fixture', grantRevision: 1 })), async call(...args) { return call(...args); }, async close() {} };
}
async function acceptTask(call, criterion) {
  let task = await call('host_task_read');
  task = await call('host_task_criteria', { expectedRevision: task.revision, criteria: [criterion] });
  await call('host_task_verify', { expectedRevision: task.revision });
  task = await call('host_task_read');
  return call('host_task_update', { expectedRevision: task.revision, complete: true });
}
async function send(host, conversation, text, extra = {}) {
  const response = await host.send(conversation.id, { projectId: conversation.projectId, text, ...extra });
  await host.runs.get(conversation.id)?.completion;
  return response;
}

test('actual host MCP persists the original goal, plan dependencies, successful tool evidence and reported artifacts across restart', async t => {
  const root = await directory(t), artifact = join(root, 'result.txt'); let toolCalls = 0;
  const runtime = fixtureRuntime(async ({ input, call }) => {
    assert.equal(input.taskContext.goal, 'Create a fixture report');
    assert.match(input.prompt, /Host durable task/);
    let task = await call('host_task_read');
    task = await call('host_task_update', { expectedRevision: task.revision, steps: [{ id: 'produce', title: 'Create report', status: 'in_progress' }, { id: 'review', title: 'Review report', status: 'pending', dependsOn: ['produce'] }] });
    await call('fixture__write'); await call('fixture__read');
    task = await call('host_task_read');
    assert.equal(task.evidence.length, 2); assert(task.evidence.every(item => item.status === 'completed'));
    await call('host_task_update', { expectedRevision: task.revision,
      steps: task.steps.map(step => ({ id: step.id, revision: step.revision, title: step.title, status: 'completed', dependsOn: step.dependsOn, evidenceRefs: [task.evidence[step.id === 'produce' ? 0 : 1].id] })),
      artifacts: [{ id: 'report', label: 'Fixture report', location: artifact, evidenceRefs: [task.evidence[0].id] }] });
    await acceptTask(call, { id: 'content', kind: 'tool_read_equals', toolName: 'fixture__read', path: ['value'], expected: 'fixture report' });
  });
  const host = new AgentHost({ dataDir: root, runtime, apps: fixtureApps(async (_app, name) => {
    toolCalls++; if (name === 'write') await writeFile(artifact, 'fixture report');
    return { content: [{ type: 'text', text: JSON.stringify({ value: name === 'write' ? artifact : await readFile(artifact, 'utf8') }) }] };
  }) });
  const project = host.store.createProject('Synthetic task'), conversation = host.store.createConversation(project.id);
  const result = await send(host, conversation, 'Create a fixture report');
  assert.equal(toolCalls, 3);
  const task = host.task(conversation.id); assert.equal(task.id, result.task.id); assert.equal(task.status, 'completed');
  assert.equal(task.steps[1].dependsOn[0], 'produce'); assert(task.steps.every(step => step.verified));
  assert.equal(task.artifacts[0].provenance, 'model-reported'); assert.equal(task.artifacts[0].location, artifact);
  assert.equal(host.store.messages(conversation.id).at(-1).status, 'completed');
  await host.close();
  const restored = new AgentHost({ dataDir: root }); t.after(() => restored.close());
  assert.deepEqual(restored.task(conversation.id), task); assert.equal(restored.runs.size, 0);
});

test('a real killed host marks durable in-flight effects interrupted; explicit continue verifies before proceeding and never replays the write', { timeout: 12000 }, async t => {
  const root = await directory(t), effects = join(root, 'effects.log');
  const childCode = `
    import {AgentHost} from ${JSON.stringify(new URL('../src/service.mjs', import.meta.url).href)};
    import {appendFileSync} from 'node:fs';
    const root=process.argv[1];let host;
    const runtime={status:{pid:801},async start(){},async newSession(){return{sessionId:'crash-session'}},async close(){},async cancel(){},async prompt(){
      const run=[...host.runs.values()][0],token=[...host.mcpScopes.keys()][0];
      let task=host.task(run.conversationId);
      await host.mcp(token,run.conversationId,'tools/call',{name:'host_task_update',arguments:{expectedRevision:task.revision,steps:[{id:'write',title:'Write once',status:'in_progress'}]}});
      await host.mcp(token,run.conversationId,'tools/call',{name:'fixture__write',arguments:{},_hostToolCallId:'uncertain-write'});
    }};
    const apps={store:{application(){return{manifest:{}}}},capabilities(){return[{name:'fixture::write',appId:'fixture',grantRevision:1}]},async call(){appendFileSync(root+'/effects.log','write\\n');process.stdout.write('READY\\n');await new Promise(()=>{})},async close(){}};
    host=new AgentHost({dataDir:root,runtime,apps});const p=host.store.createProject('Crash fixture'),c=host.store.createConversation(p.id);
    await host.send(c.id,{text:'Original durable write task'});setInterval(()=>{},1000);
  `;
  const child = spawn(process.execPath, ['--input-type=module', '-e', childCode, root], { stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = ''; child.stdout.on('data', chunk => { stdout += chunk; }); child.stderr.on('data', chunk => { stderr += chunk; });
  t.after(() => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); });
  await new Promise((resolve, reject) => { const timeout = setTimeout(() => reject(Error(`child did not reach effect: ${stderr}`)), 7000); const check = chunk => { if ((stdout + chunk).includes('READY')) { clearTimeout(timeout); child.stdout.off('data', check); resolve(); } }; child.stdout.on('data', check); child.once('exit', code => { clearTimeout(timeout); reject(Error(`child exited ${code}: ${stderr}`)); }); });
  const exited = once(child, 'exit'); child.kill('SIGKILL'); await exited;
  const reopened = new HostStore(join(root, 'host.sqlite')), conversation = reopened.conversations(reopened.projects()[0].id)[0];
  const interrupted = reopened.task(conversation.id); assert.equal(interrupted.status, 'interrupted'); assert.equal(interrupted.outcomeUnknown, true);
  assert.equal(interrupted.steps[0].status, 'interrupted'); assert.equal(interrupted.evidence[0].status, 'interrupted');
  assert.equal(reopened.messages(conversation.id).at(-1).status, 'interrupted');
  let reads = 0;
  const runtime = fixtureRuntime(async ({ input, call }) => {
    assert.equal(input.taskContext.goal, 'Original durable write task'); assert.match(input.prompt, /unknown outcomes/);
    let current = await call('host_task_read');
    await assert.rejects(call('host_task_update', { expectedRevision: interrupted.revision, complete: true }), { code: 'host_task_revision_conflict' });
    await assert.rejects(call('host_task_update', { expectedRevision: current.revision, steps: [{ id: 'write', revision: current.steps[0].revision, title: 'Write once', status: 'completed', evidenceRefs: [] }] }), { code: 'host_task_verification_required' });
    const unknownWrite = (await call('host_task_read')).evidence.find(item => item.outcomeUnknown && !item.readOnly);
    const result = await call('fixture__read'); assert.equal(result.count, 1);
    assert.equal(result.reconciliation.operationId, unknownWrite.operationId);
    current = await call('host_task_read');
    await call('host_task_update', { expectedRevision: current.revision, steps: [{ id: 'write', revision: current.steps[0].revision, title: 'Write once', status: 'completed', evidenceRefs: [current.evidence[1].id], resolvesEvidenceRefs: [current.evidence[0].id] }] });
    await acceptTask(call, { id: 'one-write', kind: 'tool_json_equals', toolName: 'fixture__read', path: ['count'], expected: 1 });
  }, 802);
  const host = new AgentHost({ dataDir: root, store: reopened, runtime, apps: fixtureApps(async (_app, name) => {
    assert.equal(name, 'read', 'continuation must not replay a side effect'); reads++;
    const unknownWrite = host.task(conversation.id).evidence.find(item => item.outcomeUnknown && !item.readOnly);
    return { content: [{ type: 'text', text: JSON.stringify({ count: (await readFile(effects, 'utf8')).trim().split('\n').length, reconciliation: { operationId: unknownWrite.operationId, argumentDigest: unknownWrite.argumentDigest, postcondition: 'effects log has one write' } }) }] };
  }) });
  t.after(() => host.close());
  assert.equal(host.runs.size, 0); assert.equal(reads, 0);
  const response = await host.continueTask(conversation.id, { taskId: interrupted.id, projectId: conversation.projectId });
  await host.runs.get(conversation.id).completion;
  assert.equal(response.task.id, interrupted.id); assert.equal(reads, 1); assert.equal(await readFile(effects, 'utf8'), 'write\n');
  const finished = host.task(conversation.id); assert.equal(finished.status, 'completed'); assert.equal(finished.outcomeUnknown, false); assert.equal(finished.runs.length, 2);
});

test('new prompts retain an unfinished task; final answers end the run without forcing another model request or claiming task acceptance', async t => {
  const root = await directory(t), contexts = [];
  const host = new AgentHost({ dataDir: root, runtime: fixtureRuntime(async ({ input }) => contexts.push(input.taskContext)) }); t.after(() => host.close());
  const project = host.store.createProject('Synthetic follow-up'), conversation = host.store.createConversation(project.id);
  const first = await send(host, conversation, 'Original goal');
  assert.equal(host.task(conversation.id).status, 'awaiting_input'); assert.equal(host.runs.size, 0);
  const next = await send(host, conversation, 'Add a constraint');
  assert.equal(next.task.id, first.task.id); assert.equal(contexts.length, 2); assert(contexts.every(item => item.goal === 'Original goal'));
  const foreign = host.store.createConversation(project.id);
  await assert.rejects(send(host, foreign, 'must not bind', { taskId: first.task.id }), { code: 'host_task_unavailable' });
  assert.equal(host.store.messages(foreign.id).length, 0);
});

test('task API is session/project/conversation scoped and continue is rejected while the original run is active', async t => {
  const host = new AgentHost({ dataDir: await directory(t), runtime: fixtureRuntime(async () => {}) });
  const web = await serveHost(host); t.after(async () => { await web.close(); await host.close(); });
  const p1 = host.store.createProject('Synthetic API'), p2 = host.store.createProject('Foreign API'), c = host.store.createConversation(p1.id);
  await send(host, c, 'API task'); const task = host.task(c.id);
  const url = `${web.origin}/api/conversations/${c.id}/task`;
  assert.equal((await fetch(`${url}?projectId=${p1.id}`)).status, 403);
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  assert.equal((await fetch(`${url}?projectId=${p2.id}`, { headers })).status, 404);
  assert.equal((await (await fetch(`${url}?projectId=${p1.id}`, { headers })).json()).task.id, task.id);
  host.runs.set(c.id, { id: 'active-synthetic' });
  const result = await fetch(`${url}/continue`, { method: 'POST', headers, body: JSON.stringify({ projectId: p1.id, taskId: task.id }) });
  assert.equal(result.status, 409);
  assert.equal((await fetch(`${web.origin}/api/conversations/${c.id}?projectId=${p1.id}`, { method: 'DELETE', headers })).status, 409);
  assert.equal(host.task(c.id).id, task.id); host.runs.delete(c.id);
});

test('task updates reject dependency cycles, unsupported evidence and stale task/step revisions', () => {
  let task = { ...createTask({ id: 'conversation', projectId: 1 }, 'Synthetic goal'), revision: 1 };
  assert.throws(() => updateTask(task, { expectedRevision: 1, steps: [{ id: 'a', title: 'a', status: 'pending', dependsOn: ['b'] }, { id: 'b', title: 'b', status: 'pending', dependsOn: ['a'] }] }), { code: 'host_task_dependency_invalid' });
  task = updateTask(task, { expectedRevision: 1, steps: [{ id: 'a', title: 'a', status: 'completed' }] });
  assert.equal(task.steps[0].verified, false); assert.throws(() => updateTask(task, { expectedRevision: 1, complete: true }), { code: 'host_task_completion_unverified' });
  assert.throws(() => updateTask(task, { expectedRevision: 1, steps: [{ id: 'a', revision: 0, title: 'a', status: 'completed' }] }), { code: 'host_task_revision_conflict' });
  assert.throws(() => updateTask(task, { expectedRevision: 1, steps: [{ id: 'a', revision: 1, title: 'a', status: 'completed', evidenceRefs: ['invented'] }] }), { code: 'host_task_evidence_invalid' });
  task = startTaskRun(task, { id: 'run', userMessageId: 'user', message: { id: 'assistant' } });
  const reopened = startTaskEvidence({ ...task, status: 'completed' }, 'run', 'fixture_write', 'late'); assert.equal(reopened.task.status, 'running', 'tool activity after acceptance invalidates task completion');
  const started = startTaskEvidence(task, 'run', 'fixture_read', 'call'); task = finishTaskEvidence(started.task, started.evidenceId, { ok: false, summary: 'failed' });
  assert.throws(() => updateTask(task, { expectedRevision: 1, steps: [{ id: 'a', revision: 1, title: 'a', status: 'completed', evidenceRefs: [started.evidenceId] }] }), { code: 'host_task_evidence_invalid' });
  task = finishTaskEvidence(task, started.evidenceId, { ok: true });
  const first = updateTask(task, { expectedRevision: 1, steps: [{ id: 'a', revision: 1, title: 'a', status: 'completed', evidenceRefs: [started.evidenceId] }] });
  assert.throws(() => updateTask(first, { expectedRevision: 1, steps: [{ id: 'a', revision: first.steps[0].revision, title: 'Perform a new write', status: 'completed', evidenceRefs: [started.evidenceId] }], complete: true }), { code: 'host_task_evidence_stale' });
});

test('a newer unrelated read cannot clear an unknown side effect', () => {
  const digest = argumentDigest({ path: 'fixture.txt' });
  let task = startTaskRun({ ...createTask({ id: 'conversation', projectId: 1 }, 'Synthetic goal'), revision: 1 }, { id: 'run-1', userMessageId: 'user', message: { id: 'assistant' } });
  const write = startTaskEvidence(task, 'run-1', 'fixture__write', 'write-call', { readOnly: false, operationId: 'op-write', argumentDigest: digest });
  task = finishTaskEvidence(write.task, write.evidenceId, { ok: false, outcomeUnknown: true });
  task = { ...task, outcomeUnknown: true, steps: [{ id: 'write', revision: 1, title: 'Write once', status: 'interrupted', outcomeUnknown: true, interruptedAt: task.evidence[0].finishedAt, interruptedRunId: 'run-1', dependsOn: [], evidenceRefs: [], resolvesEvidenceRefs: [] }] };
  task = startTaskRun(task, { id: 'run-2', userMessageId: 'user-2', message: { id: 'assistant-2' } });
  const unrelated = startTaskEvidence(task, 'run-2', 'fixture__read', 'read-call', { readOnly: true, operationId: 'op-read', argumentDigest: argumentDigest({}) });
  task = finishTaskEvidence(unrelated.task, unrelated.evidenceId, { ok: true, summary: '{"count":1}' });
  const proposal = { expectedRevision: task.revision, steps: [{ id: 'write', revision: 1, title: 'Write once', status: 'completed', evidenceRefs: [unrelated.evidenceId], resolvesEvidenceRefs: [write.evidenceId] }] };
  assert.throws(() => updateTask(task, proposal), { code: 'host_task_verification_required' });
  task = finishTaskEvidence(task, unrelated.evidenceId, { ok: true, reconciliation: { operationId: 'op-other', argumentDigest: digest, postcondition: 'wrong operation' } });
  assert.throws(() => updateTask(task, proposal), { code: 'host_task_verification_required' });
  task = finishTaskEvidence(task, unrelated.evidenceId, { ok: true, reconciliation: { operationId: 'op-write', argumentDigest: digest, postcondition: 'fixture file has the expected bytes' } });
  const cleared = updateTask(task, proposal);
  assert.equal(cleared.evidence.find(item => item.id === write.evidenceId).outcomeUnknown, false);
  assert.equal(cleared.steps[0].outcomeUnknown, undefined);
});

test('the production provider consumes task tools and completes a durable plan using actual host tool results', async t => {
  const root = await directory(t), artifact = join(root, 'provider-report.txt'); let turn = 0, writes = 0;
  const completion = (name, args) => ({ choices: [{ delta: { tool_calls: [{ index: 0, id: `call-${turn}`, function: { name, arguments: JSON.stringify(args) } }] }, finish_reason: 'tool_calls' }] });
  const host = new AgentHost({ dataDir: root, apps: fixtureApps(async (_app, name) => {
    if (name === 'write') { writes++; await writeFile(artifact, 'provider-produced fixture'); }
    return { content: [{ type: 'text', text: JSON.stringify({ artifact, value: await readFile(artifact, 'utf8') }) }] };
  }), providerFetch: async (_url, init) => {
    const request = JSON.parse(init.body); turn++;
    assert(request.tools.some(tool => tool.function.name === 'host_task_update'));
    const last = request.messages.filter(item => item.role === 'tool').at(-1);
    let delta;
    if (turn === 1) delta = completion('host_task_read', {});
    else if (turn === 2) {
      const task = JSON.parse(last.content).content ? parse(JSON.parse(last.content)) : JSON.parse(last.content);
      delta = completion('host_task_update', { expectedRevision: task.revision, steps: [{ id: 'report', title: 'Write fixture report', status: 'in_progress' }] });
    } else if (turn === 3) delta = completion('fixture__write', {});
    else if (turn === 4) delta = completion('host_task_read', {});
    else if (turn === 5) {
      const task = parse(JSON.parse(last.content));
      delta = completion('host_task_update', { expectedRevision: task.revision, steps: [{ id: 'report', revision: task.steps[0].revision, title: 'Write fixture report', status: 'completed', evidenceRefs: [task.evidence[0].id] }], artifacts: [{ id: 'report', label: 'Synthetic report', location: artifact, evidenceRefs: [task.evidence[0].id] }] });
    } else if (turn === 6) {
      const task = parse(JSON.parse(last.content));
      delta = completion('host_task_criteria', { expectedRevision: task.revision, criteria: [{ id: 'actual-read', kind: 'tool_read_equals', toolName: 'fixture__read', path: ['value'], expected: 'provider-produced fixture' }] });
    } else if (turn === 7) {
      const task = parse(JSON.parse(last.content)); delta = completion('host_task_verify', { expectedRevision: task.revision });
    } else if (turn === 8) {
      const receipt = parse(JSON.parse(last.content)); delta = completion('host_task_update', { expectedRevision: receipt.taskRevision, complete: true });
    } else delta = { choices: [{ delta: { content: 'Fixture report saved; the declared read assertion passed. No tests were executed.' }, finish_reason: 'stop' }] };
    return { ok: true, status: 200, body: (async function* () { yield `data: ${JSON.stringify(delta)}\n\n`; })() };
  } });
  const web = await serveHost(host); t.after(async () => { await web.close(); await host.close(); });
  host.setAnswerMode('quality'); await host.setProvider({ provider: 'openai-compatible', baseUrl: 'https://fixture.invalid/v1', model: 'fixture-model', apiKey: 'synthetic-key' }); await host.setRuntime('xai-api');
  const project = host.store.createProject('Synthetic provider task'), conversation = host.store.createConversation(project.id);
  await send(host, conversation, 'Create a durable fixture report');
  assert.equal(turn, 9); assert.equal(writes, 1); assert.equal(host.task(conversation.id).status, 'completed'); assert.equal(host.task(conversation.id).acceptanceValid, true); assert.equal(host.task(conversation.id).evidence.length, 2);
  assert.equal(host.store.messages(conversation.id).at(-1).status, 'completed');
});

test('cancelling after a side effect retains unknown evidence and invalidates the old MCP scope before any further task mutation', async t => {
  const entered = deferred(), release = deferred(), root = await directory(t); let writes = 0;
  const host = new AgentHost({ dataDir: root, runtime: fixtureRuntime(async ({ call }) => { await call('fixture__write'); }), apps: fixtureApps(async () => { writes++; entered.resolve(); await release.promise; return { content: [{ type: 'text', text: '{}' }] }; }) });
  t.after(() => host.close());
  const p = host.store.createProject('Synthetic cancellation'), c = host.store.createConversation(p.id);
  await host.send(c.id, { text: 'Write once and inspect' }); const run = host.runs.get(c.id); await entered.promise;
  const token = [...host.mcpScopes.keys()][0], before = host.task(c.id).revision;
  await host.cancel(c.id);
  await assert.rejects(host.mcp(token, c.id, 'tools/call', { name: 'host_task_update', arguments: { expectedRevision: before, complete: true } }), { code: 'host_mcp_scope_unavailable' });
  assert.equal(host.task(c.id).revision, before);
  release.resolve(); await run.completion;
  assert.equal(writes, 1); const task = host.task(c.id); assert.equal(task.status, 'interrupted'); assert.equal(task.outcomeUnknown, true); assert.equal(task.evidence[0].outcomeUnknown, true);
});

test('a declared read-only failure is failed evidence and does not create an unresolved write obligation', async t => {
  const host = new AgentHost({ dataDir: await directory(t), runtime: fixtureRuntime(async ({ call }) => { await call('fixture__read'); }), apps: fixtureApps(async () => ({ isError: true, content: [{ type: 'text', text: '{}' }] })) });
  t.after(() => host.close()); const p = host.store.createProject('Synthetic failed read'), c = host.store.createConversation(p.id);
  await send(host, c, 'Inspect a fixture'); const task = host.task(c.id);
  assert.equal(task.evidence[0].status, 'failed'); assert.equal(task.evidence[0].readOnly, true); assert.equal(task.evidence[0].outcomeUnknown, false); assert.equal(task.outcomeUnknown, false);
});

test('a pending write step cannot be accepted using a previous run read result', async t => {
  let turn = 0, reads = 0;
  const host = new AgentHost({ dataDir: await directory(t), runtime: fixtureRuntime(async ({ call }) => {
    if (++turn === 1) {
      await call('fixture__read'); const task = await call('host_task_read');
      await call('host_task_update', { expectedRevision: task.revision, steps: [{ id: 'read', title: 'Read initial state', status: 'completed', evidenceRefs: [task.evidence[0].id] }, { id: 'write', title: 'Produce report', status: 'pending' }] });
    } else {
      const task = await call('host_task_read');
      await assert.rejects(call('host_task_update', { expectedRevision: task.revision, steps: task.steps.map(step => ({ id: step.id, revision: step.revision, title: step.title, status: 'completed', evidenceRefs: [task.evidence[0].id] })) }), { code: 'host_task_evidence_stale' });
      await assert.rejects(call('host_task_update', { expectedRevision: task.revision, complete: true }), { code: 'host_task_acceptance_required' });
    }
  }), apps: fixtureApps(async () => { reads++; return { content: [{ type: 'text', text: '{}' }] }; }) });
  t.after(() => host.close()); const p = host.store.createProject('Synthetic stale evidence'), c = host.store.createConversation(p.id);
  await send(host, c, 'Read and create a report'); await send(host, c, 'Perform the pending write');
  const task = host.task(c.id); assert.equal(reads, 1); assert.equal(task.status, 'awaiting_input'); assert.equal(task.steps[1].verified, false);
});

test('a cancelled run with an admitted write cannot be verified before that write settles, and its late outcome reopens unknown state', async t => {
  const root = await directory(t), ready = deferred(), ended = deferred(), entered = deferred(), release = deferred(); let writes = 0;
  const runtime = { status: { pid: 901 }, async start() {}, async newSession() { return { sessionId: 'late-effect-session' }; }, async loadSession() {}, async prompt() { ready.resolve(); return ended.promise; }, async cancel() { ended.resolve({ stopReason: 'cancelled', metrics: {} }); return true; }, async close() {} };
  const host = new AgentHost({ dataDir: root, runtime, apps: fixtureApps(async () => { entered.resolve(); await release.promise; writes++; return { content: [{ type: 'text', text: '{}' }] }; }) });
  t.after(() => host.close()); const p = host.store.createProject('Synthetic late effect'), c = host.store.createConversation(p.id);
  await host.send(c.id, { text: 'Write once' }); const run = host.runs.get(c.id); await ready.promise;
  const token = [...host.mcpScopes.keys()][0], before = host.task(c.id).revision;
  await assert.rejects(host.mcp(token, c.id, 'tools/list', {}, { runId: 'previous-run' }), { code: 'host_mcp_scope_unavailable' }); assert.equal(host.task(c.id).revision, before);
  const pending = host.mcp(token, c.id, 'tools/call', { name: 'fixture__write' }).catch(error => error.code);
  await entered.promise; await host.cancel(c.id); await run.completion;
  await assert.rejects(host.continueTask(c.id, { taskId: host.task(c.id).id }), { code: 'host_task_effect_pending' });
  assert.equal(writes, 0); release.resolve(); assert.equal(await pending, 'host_mcp_scope_unavailable');
  const task = host.task(c.id); assert.equal(writes, 1); assert.equal(task.status, 'interrupted'); assert.equal(task.outcomeUnknown, true); assert.equal(task.evidence[0].outcomeUnknown, true); assert.equal(host.inflightEffects.size, 0);
});

test('a real ACP child cancels pending permissions and a late approval cannot authorize the following run', { timeout: 12000 }, async t => {
  const root = await directory(t), events = []; let host;
  const runtime = new GrokAcpRuntime({ cwd: root, command: process.execPath, args: [fileURLToPath(new URL('../../runtime-acp/tests/fixtures/agent.mjs', import.meta.url))], permissionPolicy: 'ask', requestTimeoutMs: 5000, promptTimeoutMs: 7000, cancelTimeoutMs: 2000, onEvent: event => host?.runtimeEvent(event) });
  // This protocol fixture selects behavior by an exact sentinel. Only the test
  // adapter replaces its input; production Host prompt/history stays untouched.
  const prompt = runtime.prompt.bind(runtime); runtime.prompt = input => prompt({ ...input, prompt: input.userText });
  host = new AgentHost({ dataDir: root, runtime, onEvent: event => events.push(event) }); t.after(() => host.close());
  const until = async condition => { const deadline = Date.now() + 5000; while (!condition()) { if (Date.now() > deadline) throw Error('ACP permission fixture deadline'); await new Promise(resolve => setTimeout(resolve, 5)); } };
  const p = host.store.createProject('Synthetic real ACP permission'), c = host.store.createConversation(p.id);
  await host.send(c.id, { text: 'permission-cancel' }); const first = host.runs.get(c.id); await until(() => events.some(event => event.type === 'permission.request'));
  const oldPermission = [...host.permissions.keys()][0], oldToken = [...host.mcpScopes.keys()][0];
  await host.cancel(c.id); await first.completion; assert.equal(runtime.status.pendingPermissions, 0); assert.equal(host.permissions.size, 0);
  await host.send(c.id, { text: 'wait' }); const second = host.runs.get(c.id); await until(() => runtime.status.activeTurns === 1);
  assert.notEqual([...host.mcpScopes.keys()][0], oldToken);
  await assert.rejects(host.decide(oldPermission, 'yes'), { code: 'host_permission_choice_invalid' }); assert.equal(runtime.resolvePermission(oldPermission, 'yes'), false);
  await assert.rejects(host.mcp(oldToken, c.id, 'tools/list'), { code: 'host_mcp_scope_unavailable' });
  await host.cancel(c.id); await second.completion; assert.equal(host.permissions.size, 0);
});
