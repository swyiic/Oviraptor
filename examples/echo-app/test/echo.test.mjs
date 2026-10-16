import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { createDirectoryPackage } from '../../../packages/app-sdk/src/index.mjs';
import { context, start, temporary, rpcProcess } from '../../../packages/app-sdk/test/helpers.mjs';
const sourceDirectory = fileURLToPath(new URL('../', import.meta.url));
async function setup(t) {
  const root = await temporary(t, 'oviraptor-echo-');
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const outputDirectory = join(root, 'package'); const dataDir = join(root, 'data'); await mkdir(dataDir);
  const packed = await createDirectoryPackage({ sourceDirectory, outputDirectory, ...spec });
  return { root, outputDirectory, dataDir, packed };
}
const call = (client, name, args, op, project = 1) => client.request('tool/call', { name, arguments: args, context: context(op, project) });
async function launch(t, fixture) {
  const client = rpcProcess(fixture.outputDirectory, { permission: true, dataDir: fixture.dataDir }); t.after(() => client.kill());
  const started = await client.request('lifecycle/start', start(fixture.dataDir, 'org.oviraptor.echo'));
  return { client, started };
}
test('packaged Echo process persists replays and project isolation under Node filesystem permissions', async t => {
  const fixture = await setup(t); const { client, started } = await launch(t, fixture);
  assert.equal(started.result.dataSchemaVersion, 2);
  const described = await client.request('describe', { hostApiVersion: '1.0.0' }); assert.deepEqual(described.result.tools, fixture.packed.manifest.tools);
  assert.equal((await call(client, 'echo', { text: 'hello local' }, 'op-1')).result.structuredContent.replayed, false);
  const saved = await readFile(join(fixture.dataDir, 'echo-state.json'));
  assert.equal((await call(client, 'echo', { text: 'hello local' }, 'op-1')).result.structuredContent.replayed, true);
  assert.deepEqual(await readFile(join(fixture.dataDir, 'echo-state.json')), saved);
  assert.equal((await call(client, 'echo', { text: 'changed' }, 'op-1')).error.data.code, 'app_internal_error');
  assert.deepEqual(await readFile(join(fixture.dataDir, 'echo-state.json')), saved);
  assert.equal((await call(client, 'echo', { text: 'other project' }, 'op-1', 2)).result.structuredContent.projectId, 2);
  assert.equal((await call(client, 'history', {}, 'history', 1)).result.structuredContent.total, 1);
  assert.equal((await call(client, 'history', {}, 'history', 2)).result.structuredContent.records[0].text, 'other project');
  assert.deepEqual((await client.request('lifecycle/stop', {})).result, { busy: false }); assert.equal((await client.end()).code, 0);
  const restarted = await launch(t, fixture); assert.ok(restarted.started.result);
  assert.equal((await call(restarted.client, 'echo', { text: 'hello local' }, 'op-1')).result.structuredContent.replayed, true);
  assert.equal((await call(restarted.client, 'history', {}, 'history')).result.structuredContent.total, 1);
  await restarted.client.request('lifecycle/stop', {}); assert.equal((await restarted.client.end()).code, 0);
  assert.deepEqual(client.unexpected, []); assert.equal(client.stderr, '');
});
test('Echo app owns v1 to v2 migration and preserves immutable legacy operations', async t => {
  const fixture = await setup(t);
  await writeFile(join(fixture.dataDir, 'echo-state.json'), JSON.stringify({ schemaVersion: 1, echoes: [{ projectId: 3, operationId: 'legacy', text: 'saved before upgrade' }] }));
  const { client, started } = await launch(t, fixture); assert.equal(started.result.dataSchemaVersion, 2);
  assert.deepEqual(JSON.parse(await readFile(join(fixture.dataDir, 'echo-state.json'), 'utf8')), { schemaVersion: 2, records: [{ projectId: 3, operationId: 'legacy', text: 'saved before upgrade', sequence: 1 }] });
  assert.equal((await call(client, 'echo', { text: 'saved before upgrade' }, 'legacy', 3)).result.structuredContent.replayed, true);
  await client.request('lifecycle/stop', {}); assert.equal((await client.end()).code, 0);
});
test('future or corrupt data is refused without overwriting app data or accepting tools', async t => {
  for (const data of ['{"schemaVersion":99,"records":[]}', 'invalid-json']) {
    const fixture = await setup(t); const file = join(fixture.dataDir, 'echo-state.json'); await writeFile(file, data);
    const { client, started } = await launch(t, fixture); assert.equal(started.error.data.code, 'app_internal_error');
    assert.equal(await readFile(file, 'utf8'), data);
    assert.equal((await call(client, 'echo', { text: 'bad' }, 'no-write')).error.data.code, 'app_not_running');
    await client.request('lifecycle/stop', {}); assert.equal((await client.end()).code, 0);
  }
});
test('Echo refuses overflow before writes and shows bounded history explicitly', async t => {
  const fixture = await setup(t);
  const records = Array.from({ length: 128 }, (_, i) => ({ projectId: 1, operationId: `old-${i}`, text: `saved-${i}`, sequence: i + 1 }));
  const file = join(fixture.dataDir, 'echo-state.json'); await writeFile(file, JSON.stringify({ schemaVersion: 2, records }));
  const before = await readFile(file); const { client, started } = await launch(t, fixture); assert.ok(started.result);
  assert.equal((await call(client, 'echo', { text: 'overflow' }, 'overflow')).error.data.code, 'app_internal_error');
  assert.deepEqual(await readFile(file), before);
  const history = (await call(client, 'history', {}, 'history')).result.structuredContent;
  assert.equal(history.total, 128); assert.equal(history.records.length, 16); assert.equal(history.truncated, true);
  assert.equal((await call(client, 'echo', { text: 'saved-0' }, 'old-0')).result.structuredContent.replayed, true);
  await client.request('lifecycle/stop', {}); await client.end();
});
test('Echo page is static and packaged without a repository SDK dependency', async t => {
  const fixture = await setup(t);
  const page = await readFile(join(fixture.outputDirectory, 'page.html'), 'utf8');
  assert.ok(!/<script|<iframe|<form|https?:\/\//iu.test(page)); assert.match(page, /default-src 'none'/u);
  const worker = await readFile(join(fixture.outputDirectory, 'worker.mjs'), 'utf8');
  assert.match(worker, /from '\.\/sdk\/worker\.mjs'/u); assert.ok(!worker.includes('packages/app-sdk'));
  assert.ok(fixture.packed.files.every(file => !file.path.includes('..')));
});
