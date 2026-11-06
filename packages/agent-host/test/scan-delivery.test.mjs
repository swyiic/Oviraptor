import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { ScanningTasks, inboxScanDelivery, scanTaskConversation, scanDeliveryEvents } from '../src/scanning-tasks.mjs';

const TOKEN = 'a'.repeat(64);

function tempStore() {
  const dir = mkdtempSync(join(tmpdir(), 'oviraptor-scan-delivery-'));
  const file = join(dir, 'host.sqlite');
  return { dir, file, store: new HostStore(file), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

/** A desktop bridge stand-in with the outbox semantics of the native side. */
async function fakeBridge(outbox) {
  const calls = [];
  const server = createServer((request, response) => {
    let raw = '';
    request.on('data', chunk => { raw += chunk; });
    request.on('end', () => {
      assert.equal(request.headers.authorization, `Bearer ${TOKEN}`);
      const body = JSON.parse(raw);
      calls.push(body);
      let result = { projectId: body.projectId };
      if (body.action === 'deliveries') result.deliveries = outbox.filter(item => item.state === 'pending');
      if (body.action === 'deliveries_ack') {
        for (const ack of body.args.acks) {
          const item = outbox.find(entry => entry.eventId === ack.eventId);
          if (item) item.state = ack.outcome;
        }
      }
      if (body.action === 'bind') result.bindingId = 'b-1';
      response.end(JSON.stringify({ ok: true, result }));
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  return { calls, origin, close: () => new Promise(resolve => server.close(resolve)) };
}

test('a delivery is stored once, survives a restart and never crosses projects', () => {
  const { file, store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const other = store.createProject('q', { externalId: '8' });
    const conversation = store.createConversation(project.id);
    const delivery = { eventId: 'e-1', conversationId: conversation.id, taskId: 'scan-1', text: '扫描任务 scan-1 已完成' };
    const first = inboxScanDelivery(store, delivery, project.id);
    assert.ok(first.messageId);
    assert.equal(inboxScanDelivery(store, delivery, project.id).replayed, true);
    assert.equal(store.messages(conversation.id, project.id).length, 1);
    const foreign = inboxScanDelivery(store, { ...delivery, eventId: 'e-2' }, other.id);
    assert.equal(foreign.rejected, true);
    assert.equal(store.messages(conversation.id, project.id).length, 1);
    store.close();
    const reopened = new HostStore(file);
    assert.equal(inboxScanDelivery(reopened, delivery, project.id).replayed, true);
    assert.equal(reopened.messages(conversation.id, project.id).length, 1);
    reopened.close();
  } finally { cleanup(); }
});

test('sync stores pending events in their conversation and acknowledges each outcome', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  const conversation = store.createConversation(project.id);
  const outbox = [
    { eventId: 'e-1', conversationId: conversation.id, taskId: 'scan-1', text: '结果一', state: 'pending' },
    { eventId: 'e-2', conversationId: 'deleted-conversation', taskId: 'scan-1', text: '结果二', state: 'pending' },
  ];
  const bridge = await fakeBridge(outbox);
  try {
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    const delivered = [];
    tasks.onDelivered = event => delivered.push(event);
    await tasks.sync(store, project.id);
    assert.deepEqual(outbox.map(item => item.state), ['delivered', 'rejected']);
    assert.equal(store.messages(conversation.id, project.id).length, 1);
    assert.equal(delivered.length, 1);
    assert.equal(delivered[0].conversationId, conversation.id);
    // An ack lost in transit leaves the event pending; the next pass stores nothing new.
    outbox[0].state = 'pending';
    await tasks.sync(store, project.id);
    assert.equal(store.messages(conversation.id, project.id).length, 1);
    assert.equal(outbox[0].state, 'delivered');
    assert.ok(bridge.calls.every(call => call.projectId === 7));
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('only the trusted run context names the conversation sent to the desktop', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  const conversation = store.createConversation(project.id);
  const bridge = await fakeBridge([]);
  try {
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    await tasks.call(store, project.id, 'scan_bind', { taskId: 'scan-1' }, 'op-1', { hostConversationId: conversation.id });
    await tasks.call(store, project.id, 'scan_status', { taskId: 'scan-1' }, 'op-2');
    assert.equal(bridge.calls[0].hostConversationId, conversation.id);
    assert.equal('hostConversationId' in bridge.calls[1], false);
    assert.equal(scanTaskConversation(store, project.id, 'scan-1'), conversation.id);
    store.deleteConversation(conversation.id, project.id);
    assert.equal(scanTaskConversation(store, project.id, 'scan-1'), undefined);
    await tasks.bindConversation(store, project.id, 'scan-2', store.createConversation(project.id).id);
    assert.equal(bridge.calls.at(-1).action, 'bind');
    assert.ok(scanTaskConversation(store, project.id, 'scan-2'));
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('a binding moves to this conversation only away from one that is gone', async () => {
  const { store, cleanup } = tempStore();
  const project = store.createProject('p', { externalId: '7' });
  const first = store.createConversation(project.id);
  const second = store.createConversation(project.id);
  const bridge = await fakeBridge([]);
  try {
    const tasks = new ScanningTasks({ origin: bridge.origin, token: TOKEN });
    await tasks.call(store, project.id, 'scan_bind', { taskId: 'scan-1' }, 'op-1', { hostConversationId: first.id });
    const refused = await tasks.call(store, project.id, 'scan_reassociate_binding', { taskId: 'scan-1' }, 'op-2', { hostConversationId: second.id }).catch(error => error.code);
    assert.equal(refused, 'host_scan_binding_source_active');
    assert.equal(bridge.calls.length, 1, 'nothing reached the desktop');
    store.deleteConversation(first.id, project.id);
    await tasks.call(store, project.id, 'scan_reassociate_binding', { taskId: 'scan-1' }, 'op-3', { hostConversationId: second.id });
    assert.equal(bridge.calls.at(-1).args.hostSourceGone, first.id);
    assert.equal(bridge.calls.at(-1).hostConversationId, second.id);
    assert.equal(scanTaskConversation(store, project.id, 'scan-1'), second.id);
  } finally { await bridge.close(); store.close(); cleanup(); }
});

test('worker events keep their correlation ids once and can be read per execution unit', () => {
  const { file, store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    const base = { conversationId: conversation.id, taskId: 'scan-1', kind: 'assignment_state', workerId: 'w-1', runId: 'child-1', targetKey: 'https://a.example/' };
    const running = { ...base, eventId: 'e-run', assignmentId: 'asg-1', text: '执行单元 asg-1 进入状态 running' };
    const done = { ...base, eventId: 'e-done', assignmentId: 'asg-1', text: '执行单元 asg-1 进入状态 completed' };
    const other = { ...base, eventId: 'e-other', assignmentId: 'asg-2', text: '执行单元 asg-2 进入状态 running' };
    const first = inboxScanDelivery(store, running, project.id);
    assert.deepEqual(first.correlation, { assignmentId: 'asg-1', workerId: 'w-1', targetKey: 'https://a.example/', runId: 'child-1' });
    assert.equal(first.kind, 'assignment_state');
    for (const item of [running, done, other, done]) inboxScanDelivery(store, item, project.id);
    assert.equal(store.messages(conversation.id, project.id).length, 3);
    const unit = scanDeliveryEvents(store, conversation.id, { assignmentId: 'asg-1' });
    assert.deepEqual(unit.map(row => row.eventId), ['e-run', 'e-done']);
    assert.equal(unit[0].runId, 'child-1');
    assert.equal(scanDeliveryEvents(store, conversation.id).length, 3);
    // A delivery without correlation ids (an attempt-level result) still stores.
    inboxScanDelivery(store, { eventId: 'e-plain', conversationId: conversation.id, taskId: 'scan-1', text: '扫描任务 scan-1 已完成' }, project.id);
    store.close();
    const reopened = new HostStore(file);
    const all = scanDeliveryEvents(reopened, conversation.id);
    assert.equal(all.length, 4);
    assert.equal(all.at(-1).assignmentId, '');
    assert.equal(inboxScanDelivery(reopened, other, project.id).replayed, true);
    reopened.close();
  } finally { cleanup(); }
});

test('an inbox created before correlation columns gains them without losing rows', () => {
  const { file, store, cleanup } = tempStore();
  try {
    const project = store.createProject('p', { externalId: '7' });
    const conversation = store.createConversation(project.id);
    store.db.exec(`CREATE TABLE scan_result_inbox(event_id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, task_id TEXT NOT NULL, message_id TEXT NOT NULL, created_at TEXT NOT NULL)`);
    store.db.prepare('INSERT INTO scan_result_inbox VALUES(?,?,?,?,?)').run('old-1', conversation.id, 'scan-1', 'm-old', '2026-01-01T00:00:00Z');
    const rows = scanDeliveryEvents(store, conversation.id);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].assignmentId, '');
    assert.equal(inboxScanDelivery(store, { eventId: 'old-1', conversationId: conversation.id, taskId: 'scan-1', text: 'x' }, project.id).replayed, true);
    store.close();
    void file;
  } finally { cleanup(); }
});
