import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const empty = { type: 'object', properties: {}, additionalProperties: false };
async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'native-action-scope-'));
  const source = join(root, 'source'); await mkdir(join(source, 'bin'), { recursive: true });
  await copyFile('/usr/bin/true', join(source, 'bin/worker')); await writeFile(join(source, 'page.html'), '<!doctype html>');
  const manifest = { schemaVersion: 2, id: 'example.native', version: '2.0.0', hostApi: '^2.0.0', runtime: { kind: 'native', platform: process.platform, arch: process.arch }, entry: 'bin/worker', dataSchemaVersion: 1,
    tools: [{ name: 'read', description: 'Read one project', inputSchema: empty, permission: 'read' }], pages: [{ id: 'main', title: 'Application', path: 'page.html' }],
    actions: [{ name: 'configure', description: 'Configure the application', inputSchema: empty, permission: 'configure', scope: 'application' }, { name: 'list', description: 'List this project', inputSchema: empty, permission: 'read', scope: 'project' }],
    permissions: [{ id: 'read', description: 'Read project data' }, { id: 'configure', description: 'Manage application configuration' }] };
  const built = await createDirectoryPackage({ sourceDirectory: source, outputDirectory: join(root, 'package'), manifest, files: ['bin/worker', 'page.html'] });
  const store = new HostStore(join(root, 'host.sqlite')), calls = [];
  const worker = { active: 0, stopped: false, start: async () => {}, stop: async () => {}, call: async (name, args, context) => { calls.push({ name, args, context }); return { content: [] }; }, action: async (name, args, context) => { calls.push({ name, args, context }); return { originalRawDto: 7 }; } };
  const apps = new AppManager(store, join(root, 'apps'), { processFactory: () => worker });
  const one = store.createProject('One'), two = store.createProject('Two'); await apps.install(built.directory); await apps.setEnabled(manifest.id, true);
  const scope = project => { const app = store.application(manifest.id), grant = store.grant(manifest.id, project.id); return { expectedAppRevision: app.revision, expectedGrantRevision: grant.revision, operationId: 'original-operation', pageId: 'main', applicationScopeAccepted: true }; };
  const cleanup = async () => { store.close(); await rm(root, { recursive: true, force: true }); };
  return { root, store, apps, worker, calls, manifest, one, two, scope, cleanup };
}
test('native operator actions require explicit permissions and never enter model discovery', { skip: process.platform !== 'linux' }, async () => {
  const f = await fixture();
  try {
    await assert.rejects(f.apps.callAction(f.manifest.id, 'configure', f.one.id, {}, f.scope(f.one)), { code: 'host_action_permission_denied' });
    f.apps.setGrants(f.manifest.id, f.one.id, ['configure'], 0);
    assert.deepEqual(f.apps.capabilities(f.one.id), []);
    assert.deepEqual(await f.apps.callAction(f.manifest.id, 'configure', f.one.id, {}, f.scope(f.one)), { originalRawDto: 7 });
    assert.equal(f.calls.length, 1);
    assert.equal(f.calls[0].context.permission, 'configure');
    assert.equal(f.calls[0].context.project.name, 'One');
  } finally { await f.cleanup(); }
});
test('native calls bind the actual host project and retain old Node context compatibility separately', { skip: process.platform !== 'linux' }, async () => {
  const f = await fixture();
  try {
    f.apps.setGrants(f.manifest.id, f.two.id, ['read'], 0);
    await f.apps.callAction(f.manifest.id, 'list', f.two.id, {}, f.scope(f.two));
    assert.equal(f.calls[0].context.hostBoundProjectId, f.two.id);
    assert.equal(f.calls[0].context.project.id, f.two.id);
    assert.equal(f.calls[0].context.project.name, 'Two');
    await assert.rejects(f.apps.callAction(f.manifest.id, 'list', f.two.id, { projectId: f.one.id }, f.scope(f.two)), { code: 'app_arguments_invalid' });
    await assert.rejects(f.apps.callAction(f.manifest.id, 'list', f.one.id, {}, f.scope(f.one)), { code: 'host_action_permission_denied' });
    assert.equal(f.calls.length, 1);
  } finally { await f.cleanup(); }
});
test('native page and grant revisions are mandatory, including application-wide scope acknowledgement', { skip: process.platform !== 'linux' }, async () => {
  const f = await fixture();
  try {
    f.apps.setGrants(f.manifest.id, f.one.id, ['configure'], 0);
    for (const overrides of [{ expectedAppRevision: 0 }, { expectedGrantRevision: 0 }, { pageId: 'unregistered' }, { applicationScopeAccepted: false }]) {
      await assert.rejects(f.apps.callAction(f.manifest.id, 'configure', f.one.id, {}, { ...f.scope(f.one), ...overrides }));
    }
    assert.equal(f.calls.length, 0);
  } finally { await f.cleanup(); }
});
test('revocation while a native action is pending withholds its original result', { skip: process.platform !== 'linux' }, async () => {
  const f = await fixture();
  try {
    f.apps.setGrants(f.manifest.id, f.one.id, ['read'], 0);
    let resolve, entered;
    const started = new Promise(done => { entered = done; });
    f.worker.action = async () => { entered(); return new Promise(done => { resolve = done; }); };
    const pending = f.apps.callAction(f.manifest.id, 'list', f.one.id, {}, f.scope(f.one));
    await started; f.apps.setGrants(f.manifest.id, f.one.id, [], 1); resolve({ mustNotReachPage: true });
    await assert.rejects(pending, { code: 'host_tool_authority_changed' });
  } finally { await f.cleanup(); }
});
test('a stale native page cannot call a newer tool owner', { skip: process.platform !== 'linux' }, async () => {
  const f = await fixture();
  try {
    f.apps.setGrants(f.manifest.id, f.one.id, ['read'], 0);
    await assert.rejects(f.apps.call(f.manifest.id, 'read', f.one.id, {}, 1, 'operation', { pageId: 'main', version: '1.0.0', expectedAppRevision: 2 }), { code: 'host_app_page_unavailable' });
    assert.equal(f.calls.length, 0);
  } finally { await f.cleanup(); }
});
