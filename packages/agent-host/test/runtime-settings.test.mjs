import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-runtime-settings-')); let starts = 0, closes = 0, constructions = 0;
  const runtimeFactory = () => { constructions++; return { command: 'synthetic-runtime', status: { pid: null }, async start() { starts++; throw Error('No model transport permitted in this settings test'); }, async close() { closes++; } }; };
  const host = new AgentHost({ dataDir: directory, runtimeFactory });
  const web = await serveHost(host);
  t.after(async () => { await host.close(); await web.close(); await rm(directory, { recursive: true, force: true }); });
  const post = (path, body) => fetch(web.origin + path, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  return { host, web, post, directory, counters: () => ({ starts, closes, constructions }) };
}

test('selecting an official runtime is distinct from model provider configuration and performs no transport', async t => {
  const f = await fixture(t); const before = f.host.settings();
  assert.equal(before.runtime.id, 'none'); assert.equal(before.modelProvider.availableHere, false);
  assert.equal((await f.post('/api/settings', { runtime: 'grok-acp' })).status, 200);
  assert.equal(f.host.settings().runtime.id, 'grok-acp'); assert.notEqual(f.host.runtimeIdentity(), before.runtime.identity);
  assert.deepEqual(f.counters(), { starts: 0, closes: 0, constructions: 1 });
  assert.equal((await f.post('/api/settings', { runtime: 'none' })).status, 200);
  assert.deepEqual(f.counters(), { starts: 0, closes: 1, constructions: 1 });
});

test('application disclosure binds the exact recipient shown to the UI and is not revived by switching back', async t => {
  const f = await fixture(t); const p = f.host.store.createProject('Synthetic disclosure project');
  const sourceDirectory = join(root, 'examples/echo-app'); const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const outputDirectory = join(f.directory, 'package'); await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
  await f.host.apps.install(outputDirectory); await f.host.apps.setEnabled(spec.manifest.id, true);
  const displayed = f.host.runtimeIdentity();
  await f.host.setRuntime('grok-acp');
  const endpoint = `/api/apps/${spec.manifest.id}/grants`;
  const stale = await f.post(endpoint, { projectId: p.id, permissions: ['echo.read'], disclosureAccepted: true, expectedRuntimeIdentity: displayed });
  assert.equal(stale.status, 409); assert.equal((await stale.json()).error, 'host_runtime_identity_changed');
  assert.deepEqual(f.host.store.grant(spec.manifest.id, p.id).permissions, []);
  const current = f.host.runtimeIdentity();
  assert.equal((await f.post(endpoint, { projectId: p.id, permissions: ['echo.read'], disclosureAccepted: true, expectedRuntimeIdentity: current })).status, 200);
  assert.equal(f.host.apps.capabilities(p.id, { runtimeIdentity: current }).length, 1);
  await f.host.setRuntime('none'); await f.host.setRuntime('grok-acp');
  assert.notEqual(f.host.runtimeIdentity(), current);
  assert.deepEqual(f.host.apps.capabilities(p.id, { runtimeIdentity: f.host.runtimeIdentity() }), []);
  assert.deepEqual(f.host.store.grant(spec.manifest.id, p.id).permissions, ['echo.read'], 'Local permission remains distinct from renewed runtime disclosure');
});
