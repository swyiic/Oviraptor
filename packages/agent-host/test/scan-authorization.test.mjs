import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { HostStore } from '../src/store.mjs';
import { ScanningTasks, SCAN_AUTH_LEGAL_CLASSES, scanAuthorizationConversationAction } from '../src/scanning-tasks.mjs';

const TOKEN = 'a'.repeat(64);
const SCANNING_APP_ID = 'org.oviraptor.scanning';

function tempStore() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-scan-auth-'));
  const file = join(dir, 'host.sqlite');
  return { store: new HostStore(file), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

/** Desktop bridge stand-in. Records every POST so a test can prove the advisory
 *  path NEVER reaches it, and answers the authorize_capability write. */
async function fakeBridge() {
  const calls = [];
  const server = createServer((request, response) => {
    let raw = '';
    request.on('data', chunk => { raw += chunk; });
    request.on('end', () => {
      assert.equal(request.headers.authorization, `Bearer ${TOKEN}`);
      const body = JSON.parse(raw);
      calls.push(body);
      const result = { projectId: body.projectId };
      if (body.action === 'authorize_capability') {
        result.scanId = body.args.scanId;
        result.attemptNumber = body.args.attemptNumber ?? 1;
        result.authorized = true;
        result.classes = body.args.classes;
        result.attestedBy = body.args.attestedBy;
      }
      response.end(JSON.stringify({ ok: true, result }));
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { calls, origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise(resolve => server.close(resolve)) };
}

function makeHost(store, origin, options = {}) {
  const {
    permissions = ['investigation.read', 'investigation.execute'],
    identity = 'id-1', appRevision = 1, grantRevision = 1, policyRevision = 1,
    enabled = true, disclosureRuntime = 'id-1', emit = () => {},
  } = options;
  const app = { id: SCANNING_APP_ID, revision: appRevision, enabled };
  const scanningTasks = new ScanningTasks({ origin, token: TOKEN });
  const hostStore = {
    conversation: (...a) => store.conversation(...a),
    project: (...a) => store.project(...a),
    application: () => app,
    db: store.db,
  };
  return {
    store: hostStore,
    runtimeIdentity: () => identity,
    accessPolicy: { view: () => ({ revision: policyRevision }) },
    apps: { changing: new Set(), scanningTasks, effectiveGrant: () => ({ revision: grantRevision, permissions, disclosureRuntime }) },
    emit,
    witness: { appRevision, grantRevision, policyRevision, runtimeIdentity: identity },
  };
}

// --- Step 2: request_scan_authorization is advisory and never authorizes ---

test('request_scan_authorization opens the operator panel and never bridges or authorizes', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    const events = [];
    tasks.onAuthorizationRequested = meta => events.push(meta);
    const result = await tasks.call(store, project.id, 'request_scan_authorization',
      { taskId: 'scan-1', proposedClasses: ['internal_scope', 'code_exec', 'code_exec'], reason: 'SSRF reach observed' },
      'op-1', { hostConversationId: conversation.id });
    // No authorization was granted and the trusted bridge was never contacted.
    assert.equal(result.structuredContent.authorized, false);
    assert.equal(result.structuredContent.authorizationPending, true);
    assert.deepEqual(result.structuredContent.proposedClasses, ['code_exec', 'internal_scope']);
    assert.equal(bridge.calls.length, 0, 'advisory tool must never POST to the desktop bridge');
    // The operator panel was raised, anchored to the Host-verified conversation.
    assert.equal(events.length, 1);
    assert.equal(events[0].conversationId, conversation.id);
    assert.equal(events[0].taskId, 'scan-1');
    assert.deepEqual(events[0].proposedClasses, ['code_exec', 'internal_scope']);
  } finally { await bridge.close(); cleanup(); }
});

test('request_scan_authorization rejects unknown classes and still never bridges', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    let fired = false;
    tasks.onAuthorizationRequested = () => { fired = true; };
    const code = await tasks.call(store, project.id, 'request_scan_authorization',
      { taskId: 'scan-1', proposedClasses: ['make_coffee'] }, 'op-1', { hostConversationId: conversation.id })
      .then(() => 'ok', error => error.code);
    assert.equal(code, 'host_scan_authorization_class_invalid');
    assert.equal(bridge.calls.length, 0);
    assert.equal(fired, false);
  } finally { await bridge.close(); cleanup(); }
});

test('request_scan_authorization rejects a missing taskId', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    const code = await tasks.call(store, project.id, 'request_scan_authorization',
      { taskId: '', proposedClasses: ['code_exec'] }, 'op-1', { hostConversationId: conversation.id })
      .then(() => 'ok', error => error.code);
    assert.equal(code, 'host_scan_authorization_task_invalid');
    assert.equal(bridge.calls.length, 0);
  } finally { await bridge.close(); cleanup(); }
});

// --- Step 1: the operator authorization channel ---

test('authorizeCapability bridges authorize_capability as the operator', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    const result = await tasks.authorizeCapability(store, project.id,
      { scanId: 'scan-1', legalAttestation: true, classes: ['code_exec', 'egress'] }, 'op-1',
      { hostConversationId: conversation.id });
    assert.equal(bridge.calls.length, 1);
    assert.equal(bridge.calls[0].action, 'authorize_capability');
    assert.equal(bridge.calls[0].args.scanId, 'scan-1');
    assert.equal(bridge.calls[0].args.legalAttestation, true);
    assert.equal(bridge.calls[0].args.attestedBy, 'operator');
    assert.deepEqual(bridge.calls[0].args.classes, ['code_exec', 'egress']);
    assert.equal(result.structuredContent.authorized, true);
  } finally { await bridge.close(); cleanup(); }
});

test('authorizeCapability refuses a non-empty class set without the legal attestation and never bridges', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    const code = await tasks.authorizeCapability(store, project.id,
      { scanId: 'scan-1', legalAttestation: false, classes: ['code_exec'] }, 'op-1',
      { hostConversationId: conversation.id }).then(() => 'ok', error => error.code);
    assert.equal(code, 'host_scan_authorization_attestation_required');
    assert.equal(bridge.calls.length, 0);
  } finally { await bridge.close(); cleanup(); }
});

test('scanAuthorizationConversationAction authorizes, requires attestation, and emits updated', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const emitted = [];
    const host = makeHost(store, bridge.origin, { emit: e => emitted.push(e) });
    const result = await scanAuthorizationConversationAction(host, conversation.id, project.id,
      { action: 'authorize', taskId: 'scan-1', classes: ['code_exec', 'internal_scope'], legalAttestation: true, ...host.witness });
    assert.equal(result.authorized, true);
    assert.equal(bridge.calls.length, 1);
    assert.equal(bridge.calls[0].action, 'authorize_capability');
    assert.ok(emitted.some(e => e.type === 'scan.authorization.updated' && e.taskId === 'scan-1'));
  } finally { await bridge.close(); cleanup(); }
});

test('scanAuthorizationConversationAction refuses a stale witness before bridging', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const host = makeHost(store, bridge.origin);
    const code = await scanAuthorizationConversationAction(host, conversation.id, project.id,
      { action: 'authorize', taskId: 'scan-1', classes: ['code_exec'], legalAttestation: true,
        ...host.witness, grantRevision: host.witness.grantRevision + 9 })
      .then(() => 'ok', error => error.code);
    assert.equal(code, 'host_scan_authorization_stale');
    assert.equal(bridge.calls.length, 0);
  } finally { await bridge.close(); cleanup(); }
});

test('scanAuthorizationConversationAction authorize needs investigation.execute', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const host = makeHost(store, bridge.origin, { permissions: ['investigation.read'] });
    const code = await scanAuthorizationConversationAction(host, conversation.id, project.id,
      { action: 'authorize', taskId: 'scan-1', classes: ['code_exec'], legalAttestation: true, ...host.witness })
      .then(() => 'ok', error => error.code);
    assert.equal(code, 'host_tool_permission_denied');
    assert.equal(bridge.calls.length, 0);
  } finally { await bridge.close(); cleanup(); }
});

test('scanAuthorizationConversationAction status returns the witness and legal classes with only read', async () => {
  const { store, cleanup } = tempStore();
  const bridge = await fakeBridge();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const host = makeHost(store, bridge.origin, { permissions: ['investigation.read'] });
    const view = await scanAuthorizationConversationAction(host, conversation.id, project.id, {});
    assert.deepEqual(view.confirmationWitness, host.witness);
    assert.deepEqual([...view.legalClasses].sort(), [...SCAN_AUTH_LEGAL_CLASSES].sort());
    assert.equal(bridge.calls.length, 0);
  } finally { await bridge.close(); cleanup(); }
});

// --- Structural invariant: no operator-only write action is a model tool ---

test('the scanning manifest exposes request_scan_authorization but no authorize/standing model tool', () => {
  const manifestPath = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..', 'apps', 'scanning', 'app.package.json');
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')).manifest;
  const toolNames = manifest.tools.map(tool => tool.name);
  const advisory = manifest.tools.find(tool => tool.name === 'request_scan_authorization');
  assert.ok(advisory, 'request_scan_authorization must be a model tool');
  assert.equal(advisory.permission, 'investigation.draft.write');
  for (const forbidden of ['authorize', 'authorize_capability', 'set_scan_capability_authorization', 'set_scan_standing_authorization']) {
    assert.ok(!toolNames.includes(forbidden), `${forbidden} must never be a model tool`);
  }
});

test('SCAN_AUTH_LEGAL_CLASSES mirrors the Rust high-risk class set', () => {
  assert.deepEqual([...SCAN_AUTH_LEGAL_CLASSES].sort(),
    ['code_exec', 'egress', 'exfil', 'internal_scope', 'persistence', 'post_write', 'target_write', 'waf_evasion']);
});
