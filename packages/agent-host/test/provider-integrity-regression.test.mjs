import test from 'node:test';
import assert from 'node:assert/strict';
import { providerHttpError, OpenAiCompatibleRuntime, readChatStream, readResponsesStream, runToolCalls } from '../src/provider.mjs';

const config = { provider: 'openai-compatible', apiKey: 'synthetic-test-key', model: 'fixture-model', baseUrl: 'https://fixture.invalid/v1' };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const stream = chunks => (async function* () { for (const chunk of chunks) yield `data: ${typeof chunk === 'string' ? chunk : JSON.stringify(chunk)}\n\n`; })();
const text = value => ({ choices: [{ delta: { content: value }, finish_reason: 'stop' }] });
const toolTurn = calls => ({ choices: [{ delta: { tool_calls: calls.map((call, index) => ({ index, id: call.id, function: { name: call.name, arguments: call.arguments ?? '{}' } })) }, finish_reason: 'tool_calls' }] });
const response = chunks => ({ ok: true, status: 200, body: stream(chunks) });
const session = runtime => runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'fixture-token' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'fixture-conversation' }] }] });
const assertPaired = messages => {
  const ids = messages.flatMap(message => message.tool_calls ?? []).map(call => call.id).sort();
  const outputs = messages.filter(message => message.role === 'tool').map(message => message.tool_call_id).sort();
  assert.deepEqual(outputs, ids);
};

test('concurrent provider sessions retain their own streams after the other session completes', async () => {
  const entered = [deferred(), deferred()], release = [deferred(), deferred()], events = [];
  let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, onEvent: event => events.push(event), fetch: async () => {
    const index = count++; entered[index].resolve(); await release[index].promise;
    return response([text(index === 0 ? 'ANSWER_A' : 'ANSWER_B')]);
  } });
  const a = await session(runtime), b = await session(runtime);
  const first = runtime.prompt({ sessionId: a.sessionId, prompt: 'hello' }); await entered[0].promise;
  const second = runtime.prompt({ sessionId: b.sessionId, prompt: 'hello' }); await entered[1].promise;
  release[0].resolve(); await first; release[1].resolve(); await second;
  assert.deepEqual(events.filter(event => event.type === 'text_delta').map(event => [event.sessionId, event.text]), [[a.sessionId, 'ANSWER_A'], [b.sessionId, 'ANSWER_B']]);
});

test('a concurrent speed session cannot downgrade the settings of an in-flight quality tool loop', async () => {
  const entered = deferred(), release = deferred(), requests = []; let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async (url, init) => {
    requests.push(JSON.parse(init.body)); const index = ++count;
    if (index === 1) { entered.resolve(); await release.promise; return response([toolTurn([{ id: 'read', name: 'workspace_read' }])]); }
    return response([text(index === 2 ? 'speed answer' : 'quality answer')]);
  } });
  runtime.bindHost({ async mcp(token, conversation, method) { return method === 'tools/list' ? { tools: [{ name: 'workspace_read' }] } : { content: [{ type: 'text', text: 'fixture' }] }; } });
  const a = await session(runtime), b = await session(runtime);
  const first = runtime.prompt({ sessionId: a.sessionId, prompt: 'hello', answerMode: 'quality' }); await entered.promise;
  await runtime.prompt({ sessionId: b.sessionId, prompt: 'hello', answerMode: 'speed' }); release.resolve(); await first;
  assert.deepEqual(requests.map(request => request.max_tokens), [4096, 2048, 4096]);
});

test('a model request ending without its terminal event rejects partial Chat and Responses text', async () => {
  await assert.rejects(readChatStream(stream([{ choices: [{ delta: { content: 'partial' } }] }]), () => {}), { code: 'host_provider_stream_incomplete' });
  await assert.rejects(readResponsesStream(stream([{ type: 'response.output_text.delta', delta: 'partial' }]), () => {}), { code: 'host_provider_stream_incomplete' });
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async () => response([{ choices: [{ delta: { content: 'partial' } }] }]) });
  const active = await session(runtime);
  await assert.rejects(runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' }), { code: 'host_provider_stream_incomplete' });
});

test('a provider refusal is a terminal result and does not keep the tool call', async () => {
  const chat = await readChatStream(stream([
    { choices: [{ delta: { refusal: 'cannot do that' } }] },
    { choices: [{ delta: { tool_calls: [{ index: 0, id: 't', function: { name: 'fixture__write', arguments: '{}' } }] }, finish_reason: 'refusal' }] },
  ]), () => {});
  assert.equal(chat.finishReason, 'refusal');
  assert.equal(chat.toolCalls.length, 0);
  assert.equal(chat.refusal.category, 'model_refusal');
  const responses = await readResponsesStream(stream([
    { type: 'response.output_item.done', item: { type: 'function_call', call_id: 'c1', name: 'fixture__write', arguments: '{}' } },
    { type: 'response.completed', response: { id: 'resp', status: 'completed', output: [{ type: 'message', content: [{ type: 'refusal', refusal: 'no' }] }] } },
  ]), () => {});
  assert.equal(responses.finishReason, 'refusal');
  assert.equal(responses.toolCalls.length, 0);
  let toolCalls = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async () => response([{ choices: [{ delta: { refusal: 'refused' }, finish_reason: 'refusal' }] }]) });
  runtime.bindHost({ async mcp(_token, _conversation, method) { if (method === 'tools/call') toolCalls += 1; return { tools: [{ name: 'fixture__write', inputSchema: { type: 'object' } }] }; } });
  const active = await session(runtime);
  const result = await runtime.prompt({ sessionId: active.sessionId, prompt: 'do the thing', userText: 'do the thing' });
  assert.equal(result.stopReason, 'refusal');
  assert.equal(toolCalls, 0);
});

test('explicit Chat and Responses error events reject instead of yielding a completed answer', async () => {
  await assert.rejects(readChatStream(stream([{ error: { message: 'synthetic-secret-do-not-expose' } }]), () => {}), error => error.code === 'host_provider_stream_failed' && !JSON.stringify(error).includes('synthetic-secret-do-not-expose'));
  for (const type of ['error', 'response.failed']) {
    await assert.rejects(readResponsesStream(stream([{ type, response: { status: 'failed', error: { message: 'synthetic' } } }]), () => {}), { code: 'host_provider_stream_failed' });
  }
  await assert.rejects(readChatStream(stream([{ choices: [{ delta: {}, finish_reason: 'error' }] }]), () => {}), { code: 'host_provider_stream_failed' });
  await assert.rejects(readResponsesStream(stream([{ type: 'response.completed', response: { status: 'incomplete' } }]), () => {}), { code: 'host_provider_stream_incomplete' });
  await assert.rejects(readChatStream(stream([{ choices: [{ delta: {}, finish_reason: 'tool_calls' }] }]), () => {}), { code: 'host_provider_tool_calls_invalid' });
});

test('MCP isError is failed evidence while a successful tool result remains successful', async () => {
  const results = await runToolCalls([{ id: 'failed', name: 'inventory_query' }, { id: 'ok', name: 'inventory_stats' }], async call => ({ isError: call.id === 'failed', content: [{ type: 'text', text: call.id === 'failed' ? 'synthetic permission denial' : 'count=3' }] }));
  assert.equal(results[0].ok, false); assert.equal(results[1].ok, true);
  const events = [], requests = []; let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, answerMode: 'speed', onEvent: event => events.push(event), fetch: async (url, init) => {
    requests.push(JSON.parse(init.body));
    return response([++count === 1 ? toolTurn([{ id: 'failed', name: 'inventory_query' }]) : text('本次查询失败。')]);
  } });
  runtime.bindHost({ async mcp(token, conversation, method) { return method === 'tools/list' ? { tools: [{ name: 'inventory_query' }] } : { isError: true, content: [{ type: 'text', text: 'synthetic denied' }] }; } });
  const active = await session(runtime); const result = await runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  assert.equal(result.harness.confirmedFacts.length, 0);
  assert(events.some(event => event.type === 'tool_update' && event.toolCallId === 'failed' && event.status === 'failed'));
  assertPaired(requests[1].messages);
});

test('an unrelated successful workspace read cannot validate an asserted inventory count', async () => {
  let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async () => response([++count === 1 ? toolTurn([{ id: 'file', name: 'workspace_read' }]) : text('共有 30 个资产')]) });
  runtime.bindHost({ async mcp(token, conversation, method) { return method === 'tools/list' ? { tools: [{ name: 'workspace_read' }] } : { content: [{ type: 'text', text: 'unrelated source file' }] }; } });
  const active = await session(runtime); const result = await runtime.prompt({ sessionId: active.sessionId, prompt: '我的资产有几个？', userText: '我的资产有几个？' });
  assert.equal(result.replaceText, true); assert.doesNotMatch(result.assistantText, /30/); assert.match(result.assistantText, /没有本轮只读工具证据/);
});

test('mixed listed and unlisted calls get exactly one result each and only the listed call executes', async () => {
  const requests = [], executed = []; let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async (url, init) => {
    requests.push(JSON.parse(init.body));
    return response([++count === 1 ? toolTurn([{ id: 'known', name: 'workspace_read' }, { id: 'unknown', name: 'missing_tool' }]) : text('done')]);
  } });
  runtime.bindHost({ async mcp(token, conversation, method, params) {
    if (method === 'tools/list') return { tools: [{ name: 'workspace_read' }] };
    executed.push(params.name); return { content: [{ type: 'text', text: 'file content' }] };
  } });
  const active = await session(runtime); await runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  assert.deepEqual(executed, ['workspace_read']); assertPaired(requests[1].messages);
  assert.match(requests[1].messages.find(message => message.tool_call_id === 'unknown').content, /host_tool_permission_denied/);
});

test('invalid JSON arguments are returned as tool errors and never replaced with an executable empty object', async () => {
  const requests = []; let count = 0, executed = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async (url, init) => {
    requests.push(JSON.parse(init.body));
    return response([++count === 1 ? toolTurn([{ id: 'bad-json', name: 'workspace_write', arguments: '{"path":' }]) : text('could not write')]);
  } });
  runtime.bindHost({ async mcp(token, conversation, method) { if (method === 'tools/list') return { tools: [{ name: 'workspace_write' }] }; executed++; return {}; } });
  const active = await session(runtime); await runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  assert.equal(executed, 0); assertPaired(requests[1].messages);
  assert.match(requests[1].messages.find(message => message.tool_call_id === 'bad-json').content, /host_tool_input_invalid/);
});

test('declined unavailable tools remain paired when the next user turn is sent', async () => {
  const requests = []; let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async (url, init) => { requests.push(JSON.parse(init.body)); return response([++count === 1 ? toolTurn([{ id: 'unavailable', name: 'missing_tool' }]) : text('next answer')]); } });
  runtime.bindHost({ async mcp() { return { tools: [] }; }, permissionOffer() { return { offers: [{ appId: 'fixture' }] }; }, requestAuthorization() { return false; } });
  const active = await session(runtime); await runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  await runtime.prompt({ sessionId: active.sessionId, prompt: 'next' });
  assertPaired(requests[1].messages);
});

test('cancelling an unavailable-tool authorization wait finishes without requiring a late grant', async () => {
  const requested = deferred(), never = deferred();
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async () => response([toolTurn([{ id: 'unavailable', name: 'missing_tool' }])]) });
  runtime.bindHost({ async mcp() { return { tools: [] }; }, permissionOffer() { return { offers: [{ appId: 'fixture' }] }; }, requestAuthorization() { requested.resolve(); return never.promise; } });
  const active = await session(runtime); const work = runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  await requested.promise; await runtime.cancel(active.sessionId);
  const result = await work; assert.equal(result.stopReason, 'cancelled');
  assertPaired(runtime.sessions.get(active.sessionId).messages);
});

test('a steer accepted during a final model response gets another response even on the last budgeted request', async () => {
  const entered = deferred(), release = deferred(), queue = [], requests = []; let count = 0;
  const runtime = new OpenAiCompatibleRuntime({ config, maxSteps: 1, fetch: async (url, init) => {
    requests.push(JSON.parse(init.body)); if (++count === 1) { entered.resolve(); await release.promise; }
    return response([text(count === 1 ? 'old answer' : 'corrected answer')]);
  } });
  runtime.bindHost({ takeSteer() { return queue.splice(0); }, async mcp() { return { tools: [] }; } });
  const active = await session(runtime); const work = runtime.prompt({ sessionId: active.sessionId, prompt: 'original task' });
  await entered.promise; queue.push('use the corrected requirement'); release.resolve();
  const result = await work; assert.equal(result.stopReason, 'end_turn'); assert.equal(requests.length, 2); assert.equal(queue.length, 0);
  assert(requests[1].messages.some(message => message.role === 'user' && message.content.includes('use the corrected requirement')));
});

test('output-token truncation never executes a partially generated tool call', async () => {
  let executions = 0;
  const chunk = toolTurn([{ id: 'partial', name: 'workspace_write', arguments: '{"path":' }]); chunk.choices[0].finish_reason = 'length';
  const runtime = new OpenAiCompatibleRuntime({ config, fetch: async () => response([chunk]) });
  runtime.bindHost({ async mcp(token, conversation, method) { if (method === 'tools/list') return { tools: [{ name: 'workspace_write' }] }; executions++; return {}; } });
  const active = await session(runtime); const result = await runtime.prompt({ sessionId: active.sessionId, prompt: 'hello' });
  assert.equal(result.stopReason, 'max_tokens'); assert.equal(executions, 0); assertPaired(runtime.sessions.get(active.sessionId).messages);
});

test('HTTP errors distinguish invalid format, quota, rate limit, authentication and outages without exposing response bodies',()=>{
  const codes={400:'request_invalid',401:'unauthorized',402:'quota_exhausted',403:'unauthorized',404:'model_or_endpoint_missing',422:'request_invalid',429:'rate_limited',500:'service_unavailable',503:'service_unavailable'};
  for(const [status,suffix] of Object.entries(codes)){
    const error=providerHttpError(Number(status));assert.equal(error.code,'host_provider_'+suffix);
    assert.equal(error.details.httpStatus,Number(status));assert.equal(error.details.outcomeUnknown,false);
  }
});
