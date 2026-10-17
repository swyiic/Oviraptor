import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AppProcess } from '../src/app-process.mjs';

const peer = fileURLToPath(new URL('./fixtures/native-protocol-peer.mjs', import.meta.url));
async function fixture(t, mode) {
  const root = await mkdtemp(join(tmpdir(), 'native-owner-peer-'));
  const manifest = { schemaVersion: 2, id: 'example.native', version: '2.0.0', entry: 'bin/worker', dataSchemaVersion: 1,
    runtime: { platform: process.platform, arch: process.arch }, tools: [], actions: [{ name: 'read' }], pages: [{ id: 'main' }] };
  await writeFile(join(root, 'manifest.json'), JSON.stringify(manifest));
  const events = [], launches = [], kills = [];
  const worker = new AppProcess({ id: manifest.id, version: manifest.version, path: root, manifest }, join(root, 'data'), {
    timeoutMs: mode === 'timeout' ? 300 : 3000,
    onActivity: event => events.push(event),
    spawn(executable, args, options) {
      launches.push({ executable, args, options });
      const child = spawn(process.execPath, [peer, root, mode], options);
      const kill = child.kill.bind(child);
      child.kill = (...args) => { kills.push(args); return kill(...args); };
      return child;
    },
  });
  t.after(async () => {
    // Only the inert test peer is forcibly cleaned up, after ownership assertions.
    if (worker.child?.pid && !worker.exitObserved) process.kill(worker.child.pid, 'SIGTERM');
    if (worker.exitPromise) await worker.exitPromise;
    await rm(root, { recursive: true, force: true });
  });
  return { worker, events, launches, kills };
}
test('native transport uses the declared binary contract and exposes refresh metadata only', async t => {
  const f = await fixture(t, 'normal');
  assert.deepEqual(await f.worker.action('read', {}, {}), { rawLegacyDto: 7 });
  assert(f.events.some(event => event.appId === 'example.native' && event.busy === false));
  assert(f.events.every(event => Object.keys(event).every(key => ['appId', 'busy'].includes(key))));
  assert.deepEqual(f.launches[0].args, ['--app-worker', '--data-dir', f.worker.dataDir, '--package-dir', f.worker.app.path]);
  assert.deepEqual(Object.keys(f.launches[0].options.env).sort(), ['NODE_NO_WARNINGS', 'PATH']);
  await f.worker.stop();
  assert.equal(f.worker.exitObserved, true);
  assert.equal(f.worker.nativeBusy, false);
  await f.worker.stop();
  assert.equal(f.kills.length, 1);
});
test('native busy acknowledgement preserves the live controller', async t => {
  const f = await fixture(t, 'busy'); await f.worker.start();
  await assert.rejects(f.worker.stop(), { code: 'host_app_busy' });
  assert.equal(f.worker.nativeBusy, true);
  assert.equal(f.kills.length, 0); assert.equal(f.worker.exitObserved, false);
  process.kill(f.worker.child.pid, 0);
});
test('read-only native calls do not create an activity feedback loop while background transitions remain visible', async t => {
  const f=await fixture(t,'quiet-read');await f.worker.start();
  for(let i=0;i<3;i++)await f.worker.action('read',{},{});
  assert.deepEqual(f.events.map(event=>event.busy),[false]);
  await f.worker.action('begin',{},{});await f.worker.refreshStatus();
  await f.worker.action('finish',{},{});await f.worker.refreshStatus();
  assert.deepEqual(f.events.map(event=>event.busy),[false,true,false]);
  await f.worker.stop();
});
for (const mode of ['unknown-status', 'uncertain-status', 'uncertain-string', 'outcomeUnknown', 'ownerUnresolved', 'quarantined']) {
  test(`native ${mode} cannot be advertised as idle`, async t => {
    const f = await fixture(t, mode); await f.worker.start();
    assert.equal(f.worker.nativeBusy, true);
    assert(f.events.every(event => Object.keys(event).every(key => ['appId', 'busy'].includes(key))));
  });
}
test('an exited native controller keeps unknown ownership quarantined', async t => {
  const f = await fixture(t, 'dead-owner');
  await assert.rejects(f.worker.action('read', {}, {}), { code: 'host_app_process_exited' });
  await f.worker.exitPromise;
  assert.equal(f.worker.exitObserved, true); assert.equal(f.worker.nativeBusy, true);
  await assert.rejects(f.worker.stop(), { code: 'host_app_owner_unresolved' });
  assert.equal(f.launches.length, 1); assert.equal(f.kills.length, 0);
});
test('an oversized native status frame quarantines its live owner', async t => {
  const f = await fixture(t, 'oversized-status');
  await assert.rejects(f.worker.start(), { code: 'host_app_owner_unresolved' });
  assert.equal(f.worker.nativeBusy, true);
  await assert.rejects(f.worker.stop(), { code: 'host_app_owner_unresolved' });
  assert.equal(f.kills.length, 0);
});
for (const mode of ['timeout', 'malformed', 'bad-description']) {
  test(`native ${mode} quarantines the existing owner without killing or starting a successor`, async t => {
    const f = await fixture(t, mode);
    const code = mode === 'timeout' ? 'host_app_timeout' : mode === 'malformed' ? 'host_app_protocol_invalid' : 'host_app_description_mismatch';
    await assert.rejects(f.worker.action('read', {}, {}), { code });
    assert.equal(f.kills.length, 0); assert.equal(f.worker.exitObserved, false);
    process.kill(f.worker.child.pid, 0);
    await assert.rejects(f.worker.stop(), { code: 'host_app_owner_unresolved' });
    await assert.rejects(f.worker.action('read', {}, {}));
    assert.equal(f.launches.length, 1); assert.equal(f.kills.length, 0);
  });
}
