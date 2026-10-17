import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

test('ensureProjectByExternalId maps desktop workspace stably and updates name', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ovi-extid-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const host = new AgentHost({ dataDir: dir, runtimeSelection: 'none' });
  t.after(() => host.close());
  const a = host.store.ensureProjectByExternalId('42', '中国移动历史资产');
  const b = host.store.ensureProjectByExternalId('42', '中国移动历史资产（更新）');
  assert.equal(a.id, b.id);
  assert.equal(b.externalId, '42');
  assert.match(b.name, /更新/);
  const other = host.store.ensureProjectByExternalId('99', '其他');
  assert.notEqual(other.id, a.id);
});

test('POST /api/projects ensureByExternalId is available over HTTP', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'ovi-extid-http-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const host = new AgentHost({ dataDir: dir, runtimeSelection: 'none' });
  const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); });
  const res = await fetch(`${web.origin}/api/projects`, {
    method: 'POST',
    headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'Desk', ensureByExternalId: 'desk-7' }),
  });
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.externalId, 'desk-7');
  const again = await (await fetch(`${web.origin}/api/projects`, {
    method: 'POST',
    headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'Desk2', ensureByExternalId: 'desk-7' }),
  })).json();
  assert.equal(again.id, body.id);
});

test('config center four sections and aliases are wired in App.vue', async () => {
  const root = fileURLToPath(new URL('../../..', import.meta.url));
  const center = await readFile(join(root, 'src/features/configuration/configCenter.ts'), 'utf8');
  assert.match(center, /runtime-model/);
  assert.match(center, /data-permissions/);
  assert.match(center, /"agent-runtime": "runtime-model"/);
  assert.match(center, /plugins: "apps"/);
  const app = await readFile(join(root, 'src/App.vue'), 'utf8');
  assert.match(app, /config-nav-runtime-model/);
  assert.match(app, /ConfigDataPermissionsSection/);
  assert.match(app, /ConfigInstalledAppsSection/);
  assert.match(app, /ConfigAdvancedSection/);
  assert.doesNotMatch(app, /configSection==='agent-runtime'/);
});

test('ConfigDialog labels legacy models as advanced / scan-only', async () => {
  const root = fileURLToPath(new URL('../../..', import.meta.url));
  const dialog = await readFile(join(root, 'src/features/configuration/components/ConfigDialog.vue'), 'utf8');
  assert.match(dialog, /扫描 \/ 旧版对话模型（高级）|Scan \/ legacy chat models/);
  assert.match(dialog, /configProfileDefaults/);
});
