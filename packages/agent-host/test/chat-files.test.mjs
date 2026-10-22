import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID, createHash } from 'node:crypto';
import { mkdtemp, realpath, rm, writeFile, unlink, symlink, link, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { ChatFiles } from '../src/chat-files.mjs';
import { runWorkspaceTool } from '../src/workspace-tools.mjs';
import { startTaskEvidence, finishTaskEvidence } from '../src/tasks.mjs';

async function fixture(t) {
  const dataDir = await mkdtemp(join(tmpdir(), 'ovi-chat-files-'));
  const store = new HostStore(join(dataDir, 'host.sqlite'));
  t.after(async () => { store.close(); await rm(dataDir, { recursive: true, force: true }); });
  const project = store.createProject('One'), other = store.createProject('Other');
  const conversation = store.createConversation(project.id), second = store.createConversation(project.id), foreign = store.createConversation(other.id);
  const files = new ChatFiles({ dataDir, store }), root = join(dataDir, 'workspaces', String(project.id), 'agent');
  function receipt(value, { owner = conversation, toolName = 'workspace_write', ok = true, unknown = false, runId = 'run-one' } = {}) {
    const task = store.createTask(owner.id, 'Write requested file');
    const started = startTaskEvidence(task, runId, toolName, 'call', { operationId: randomUUID() });
    const body = JSON.stringify({ content: [{ type: 'text', text: JSON.stringify(value) }] });
    const id = randomUUID();
    store.saveToolRecord({ id, projectId: owner.projectId, conversationId: owner.id, operationId: started.task.evidence[0].operationId, sha256: createHash('sha256').update(body).digest('hex'), body });
    store.saveTask(finishTaskEvidence(started.task, started.evidenceId, { ok, rawRecordId: id, outcomeUnknown: unknown }));
    return id;
  }
  async function write(path, content, options) {
    const result = await runWorkspaceTool(root, 'workspace_write', { path, content }, { anchor: dataDir });
    const id = receipt({ ...result, workspace: root }, options);return id;
  }
  return { dataDir, store, project, other, conversation, second, foreign, files, root, receipt, write };
}

test('only successful writes bound to this conversation become clickable file entries', async t => {
  const f = await fixture(t);await f.write('reports/移动 报告.md', '# Report\n');
  await writeFile(join(f.root, 'unrelated.txt'), 'Another conversation or external file');
  const changes = f.store.db.prepare('SELECT total_changes() n').get().n;
  const result = await f.files.list(f.conversation.id, f.project.id);
  assert.equal(result.items.length, 1);assert.equal(result.shared, true);
  assert.equal(result.items[0].path, 'reports/移动 报告.md');assert.equal(result.items[0].name, '移动 报告.md');
  assert.equal(result.items[0].runId, 'run-one');assert.equal(result.items[0].openable, true);assert.equal(result.items[0].previewable, true);
  assert.equal(result.items[0].absolutePath, await realpath(join(f.root, 'reports/移动 报告.md')));assert.ok(Date.parse(result.items[0].generatedAt));assert.ok(Date.parse(result.items[0].modifiedAt));
  assert.deepEqual((await f.files.list(f.second.id, f.project.id)).items, []);
  const preview = await f.files.preview(f.conversation.id, f.project.id, 'reports/移动 报告.md');assert.equal(preview.content, '# Report\n');
  assert.equal(f.store.db.prepare('SELECT total_changes() n').get().n, changes);
});

test('wrong project, invented local links, read receipts and unknown writes never become files', async t => {
  const f = await fixture(t);await f.write('written.txt', 'ok');
  for (const options of [{ toolName: 'workspace_read' }, { ok: false }, { unknown: true }]) {
    await writeFile(join(f.root, 'not-generated.txt'), 'not a successful write');
    f.receipt({ path: 'not-generated.txt', bytes: 22, workspace: f.root }, options);
  }
  f.receipt({ path: '../host.sqlite', bytes: 1, workspace: f.root });
  f.receipt({ path: 'written.txt', bytes: 2, workspace: '/an/unrelated/workspace' });
  assert.equal((await f.files.list(f.conversation.id, f.project.id)).items.length, 1);
  await assert.rejects(f.files.list(f.conversation.id, f.other.id), /host_conversation_unavailable/);
  await assert.rejects(f.files.preview(f.second.id, f.project.id, 'written.txt'), /host_chat_file_unavailable/);
  await assert.rejects(f.files.preview(f.conversation.id, f.project.id, '../host.sqlite'), /host_chat_file_path_denied/);
  await assert.rejects(f.files.preview(f.conversation.id, f.project.id, 'not-generated.txt'), /host_chat_file_unavailable/);
});

test('a missing old file stays unavailable, duplicate writes use the latest durable receipt', async t => {
  const f = await fixture(t);await f.write('notes.txt', 'first');
  const latest = await f.write('notes.txt', 'latest', { runId: 'run-two' });
  let result = await f.files.list(f.conversation.id, f.project.id);
  assert.equal(result.items.length, 1);assert.equal(result.items[0].recordId, latest);assert.equal(result.items[0].runId, 'run-two');
  assert.equal((await f.files.preview(f.conversation.id, f.project.id, 'notes.txt')).content, 'latest');
  await unlink(join(f.root, 'notes.txt'));result = await f.files.list(f.conversation.id, f.project.id);
  assert.deepEqual(result.items, []);assert.equal(result.unavailable, 1);
  await assert.rejects(f.files.preview(f.conversation.id, f.project.id, 'notes.txt'), /host_chat_file_unavailable/);
});

test('fresh conversation workspace reads do not create directories or claim per-conversation isolation', async t => {
  const f = await fixture(t);const workspace = await f.files.workspace(f.conversation.id, f.project.id);
  assert.equal(workspace.exists, false);assert.equal(workspace.shared, true);assert.equal(workspace.path, f.root);
  assert.deepEqual((await f.files.list(f.conversation.id, f.project.id)).items, []);
  await assert.rejects(f.files.workspace(f.foreign.id, f.project.id), /host_conversation_unavailable/);
});

test('text previews are bounded and HTML is plain content, never an external open target', async t => {
  const f = await fixture(t);await f.write('report.html', '<script>fetch("https://untrusted.test")</script>');
  await f.write('big.log', 'A'.repeat(70 * 1024));
  const list = await f.files.list(f.conversation.id, f.project.id), html = list.items.find(item => item.path === 'report.html');
  assert.equal(html.previewable, true);assert.equal(html.openable, false);
  assert.match((await f.files.preview(f.conversation.id, f.project.id, 'report.html')).content, /<script>/);
  const preview = await f.files.preview(f.conversation.id, f.project.id, 'big.log');assert.equal(preview.content.length, 64 * 1024);assert.equal(preview.truncated, true);
  await f.write('binary.txt', 'before\0after');
  await assert.rejects(f.files.preview(f.conversation.id, f.project.id, 'binary.txt'), /host_chat_file_preview_unsupported/);
});

test('links to outside files and replaced workspace directories are rejected', { skip: process.platform === 'win32' }, async t => {
  const f = await fixture(t);await f.write('linked.txt', 'original');
  await unlink(join(f.root, 'linked.txt'));const outside = join(f.dataDir, 'secret.txt');await writeFile(outside, 'private');
  await symlink(outside, join(f.root, 'linked.txt'));
  assert.equal((await f.files.list(f.conversation.id, f.project.id)).unavailable, 1);
  await assert.rejects(f.files.preview(f.conversation.id, f.project.id, 'linked.txt'), /host_chat_file_path_denied/);
  await unlink(join(f.root, 'linked.txt'));await link(outside, join(f.root, 'linked.txt'));
  assert.equal((await f.files.list(f.conversation.id, f.project.id)).unavailable, 1);
  await assert.rejects(f.files.preview(f.conversation.id, f.project.id, 'linked.txt'), /host_chat_file_path_denied/);
  await unlink(join(f.root, 'linked.txt'));await rm(f.root, { recursive: true });await symlink(f.dataDir, f.root);
  await assert.rejects(f.files.workspace(f.conversation.id, f.project.id), /host_chat_file_path_denied/);
});

test('executable text is previewable but not offered for system opening', { skip: process.platform === 'win32' }, async t => {
  const f = await fixture(t);await f.write('run.txt', '#!/bin/sh\necho test\n');await chmod(join(f.root, 'run.txt'), 0o700);
  const entry = (await f.files.list(f.conversation.id, f.project.id)).items[0];assert.equal(entry.openable, false);assert.equal(entry.previewable, true);
});
