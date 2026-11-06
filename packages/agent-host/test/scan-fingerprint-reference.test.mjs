import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { validateToolArguments } from '../../app-sdk/src/schema.mjs';
import { assertPhase1Surface, createThinScanningApplication } from '../../../apps/scanning/thin-service.mjs';
import { ScanningTasks, SCANNING_TOOLS } from '../src/scanning-tasks.mjs';
import { isDataReadTool } from '../src/quality-prompt.mjs';
import { requestedBusinessTools } from '../src/task-intent-permissions.mjs';
import { OpenAiCompatibleRuntime, providerToolName } from '../src/provider.mjs';

const manifest = JSON.parse(readFileSync(new URL('../../../apps/scanning/app.package.json', import.meta.url))).manifest;
const names = ['scan_fingerprint_search', 'scan_fingerprint_read'];
const TOKEN = 'a'.repeat(64);
const revision = 'b'.repeat(64);
const schema = name => manifest.tools.find(tool => tool.name === name).inputSchema;
const store = { project: id => ({ id, externalId: id === 11 ? '7' : '8' }) };

test('public fingerprint references are bounded Host proxies with no direct worker execution', async () => {
  assertPhase1Surface(manifest);
  const app = createThinScanningApplication(manifest);
  for (const name of names) {
    assert.equal(manifest.tools.find(tool => tool.name === name).permission, 'investigation.read');
    assert.ok(isDataReadTool(`org.oviraptor.scanning::${name}`));
    await assert.rejects(app.tools[name](), /host_executor_required/);
  }
  assert.equal(SCANNING_TOOLS.get(names[0]), 'fingerprint_search');
  assert.equal(SCANNING_TOOLS.get(names[1]), 'fingerprint_read');
  assert.equal(SCANNING_TOOLS.get('scan_status'), 'status');
  assert.equal(SCANNING_TOOLS.get('scan_start'), 'start');
});

test('schemas reject missing ownership, caller paths, unknown keys and wrong types before dispatch', () => {
  for (const args of [{}, { query: 'eos' }, { taskId: 's', query: '' },
    { taskId: 's', query: 'x'.repeat(161) }, { taskId: 's', query: null },
    { taskId: 's', query: 'eos', limit: 21 }, { taskId: 's', query: 'eos', limit: 1.2 },
    { taskId: 's', query: 'eos', offset: -1 }, { taskId: 's', query: 'eos', projectId: 8 },
    { taskId: 's', query: 'eos', file: '/private/rules' }, { taskId: 's', query: 'eos', catalogRevision: '' }]) {
    assert.throws(() => validateToolArguments(schema(names[0]), args), { code: 'app_arguments_invalid' });
  }
  for (const args of [{ taskId: 's' }, { taskId: 's', fingerprintId: 'eos', limit: 4097 },
    { taskId: 's', fingerprintId: 'eos', offset: null }, { taskId: 's', fingerprintId: 'eos', includeUserRules: true }]) {
    assert.throws(() => validateToolArguments(schema(names[1]), args), { code: 'app_arguments_invalid' });
  }
  assert.doesNotThrow(() => validateToolArguments(schema(names[0]), { taskId: 's', query: '普元 EOS', limit: 20 }));
  assert.doesNotThrow(() => validateToolArguments(schema(names[1]), { taskId: 's', fingerprintId: 'eos', offset: 12, limit: 4096, catalogRevision: revision }));
});

test('intent offers only installed read tools and does not select or grant a Skill', () => {
  const applications = [{ id: manifest.id, enabled: true, manifest }];
  const requested = requestedBusinessTools('查询当前扫描任务的 EOS 指纹', { applications, scanningAvailable: true });
  for (const name of names) assert.ok(requested.includes(`${manifest.id}::${name}`));
  assert.ok(requested.every(name => !/skills_|scan_(start|resume|command)$/.test(name)));
  assert.deepEqual(requestedBusinessTools('查询 EOS 指纹', { applications: [], scanningAvailable: true }), []);
});

async function bridge(handler) {
  const calls = [];
  const server = createServer((request, response) => {
    let raw = '';
    request.on('data', chunk => { raw += chunk; });
    request.on('end', () => {
      assert.equal(request.url, '/scan-task');
      assert.equal(request.headers.authorization, `Bearer ${TOKEN}`);
      const body = JSON.parse(raw); calls.push(body);
      response.end(JSON.stringify(handler(body)));
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { calls, origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise(resolve => server.close(resolve)) };
}

test('read references cross only the trusted project bridge and preserve pagination evidence', async () => {
  const mock = await bridge(body => ({ ok: true, result: {
    projectId: body.projectId, taskId: body.args.taskId, scope: 'bundled-defaults',
    userOverridesIncluded: false, targetRequests: 0, catalogRevision: revision,
    ...(body.action === 'fingerprint_search' ? { totalCatalog: 3, totalMatches: 2, returned: 1, nextOffset: 1,
      fingerprints: [{ id: 'eos', name: 'EOS', signalSummary: { mode: 'all', conditionLeaves: 2 } }] }
      : { fingerprintId: 'eos', content: '{"id":', totalBytes: 30, offset: body.args.offset, nextOffset: 6, sha256: 'c'.repeat(64) }),
  } }));
  try {
    const tasks = new ScanningTasks({ origin: mock.origin, token: TOKEN });
    const search = await tasks.call(store, 11, names[0], { taskId: 'mine', query: 'EOS', limit: 1 }, 'reference-1');
    const read = await tasks.call(store, 11, names[1], { taskId: 'mine', fingerprintId: 'eos', limit: 6, offset: 0, catalogRevision: revision }, 'reference-2');
    assert.deepEqual(mock.calls.map(call => call.action), ['fingerprint_search', 'fingerprint_read']);
    assert.ok(mock.calls.every(call => call.projectId === 7 && call.args.taskId === 'mine'));
    assert.equal(search.structuredContent.projectId, 11);
    assert.equal(search.structuredContent.desktopProjectId, 7);
    assert.equal(search.structuredContent.totalMatches, 2);
    assert.equal(read.structuredContent.nextOffset, 6);
    assert.equal(read.structuredContent.sha256, 'c'.repeat(64));
    assert.ok([search, read].every(result => result.structuredContent.targetRequests === 0));
  } finally { await mock.close(); }
});

test('foreign project payloads are refused and read failures never claim an unknown execution', async () => {
  let reject = false;
  const mock = await bridge(() => reject ? { ok: false, error: 'scan_fingerprint_catalog_changed' }
    : { ok: true, result: { projectId: 8 } });
  try {
    const tasks = new ScanningTasks({ origin: mock.origin, token: TOKEN });
    await assert.rejects(tasks.call(store, 11, names[0], { taskId: 'mine', query: 'eos' }, 'foreign'), { code: 'host_scan_project_binding_changed' });
    reject = true;
    for (const name of names) await assert.rejects(tasks.call(store, 11, name, { taskId: 'mine' }, 'rejected'), error => error.details.outcomeUnknown === false);
  } finally { await mock.close(); }
  const offline = new ScanningTasks({ origin: mock.origin, token: TOKEN });
  for (const name of names) await assert.rejects(offline.call(store, 11, name, { taskId: 'mine' }, 'offline'), error => error.details.outcomeUnknown === false);
});

function sse(call) {
  const delta = call ? { tool_calls: [{ index: 0, id: 'call-1', type: 'function', function: { name: providerToolName(call), arguments: '{"taskId":"mine"}' } }] } : { content: '已读取任务状态及参考范围。' };
  return new Response(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: call ? 'tool_calls' : 'stop' }] })}\n\ndata: [DONE]\n\n`, { headers: { 'content-type': 'text/event-stream' } });
}

for (const skillStillLeased of [true, false]) test(`scan references follow the fresh grant list after status; Skill lease ${skillStillLeased ? 'active' : 'revoked'}`, async () => {
  const listed = ['scan_status', ...names].map(name => ({ name: `${manifest.id}::${name}`, inputSchema: schema(name) }));
  const selectedSkill = { name: 'skills_read_resource', inputSchema: { type: 'object', properties: {}, additionalProperties: false } };
  const bodies = [];
  let listings = 0;
  const runtime = new OpenAiCompatibleRuntime({ config: { authentication: 'none', baseUrl: 'http://127.0.0.1:1', model: 'mock' }, maxSteps: 3,
    fetch: async (_url, init) => { bodies.push(JSON.parse(init.body)); return sse(bodies.length === 1 ? listed[0].name : null); } });
  runtime.bindHost({ mcp: async (_token, _conversation, method) => method === 'tools/list' ? { tools: [...listed, ...(++listings === 1 || skillStillLeased ? [selectedSkill] : [])] }
    : { content: [{ type: 'text', text: JSON.stringify({ task: { id: 'mine', status: 'running' } }) }] } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'test' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'conversation' }] }] });
  try {
    await runtime.prompt({ sessionId, prompt: '查看当前扫描任务状态', userText: '查看当前扫描任务状态' });
    assert.ok(bodies.length >= 2);
    const second = bodies[1].tools.map(tool => tool.function.name);
    for (const name of names.map(name => `${manifest.id}::${name}`)) assert.ok(second.includes(providerToolName(name)), name);
    assert.equal(second.includes(providerToolName(selectedSkill.name)), skillStillLeased);
    assert.ok(!second.includes(providerToolName(listed[0].name)), 'the completed status read remains closed');
  } finally { await runtime.close(); }
});

test('references do not interrupt the mandatory status read after scan control', async () => {
  const listed = ['scan_status', 'scan_start', ...names].map(name => ({ name: `${manifest.id}::${name}`, inputSchema: schema(name) }));
  listed.push({ name: 'skills_read_resource', inputSchema: { type: 'object', properties: {}, additionalProperties: false } });
  const bodies = [];
  const runtime = new OpenAiCompatibleRuntime({ config: { authentication: 'none', baseUrl: 'http://127.0.0.1:1', model: 'mock' }, maxSteps: 2,
    fetch: async (_url, init) => { bodies.push(JSON.parse(init.body)); return sse(bodies.length === 1 ? `${manifest.id}::scan_start` : null); } });
  runtime.bindHost({ mcp: async (_token, _conversation, method) => method === 'tools/list' ? { tools: listed }
    : { content: [{ type: 'text', text: JSON.stringify({ task: { id: 'mine', status: 'running' } }) }] } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'test' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'conversation' }] }] });
  try {
    await runtime.prompt({ sessionId, prompt: '立即启动扫描任务 mine', userText: '立即启动扫描任务 mine' });
    assert.ok(bodies.length >= 2);
    assert.deepEqual(bodies[1].tools.map(tool => tool.function.name), [providerToolName(`${manifest.id}::scan_status`)]);
  } finally { await runtime.close(); }
});
