import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { OpenAiCompatibleRuntime, providerToolName, runToolCalls, truncateToolResult } from '../src/provider.mjs';
import { compressSessionMessages } from '../src/memory.mjs';
import { toolFailureReceipt } from '../src/tool-errors.mjs';

const revision = 'a'.repeat(64);
const token = 'fixture-token-0123456789';
const path = '/Users/fixture/public-rule.json';
const url = 'HTTPS://User:p%40ss@Example.TEST:443/a%2Fb?token=fixture%2Bvalue&x=1#saved';
const selected = { token, path, url, hash: revision };
const rule = JSON.stringify({ id: 'fixture', name: '框架', condition: { type: 'body', pattern: `${url} ${path} api_key=${token} ${revision}` } });
const sha256 = createHash('sha256').update(rule).digest('hex');
let split = 150;
while ((Buffer.from(rule)[split] & 0xc0) === 0x80) split--;
const page = offset => ({ projectId: 7, taskId: 'mine', scope: 'bundled-defaults', source: 'bundled_fingerprint_resources',
  userOverridesIncluded: false, readOnly: true, catalogRevision: revision, sha256, fingerprintId: 'fixture',
  offset, totalBytes: Buffer.byteLength(rule), encoding: 'utf8-json',
  content: Buffer.from(rule).subarray(offset, offset === 0 ? split : undefined).toString('utf8'),
  nextOffset: offset === 0 ? split : null, hasMore: offset === 0 });

test('runToolCalls retains authorized result strings and selected failure reasons without field expansion', async () => {
  const calls = [{ id: 'a', name: 'authorized_evidence' }, { id: 'b', name: 'authorized_failure' }];
  const results = await runToolCalls(calls, async call => {
    if (call.id === 'a') return { structuredContent: selected, content: [{ type: 'text', text: JSON.stringify(selected) }] };
    throw Object.assign(Error('unselected-error-message'), { code: 'host_fixture_failed', arbitrary: 'unselected-os-property',
      details: { reason: JSON.stringify(selected), phase: 'not_started', outcomeUnknown: false, operationId: 'fixture-op', privateField: 'unselected' } });
  });
  assert.deepEqual(JSON.parse(results[0].content), { structuredContent: selected });
  const failure = JSON.parse(results[1].content);
  assert.equal(failure.reason, JSON.stringify(selected));
  assert.equal(failure.operationId, 'fixture-op');
  assert.equal(failure.phase, 'not_started');
  assert.ok(!results[1].content.includes('unselected'));
  const receipt = toolFailureReceipt({ code: 'ECONNRESET', message: 'OS exception', details: { reason: token.repeat(100), anything: 'not-selected' } });
  assert.equal(receipt.error, 'host_tool_failed');
  assert.equal(receipt.reason.length, 1000);
  assert.ok(!JSON.stringify(receipt).includes('not-selected'));
});

test('tool-result truncation preserves retained UTF8 bytes and reports clipping', () => {
  const raw = `${JSON.stringify(selected)} 汉字${'x'.repeat(3000)} END:${revision}`;
  assert.equal(truncateToolResult(raw, 10000).text, raw);
  const cut = truncateToolResult(raw, 1024);
  assert.equal(cut.truncated, true);
  assert.equal(cut.totalBytes, Buffer.byteLength(raw));
  assert.ok(Buffer.byteLength(cut.text) <= 1024);
  assert.ok(cut.text.startsWith(JSON.stringify(selected)));
  assert.ok(cut.text.endsWith(`END:${revision}`));
  assert.ok(!cut.text.includes('\uFFFD'));
});

test('context compression clips current tool bodies as plaintext and keeps its size limit', () => {
  const raw = `${JSON.stringify(selected)} ${'汉'.repeat(10000)}`;
  const packed = compressSessionMessages([{ role: 'system', content: 'fixture' }, { role: 'user', content: 'read saved evidence' },
    { role: 'assistant', content: null, tool_calls: [{ id: 'a', type: 'function', function: { name: 'fixture', arguments: '{}' } }] },
    { role: 'tool', tool_call_id: 'a', content: raw }], { maxTotalBytes: 5000 });
  const saved = packed.messages.find(message => message.role === 'tool');
  assert.ok(saved.content.startsWith(JSON.stringify(selected)));
  assert.ok(saved.content.includes('host compressed'));
  assert.ok(Buffer.byteLength(JSON.stringify(packed.messages)) <= 5000);
});

function response(api, call, index) {
  const item = call && { type: 'function_call', call_id: `call-${index}`, name: providerToolName(call.name), arguments: JSON.stringify(call.args) };
  if (api === 'responses') return new Response(`event: response.completed\ndata: ${JSON.stringify({ response: { id: `r-${index}`, status: 'completed', output: item ? [item] : [{ type: 'message', content: [{ type: 'output_text', text: '读取完成。' }] }] } })}\n\n`);
  const delta = item ? { tool_calls: [{ index: 0, id: item.call_id, type: 'function', function: { name: item.name, arguments: item.arguments } }] } : { content: '读取完成。' };
  return new Response(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: item ? 'tool_calls' : 'stop' }] })}\n\ndata: [DONE]\n\n`);
}
const outputs = (api, body) => api === 'responses' ? body.input.filter(item => item.type === 'function_call_output').map(item => JSON.parse(item.output))
  : body.messages.filter(item => item.role === 'tool').map(item => JSON.parse(item.content));
const argumentsIn = (api, body) => api === 'responses' ? body.input.filter(item => item.type === 'function_call').map(item => JSON.parse(item.arguments))
  : body.messages.flatMap(item => item.tool_calls ?? []).map(item => JSON.parse(item.function.arguments));

for (const api of ['chat', 'responses']) test(`${api}: actual tool runner/model messages preserve revision, content, SHA and authorized evidence`, async () => {
  const search = 'org.oviraptor.scanning::scan_fingerprint_search', read = 'org.oviraptor.scanning::scan_fingerprint_read';
  const evidence = 'fixture_authorized_evidence';
  const names = [search, read, evidence];
  const listed = names.map(name => ({ name, inputSchema: { type: 'object' } }));
  const bodies = [], executed = [];
  const runtime = new OpenAiCompatibleRuntime({ config: { authentication: 'none', provider: api === 'responses' ? 'xai' : 'mock', baseUrl: 'http://127.0.0.1:1', model: 'mock' }, maxSteps: 5,
    fetch: async (_url, init) => {
      const body = JSON.parse(init.body); bodies.push(body);
      const saved = outputs(api, body).map(value => value.structuredContent);
      if (bodies.length === 1) return response(api, { name: search, args: { taskId: 'mine', query: 'fixture' } }, 1);
      if (bodies.length === 2) {
        assert.equal(saved[0].catalogRevision, revision);
        return response(api, { name: read, args: { taskId: 'mine', fingerprintId: 'fixture', catalogRevision: saved[0].catalogRevision } }, 2);
      }
      if (bodies.length === 3) {
        assert.equal(saved[1].sha256, sha256);
        assert.equal(saved[1].content, page(0).content);
        assert.equal(argumentsIn(api, body).at(-1).catalogRevision, revision);
        return response(api, { name: read, args: { taskId: 'mine', fingerprintId: 'fixture', offset: saved[1].nextOffset, catalogRevision: saved[1].catalogRevision } }, 3);
      }
      if (bodies.length === 4) {
        const content = saved[1].content + saved[2].content;
        assert.equal(content, rule);
        assert.equal(createHash('sha256').update(content).digest('hex'), saved[2].sha256);
        return response(api, { name: evidence, args: { lease: revision, token, path, url } }, 4);
      }
      assert.deepEqual(saved[3], selected);
      assert.deepEqual(argumentsIn(api, body).at(-1), { lease: revision, token, path, url });
      return response(api, null, 5);
    } });
  runtime.bindHost({ mcp: async (_token, _conversation, method, params) => {
    if (method === 'tools/list') return { tools: listed };
    executed.push(params);
    const value = params.name === search ? { ...page(0), fingerprints: [{ id: 'fixture' }], content: undefined }
      : params.name === read ? page(params.arguments.offset ?? 0) : selected;
    return { content: [{ type: 'text', text: JSON.stringify(value) }], structuredContent: value };
  } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'fixture' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'conversation' }] }] });
  try {
    await runtime.prompt({ sessionId, prompt: 'Review saved references', userText: 'Review saved references' });
    assert.equal(bodies.length, 5);
    assert.equal(executed.length, 4);
    assert.equal(executed[2].arguments.catalogRevision, revision);
    assert.equal(executed[2].arguments.offset, split);
  } finally { await runtime.close(); }
});
