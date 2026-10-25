import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createServer } from 'node:http';
import { AgentHost } from '../src/service.mjs';
import { HostStore } from '../src/store.mjs';
import { DiscoveryAssist } from '../src/discovery-assist.mjs';
import { CertificateTasks } from '../src/certificate-tasks.mjs';
import { startTaskRun, bindReconciliation } from '../src/tasks.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { readChatStream, readResponsesStream, runToolCalls } from '../src/provider.mjs';
import { normalizeProviderUsage } from '../src/provider-usage.mjs';
import { toolFailureReceipt } from '../src/tool-errors.mjs';
import { serveHost } from '../src/server.mjs';

const repository = new URL('../../../', import.meta.url);
async function fixture(t, appName = 'assets') {
  const root = await mkdtemp(join(tmpdir(), 'ovi-host-reliability-'));
  const host = new AgentHost({ dataDir: root, runtime: { status: { pid: 4242 }, async close() {}, async cancel() {} } });
  t.after(async () => { host.runs.clear(); await host.close(); await rm(root, { recursive: true, force: true }); });
  const sourceDirectory = new URL(`apps/${appName}/`, repository).pathname;
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const pack = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(root, 'package') });
  await host.apps.install(pack.directory); await host.apps.setEnabled(spec.manifest.id, true);
  const project = host.store.createProject('local reliability', { externalId: '91' });
  const conversation = host.store.createConversation(project.id);
  const mode = value => host.setAccessPolicy({ projectId: project.id, mode: value, operatorConfirmed: true, expectedRevision: host.accessPolicyView(project.id).revision, expectedRuntimeIdentity: host.runtimeIdentity() });
  mode('full');
  const task = host.store.createTask(conversation.id, '补采当前项目证书');
  const user = host.store.addMessage(conversation.id, 'user', task.goal);
  const answer = host.store.addMessage(conversation.id, 'assistant', '', 'running', 'run');
  const run = { id: 'run', conversationId: conversation.id, taskId: task.id, state: 'running', userMessageId: user.id, message: answer, cancelled: false };
  host.store.saveTask(startTaskRun(task, run)); host.runs.set(conversation.id, run);
  host.mcpScopes.set('scope', { conversationId: conversation.id, runtimePid: 4242, runId: run.id });
  const call = (name, args = {}) => host.mcp('scope', conversation.id, 'tools/call', { name: `${spec.manifest.id}__${name}`, arguments: args }, { runId: run.id });
  return { root, host, project, conversation, run, call, mode, id: spec.manifest.id };
}
async function bridge(t, f, handler) {
  const server = createServer(async (req, res) => {
    let text = ''; for await (const chunk of req) text += chunk;
    const result = await handler(JSON.parse(text));
    res.end(typeof result === 'string' ? result : JSON.stringify(result));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  f.host.apps.certificateTasks = new CertificateTasks({ origin: `http://127.0.0.1:${server.address().port}`, token: 'a'.repeat(64) });
}

test('pre-admission certificate rejection remains recoverable and its reason is a durable model-visible receipt', async t => {
  const f = await fixture(t); let configured = false, calls = 0;
  await bridge(t, f, () => { calls++; return configured ? { ok: true, result: { projectId: 91, roots: ['example.test'], tasks: [] } } : { ok: false, error: '请先在新建查询中确认允许补采的域名范围', phase: 'not_started', outcomeUnknown: false }; });
  let rejection;
  await assert.rejects(f.call('certificate_task_start'), error => { rejection = error; return error.code === 'host_certificate_task_rejected' && error.details.outcomeUnknown === false; });
  const failure = f.host.store.task(f.conversation.id, f.run.taskId).evidence[0];
  assert.equal(failure.outcomeUnknown, false);
  const raw = JSON.parse(f.host.store.toolRecord(failure.rawRecordId, f.project.id).body);
  assert.match(raw.structuredContent.reason, /确认允许补采/);
  const model = await runToolCalls([{ id: 'failure', name: 'certificate_task_start' }], async () => { throw rejection; });
  assert.match(model[0].content, /确认允许补采/); assert.match(model[0].content, /operationId/);
  configured = true; await f.call('certificate_task_status'); await f.call('certificate_task_start');
  assert.equal(calls, 3); assert.equal(f.host.store.task(f.conversation.id, f.run.taskId).evidence.some(item => item.outcomeUnknown), false);
});

test('a genuinely unknown start is not replayed and can be reconciled only by its original owner operation receipt', async t => {
  const f = await fixture(t); let starts = 0, originalOperation;
  await bridge(t, f, request => {
    if (request.action === 'start') { starts++; originalOperation = request.operationId; return '{'; }
    return { ok: true, result: { projectId: 91, tasks: [{ id: 'saved-task', status: 'paused' }], operationReceipt: { operationId: request.args.operationId, action: 'start', taskId: 'saved-task', source: 'durable_certificate_task' } } };
  });
  await assert.rejects(f.call('certificate_task_start'), { code: 'host_certificate_task_outcome_unknown' });
  await assert.rejects(f.call('certificate_task_start'), error => error.code === 'host_operation_unknown' && error.details.operationId === originalOperation);
  assert.equal(starts, 1);
  const task = () => f.host.store.task(f.conversation.id, f.run.taskId);
  const unrelated = await f.call('certificate_task_status', { operationId: 'other-operation' });
  assert.equal(unrelated.structuredContent.reconciliation, undefined);
  const result = await f.call('certificate_task_status', { operationId: originalOperation });
  assert.equal(result.structuredContent.reconciliation.operationId, originalOperation);
  assert.equal(result.structuredContent.reconciliation.argumentDigest, task().evidence[0].argumentDigest);
  assert.equal(task().evidence[0].outcomeUnknown, false);
  assert.equal(task().evidence[0].resolvedBy.source, 'desktop-certificate-start');
  assert.equal(task().evidence[0].status, 'failed', 'original failure evidence is preserved');
  assert.equal(task().outcomeUnknown, false);
  assert.notEqual(task().status, 'completed', 'a recovered admission does not silently complete the plan');
});
test('invalid app arguments are known not started and do not poison a corrected mutation', async t => {
  const f = await fixture(t); let calls = 0;
  await bridge(t, f, () => { calls++; return { ok: true, result: { projectId: 91, tasks: [] } }; });
  await assert.rejects(f.call('certificate_task_start', { workers: 'wrong-type' }), error => error.code === 'app_arguments_invalid' && error.details.outcomeUnknown === false);
  assert.equal(calls, 0); assert.equal(f.host.store.task(f.conversation.id, f.run.taskId).outcomeUnknown, false);
  await f.call('certificate_task_start', { workers: 4 }); assert.equal(calls, 1);
});

test('schema-1 scanning calls recheck Full policy after dispatch and never replay after revocation', async t => {
  const f = await fixture(t, 'scanning'); let release, entered, calls = 0;
  const pending = new Promise(resolve => { release = resolve; }); const started = new Promise(resolve => { entered = resolve; });
  f.host.apps.scanningTasks = { available: true, async call() { calls++; entered(); return pending; } };
  const request = f.host.apps.call(f.id, 'scan_start', f.project.id, { taskId: 'saved-task' }, 0, 'same-operation', undefined, f.host.runtimeIdentity());
  await started; f.mode('ask'); release({ structuredContent: { task: { id: 'saved-task', status: 'running' } } });
  await assert.rejects(request, { code: 'host_tool_authority_changed' }); assert.equal(calls, 1);
});

test('steering is durable without truncation; queued restart needs continuation and delivery is recorded', async t => {
  const f = await fixture(t); const text = '只读证书不要扫描。'.repeat(600);
  const received = f.host.steer(f.conversation.id, text);
  assert.equal(received.message.content, text); assert.equal(received.message.stopReason, 'steer_queued');
  assert.equal(f.host.store.messages(f.conversation.id).at(-1).content, text);
  assert.throws(() => f.host.steer(f.conversation.id, 'x'.repeat(70000)), { code: 'host_text_invalid' });
  const copy = new HostStore(join(f.root, 'host.sqlite'));
  try { assert.equal(copy.pendingSteers(f.conversation.id)[0].content, text); copy.deliverSteers([received.message.id]); }
  finally { copy.close(); }
  assert.equal(f.host.store.messages(f.conversation.id).at(-1).stopReason, 'steer_delivered');
  assert.deepEqual(f.host.takeSteer(f.conversation.id), [text]);
  assert.equal(f.host.takeSteer(f.conversation.id).length, 0);
});

const sse = chunks => (async function* () { for (const value of chunks) yield `data: ${JSON.stringify(value)}\n\n`; })();
test('both stream formats retain actual usage including usage-only frames, cached and reasoning counters', async () => {
  const chat = await readChatStream(sse([{ choices: [{ delta: { content: 'saved' }, finish_reason: 'stop' }] }, { choices: [], usage: { prompt_tokens: 120, completion_tokens: 30, total_tokens: 150, prompt_tokens_details: { cached_tokens: 90 }, completion_tokens_details: { reasoning_tokens: 10 } } }]), () => {});
  const responses = await readResponsesStream(sse([{ type: 'response.completed', response: { status: 'completed', usage: { input_tokens: 120, output_tokens: 30, total_tokens: 150, input_tokens_details: { cached_tokens: 90 }, output_tokens_details: { reasoning_tokens: 10 } }, output: [] } }]), () => {});
  assert.deepEqual(chat.usage, responses.usage);
  assert.deepEqual(chat.usage, { inputTokens: 120, outputTokens: 30, totalTokens: 150, cachedInputTokens: 90, reasoningTokens: 10 });
  assert.equal(normalizeProviderUsage({ prompt_tokens: -1 }), null);
});

test('usage already observed before stream failure is retained once per HTTP attempt; missing usage stays unknown', async t => {
  const root = await mkdtemp(join(tmpdir(), 'ovi-host-usage-')); const store = new HostStore(join(root, 'host.sqlite'));
  t.after(async () => { store.close(); await rm(root, { recursive: true, force: true }); });
  const project = store.createProject('usage'); const conversation = store.createConversation(project.id);
  for (const requestId of ['first','retry']) store.beginProviderRequest({ requestId, conversationId: conversation.id, runId: 'run', api: 'chat', model: 'local-test' });
  const usage = { prompt_tokens: 120, completion_tokens: 30, total_tokens: 150 };
  await assert.rejects(readChatStream(sse([{ usage }, { usage }, { error: { message: 'upstream failed' } }]), () => {}, value => store.recordProviderUsage('first', value)), { code: 'host_provider_stream_failed' });
  const totals = store.providerUsage('run'); assert.equal(totals.requests, 2); assert.equal(totals.unknownRequests, 1); assert.equal(totals.inputTokens, 120); assert.equal(totals.outputTokens, 30); assert.equal(totals.price, null);
  const assistant = store.addMessage(conversation.id, 'assistant', 'interrupted', 'failed', 'run'); store.metrics('run', conversation.id, { usage: totals });
  assert.deepEqual(store.messages(conversation.id).find(row => row.id === assistant.id).usage, totals);
});
test('actual Host provider orchestration persists and displays usage without a real model request', async t => {
  const root = await mkdtemp(join(tmpdir(), 'ovi-host-usage-flow-')); const events = [];
  const host = new AgentHost({ dataDir: root, onEvent: event => events.push(event), providerFetch: async (_url, input) => {
    assert.equal(JSON.parse(input.body).stream_options.include_usage, true);
    return { ok: true, status: 200, body: sse([{ choices: [{ delta: { content: '你好。' }, finish_reason: 'stop' }] }, { choices: [], usage: { prompt_tokens: 120, completion_tokens: 30, total_tokens: 150 } }]) };
  } });
  const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); await rm(root, { recursive: true, force: true }); });
  await host.setProvider({ provider: 'openai-compatible', baseUrl: 'https://local-fixture.invalid/v1', model: 'local-test', apiKey: 'synthetic-test-key' }); await host.setRuntime('xai-api'); host.setAnswerMode('speed');
  const project = host.store.createProject('usage flow'), conversation = host.store.createConversation(project.id);
  const sent = await host.send(conversation.id, { projectId: project.id, text: '你好' }); await host.runs.get(conversation.id)?.completion;
  const usage = host.store.providerUsage(sent.run.id);
  assert.equal(usage.requests, 1); assert.equal(usage.inputTokens, 120); assert.equal(usage.outputTokens, 30); assert.equal(usage.unknownRequests, 0);
  assert.ok(events.some(event => event.type === 'runtime.usage' && event.runId === sent.run.id));
  const answer = host.store.messages(conversation.id).find(message => message.role === 'assistant'); assert.equal(answer.usage.inputTokens, 120); assert.equal(answer.status, 'completed');
  const metrics = await fetch(web.origin + '/api/runtime/metrics', { headers: { 'X-Oviraptor-Session': web.token } }).then(response => response.json());
  assert.equal(metrics.items[0].usage.outputTokens, 30);
});

test('failure receipts never include arbitrary Error properties or raw provider bodies', () => {
  const error = Object.assign(new Error('Bearer raw-should-not-appear'), { code: 'host_certificate_task_rejected', headers: { Authorization: 'secret' }, details: { reason: 'api_key=abcdefghijk123456789 请求被拒绝', phase: 'not_started', outcomeUnknown: false } });
  const receipt = toolFailureReceipt(error, 'original-operation');
  assert.match(receipt.reason, /redacted/); assert.doesNotMatch(JSON.stringify(receipt), /abcdefghijk|raw-should-not|Authorization/);
});

test('unknown cleanup resolves only from its exact original Native operation and project receipt while preserving the original failure',async t=>{
 const f=await fixture(t);let original,starts=0,mode='valid';
 const server=createServer(async(req,res)=>{let raw='';for await(const chunk of req)raw+=chunk;const q=JSON.parse(raw);if(q.action==='cleanup_start'){original=q.operationId;starts++;res.end('{');return;}
  const op=q.args.operationId;
  const receipt=mode==='latest'?null:{projectId:mode==='cross-project'?92:91,operationId:mode==='other-operation'?'different-op':op,action:mode==='wrong-action'?'restore':'start',runId:'saved-cleanup'};
  res.end(JSON.stringify({ok:true,result:mode==='valid'?{projectId:91,operation:{id:op,state:'completed',result:receipt},operationFound:true,runId:'saved-cleanup'}:{projectId:91,operationId:op,operationFound:mode!=='latest',receipt,runId:'latest-other-task'}}));
 });await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));t.after(()=>new Promise(resolve=>server.close(resolve)));
 f.host.apps.discoveryAssist=new DiscoveryAssist({origin:`http://127.0.0.1:${server.address().port}`,token:'a'.repeat(64)});
 const args={organization:'中国移动集团',autoExcludeUnrelated:true},task=()=>f.host.store.task(f.conversation.id,f.run.taskId);
 await assert.rejects(f.call('discovery_cleanup_start',args),{code:'host_discovery_task_outcome_unknown'});
 await assert.rejects(f.call('discovery_cleanup_start',args),error=>error.code==='host_operation_unknown'&&error.details.operationId===original);assert.equal(starts,1);
 for(mode of ['latest','cross-project','other-operation','wrong-action']){
  const r=await f.call('discovery_cleanup_status',{operationId:original});assert.equal(r.structuredContent.reconciliation,undefined,mode);assert.equal(task().evidence[0].outcomeUnknown,true,mode);
 }
 mode='valid';const r=await f.call('discovery_cleanup_status',{operationId:original});
 assert.equal(r.structuredContent.reconciliation.operationId,original);assert.equal(r.structuredContent.outcomeUnknown,false);assert.equal(task().evidence[0].outcomeUnknown,false);assert.equal(task().evidence[0].resolvedBy.source,'desktop-seed-cleanup-operation');assert.equal(task().evidence[0].status,'failed');assert.equal(task().outcomeUnknown,false);assert.notEqual(task().status,'completed');
 assert.equal(starts,1);
});
