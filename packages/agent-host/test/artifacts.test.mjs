import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { ArtifactDownloads } from '../src/artifacts.mjs';
import { serveHost } from '../src/server.mjs';

const input = { projectId: 1, pageId: 'main', version: '2.0.0', expectedAppRevision: 2, expectedGrantRevision: 3,
  actionName: 'read_artifact', artifactId: 'owned-export', filename: '资产.csv' };
async function fixture(t, { bytes = Buffer.alloc(600000, 0x41), now = Date.now, maxBytes } = {}) {
  const app = { id: 'example.native', enabled: true, revision: 2, version: '2.0.0', digest: 'manifest-digest', manifest: {
    schemaVersion: 2, actions: [{ name: 'read_artifact', scope: 'project', delivery: 'artifact-read', permission: 'export' }], pages: [{ id: 'main' }],
  } }, grant = { revision: 3, permissions: ['export'] }, calls = [];
  const digest = createHash('sha256').update(bytes).digest('hex');
  let mutate = value => value;
  const apps = { store: { application: id => { assert.equal(id, app.id); return app; },
    project: id => { if (id !== 1) throw Object.assign(Error(), { code: 'host_project_missing' }); return { id }; },
    grant: () => grant }, changing: new Set(), workers: new Map(),
    async callAction(id, name, projectId, args, context) {
      calls.push({ id, name, projectId, args, context });
      const nextOffset = Math.min(bytes.length, args.offset + args.maxBytes), chunk = bytes.subarray(args.offset, nextOffset);
      return mutate({ artifactId: args.artifactId, offset: args.offset, bytesBase64: chunk.toString('base64'), nextOffset,
        totalBytes: bytes.length, sha256: digest, done: nextOffset === bytes.length });
    } };
  const host = { apps, onEvent() {}, setOrigin() {} }, endpoint = await serveHost(host);
  t.after(() => endpoint.close());
  const create = async overrides => fetch(`${endpoint.origin}/api/apps/${app.id}/artifact-downloads`, { method: 'POST',
    headers: { 'x-oviraptor-session': endpoint.token, 'content-type': 'application/json' }, body: JSON.stringify({ ...input, ...overrides }) });
  return { app, grant, calls, bytes, digest, apps, endpoint, create, setMutate(value) { mutate = value; }, downloads: new ArtifactDownloads(apps, { now, ...(maxBytes === undefined ? {} : { maxBytes }) }) };
}
test('authenticated artifact delivery streams exact bytes through current scoped action reads and consumes its ticket', async t => {
  const f = await fixture(t), created = await f.create(); assert.equal(created.status, 200);
  const ticket = await created.json(); assert.equal(ticket.sha256, f.digest); assert.equal(ticket.totalBytes, f.bytes.length);
  assert(!JSON.stringify(ticket).includes(f.endpoint.token));
  const response = await fetch(f.endpoint.origin + ticket.url);
  assert.equal(response.status, 200); assert.equal(response.headers.get('x-content-sha256'), f.digest);
  assert.match(response.headers.get('content-disposition'), /^attachment;/u);
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), f.bytes);
  assert.equal(f.calls.length, 4);
  for (const call of f.calls) { assert.equal(call.projectId, 1); assert.equal(call.context.expectedGrantRevision, 3); assert.equal(call.args.maxBytes, 196608); }
  assert.equal((await fetch(f.endpoint.origin + ticket.url)).status, 404);
});
test('artifact ticket creation still requires the Host token and same-origin checks', async t => {
  const f = await fixture(t);
  const url = `${f.endpoint.origin}/api/apps/${f.app.id}/artifact-downloads`;
  assert.equal((await fetch(url, { method: 'POST', body: JSON.stringify(input) })).status, 403);
  assert.equal((await fetch(url, { method: 'POST', headers: { 'x-oviraptor-session': f.endpoint.token, origin: 'https://foreign.example' }, body: JSON.stringify(input) })).status, 403);
  assert.equal(f.calls.length, 0);
});
test('revision or project changes invalidate artifact authority before a cached chunk is delivered', async t => {
  const f = await fixture(t), ticket = await (await f.create()).json();
  f.grant.revision++;
  const response = await fetch(f.endpoint.origin + ticket.url);
  assert.equal(response.status, 409); assert.equal(response.headers.get('content-disposition'), null);
  assert.equal(f.calls.length, 1);
  await assert.rejects(f.downloads.create(f.app.id, { ...input, projectId: 2 }), { code: 'host_project_missing' });
});
test('revocation during a native chunk read aborts a partial attachment and admits no later reads', async t => {
  const f = await fixture(t), ticket = await (await f.create()).json();
  f.setMutate(value => { if (value.offset > 0) f.grant.revision++; return value; });
  const response = await fetch(f.endpoint.origin + ticket.url);
  await assert.rejects(response.arrayBuffer()); assert.equal(f.calls.length, 2);
});
test('unregistered action roles, stale pages, malformed base64 and incoherent offsets never become download tickets', async t => {
  const f = await fixture(t);
  await assert.rejects(f.downloads.create(f.app.id, { ...input, pageId: 'unregistered' }), { code: 'host_artifact_authority_changed' });
  f.app.manifest.actions[0].delivery = undefined;
  await assert.rejects(f.downloads.create(f.app.id, input), { code: 'host_artifact_authority_changed' });
  f.app.manifest.actions[0].delivery = 'artifact-read';
  for (const mutate of [v => ({ ...v, bytesBase64: '!!!!' }), v => ({ ...v, nextOffset: v.nextOffset + 1 }), v => ({ ...v, totalBytes: -1 }), v => ({ ...v, done: true })]) {
    f.setMutate(mutate);
    await assert.rejects(f.downloads.create(f.app.id, input), { code: 'host_artifact_chunk_invalid' });
  }
  assert.equal(f.downloads.tickets.size, 0);
});
test('artifact checksum failure produces an error instead of a corrupt complete attachment', async t => {
  const f = await fixture(t, { bytes: Buffer.from('small export') });
  f.setMutate(value => ({ ...value, sha256: '0'.repeat(64) }));
  const ticket = await (await f.create()).json(), response = await fetch(f.endpoint.origin + ticket.url);
  assert.equal(response.status, 400); assert.equal(response.headers.get('content-length'), null);
  assert.equal(response.headers.get('content-disposition'), null);
  assert.equal((await response.json()).error, 'host_artifact_digest_mismatch');
});
test('tickets expire, reject oversize artifacts and support a real empty attachment', async t => {
  let clock = 10; const f = await fixture(t, { bytes: Buffer.alloc(0), now: () => clock, maxBytes: 10 });
  const ticket = await f.downloads.create(f.app.id, input); clock += 60001;
  await assert.rejects(f.downloads.stream(ticket.url.split('/').pop(), {}), { code: 'host_artifact_ticket_unavailable' });
  const response = await fetch(f.endpoint.origin + (await (await f.create()).json()).url);
  assert.equal(response.status, 200); assert.equal((await response.arrayBuffer()).byteLength, 0);
  const g = await fixture(t, { bytes: Buffer.alloc(11), maxBytes: 10 });
  await assert.rejects(g.downloads.create(g.app.id, input), { code: 'host_artifact_chunk_invalid' });
});
