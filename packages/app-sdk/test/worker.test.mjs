import test from 'node:test';
import assert from 'node:assert/strict';
import { PassThrough } from 'node:stream';
import { join } from 'node:path';
import { writeFile } from 'node:fs/promises';
import { createWorker, startStdioWorker, MAX_MESSAGE_BYTES } from '../src/index.mjs';
import { manifest, context, start, temporary, rpcProcess } from './helpers.mjs';
const request = (method, params, id = 1) => ({ jsonrpc: '2.0', id, method, params });
const definition = overrides => ({ manifest: manifest(), tools: { echo: async args => ({ content: [{ type: 'text', text: args.text }] }) }, ...overrides });
test('worker enforces lifecycle context schemas local names and supports project switching per call', async () => {
  const worker = createWorker(definition());
  assert.equal((await worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'x' }, context: context() }))).error.data.code, 'app_not_running');
  assert.equal((await worker.handle(request('describe', { hostApiVersion: '1.0.0' }))).result.appId, 'org.oviraptor.test');
  assert.equal((await worker.handle(request('describe', { hostApiVersion: '2.0.0' }))).error.data.code, 'app_host_api_incompatible');
  assert.equal((await worker.handle(request('lifecycle/start', start('/tmp/test')))).result.started, true);
  for (const project of [1, 2]) assert.deepEqual((await worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'x' }, context: context('op', project) }))).result.content, [{ type: 'text', text: 'x' }]);
  for (const params of [{ name: 'echo', arguments: { text: 'x', undeclared: true }, context: context() }, { name: 'echo', arguments: { text: 'x' }, context: { ...context(), hostBoundProjectId: 0 } }, { name: 'org.other::echo', arguments: { text: 'x' }, context: context() }]) assert.ok((await worker.handle(request('tool/call', params))).error);
  assert.deepEqual((await worker.handle(request('lifecycle/stop', {}))).result, { busy: false });
  assert.equal((await worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'x' }, context: context() }))).error.data.code, 'app_not_running');
});
test('stop refuses while a real asynchronous tool is active and does not invoke its stop hook', async () => {
  let finish; let stopped = 0;
  const worker = createWorker(definition({ tools: { echo: async () => { await new Promise(resolve => { finish = resolve; }); return { content: [] }; } }, onStop: async () => { stopped++; return { busy: false }; } }));
  await worker.handle(request('lifecycle/start', start('/tmp/test')));
  const running = worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'wait' }, context: context() }, 2));
  assert.equal(worker.activeCalls, 1);
  assert.deepEqual((await worker.handle(request('lifecycle/stop', {}, 3))).result, { busy: true });
  assert.equal(stopped, 0);
  assert.equal((await worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'wait' }, context: context() }, 2))).error.data.code, 'app_request_duplicate');
  finish(); assert.ok((await running).result);
  assert.deepEqual((await worker.handle(request('lifecycle/stop', {}, 4))).result, { busy: false }); assert.equal(stopped, 1);
});
test('application tasks may veto stop and raw handler exceptions never enter protocol diagnostics', async () => {
  let busy = true;
  const worker = createWorker(definition({ onStop: async () => ({ busy }), tools: { echo: async () => { throw new Error('secret-token/raw-path'); } } }));
  await worker.handle(request('lifecycle/start', start('/tmp/test')));
  const failed = await worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'x' }, context: context() }));
  assert.equal(failed.error.data.code, 'app_internal_error'); assert.ok(!JSON.stringify(failed).includes('secret-token'));
  assert.deepEqual((await worker.handle(request('lifecycle/stop', {}))).result, { busy: true });
  busy = false; assert.deepEqual((await worker.handle(request('lifecycle/stop', {}))).result, { busy: false });
});
test('worker rejects oversized and non-JSON tool output before transport', async () => {
  for (const value of [{ content: [{ type: 'text', text: 'a'.repeat(65536) }] }, { content: [], structuredContent: { invalid: undefined } }, { content: [{ type: 'html', text: 'x' }] }, { content: [], surprise: true }]) {
    const worker = createWorker(definition({ tools: { echo: async () => value } }));
    await worker.handle(request('lifecycle/start', start('/tmp/test')));
    assert.equal((await worker.handle(request('tool/call', { name: 'echo', arguments: { text: 'x' }, context: context() }))).error.data.code, 'app_result_invalid');
  }
});
test('stdio rejects malformed invalid UTF-8 oversized and unterminated lines and recovers without raw logs', async () => {
  const input = new PassThrough(); const output = new PassThrough(); const diagnostics = new PassThrough(); let bytes = ''; let errors = '';
  output.on('data', part => { bytes += part; }); diagnostics.on('data', part => { errors += part; });
  const controller = startStdioWorker(definition(), { input, output, diagnostics });
  input.write('secret-not-json\n'); input.write(Buffer.from([0xff, 10])); input.write(`${'a'.repeat(MAX_MESSAGE_BYTES + 1)}\n`);
  input.write(`${JSON.stringify(request('describe', { hostApiVersion: '1.0.0' }, 9))}\n`); input.end('incomplete'); await controller.closed;
  const responses = bytes.trim().split('\n').map(line => JSON.parse(line));
  assert.equal(responses.length, 5); assert.equal(responses.filter(row => row.error?.data.code === 'app_parse_error').length, 3);
  assert.equal(responses.filter(row => row.error?.data.code === 'app_message_too_large').length, 1);
  assert.equal(responses.find(row => row.id === 9).result.protocolVersion, 1); assert.ok(!bytes.includes('secret-not-json')); assert.equal(errors, '');
});
test('actual Node child speaks only NDJSON and stop stays responsive during an active tool', async t => {
  const directory = await temporary(t);
  const workerUrl = new URL('../src/worker.mjs', import.meta.url).href;
  const source = `import { startStdioWorker } from ${JSON.stringify(workerUrl)};\nconst manifest=${JSON.stringify(manifest())};\nawait startStdioWorker({manifest,tools:{echo:async args=>{await new Promise(resolve=>setTimeout(resolve,150));return {content:[{type:'text',text:args.text}]};}}}).closed;\n`;
  await writeFile(join(directory, 'worker.mjs'), source);
  const client = rpcProcess(directory); t.after(() => client.kill());
  assert.equal((await client.request('lifecycle/start', start(directory))).result.started, true);
  const running = client.request('tool/call', { name: 'echo', arguments: { text: 'actual' }, context: context() });
  assert.deepEqual((await client.request('lifecycle/stop', {})).result, { busy: true });
  assert.equal((await running).result.content[0].text, 'actual');
  assert.deepEqual((await client.request('lifecycle/stop', {})).result, { busy: false });
  assert.equal((await client.end()).code, 0); assert.deepEqual(client.unexpected, []); assert.equal(client.stderr, '');
});
