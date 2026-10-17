import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

async function withHost(t, run) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-ux-scratch-'));
  const store = new HostStore(':memory:');
  const events = [];
  const host = new AgentHost({ dataDir: directory, store, onEvent: event => events.push(event) });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  return run({ host, store, events, directory });
}

test('createScratchProject marks kind=scratch and remains project-bound for conversations', async t => {
  await withHost(t, ({ store }) => {
    const scratch = store.createScratchProject();
    assert.equal(scratch.kind, 'scratch');
    assert.match(scratch.name, /^Scratch · /);
    const conversation = store.createConversation(scratch.id);
    assert.equal(conversation.projectId, scratch.id);
    const listed = store.projects();
    assert.equal(listed.find(p => p.id === scratch.id)?.kind, 'scratch');
  });
});

test('promoteProject renames scratch to durable project and rejects non-scratch', async t => {
  await withHost(t, ({ store }) => {
    const scratch = store.createScratchProject();
    const promoted = store.promoteProject(scratch.id, 'Asset review');
    assert.equal(promoted.kind, 'project');
    assert.equal(promoted.name, 'Asset review');
    const durable = store.createProject('Already real');
    assert.throws(() => store.promoteProject(durable.id, 'Nope'), error => error.code === 'host_project_not_scratch');
  });
});

test('createConversationFromRequest auto-creates scratch when projectId omitted and switch is on', async t => {
  await withHost(t, ({ host, store, events }) => {
    assert.equal(host.scratchConversationsEnabled(), true);
    assert.equal(store.projects().length, 0);
    const conversation = host.createConversationFromRequest({ title: 'New conversation' });
    assert.ok(conversation.id);
    const project = store.project(conversation.projectId);
    assert.equal(project.kind, 'scratch');
    assert.equal(events.some(e => e.type === 'project.created' && e.kind === 'scratch'), true);
  });
});

test('createConversationFromRequest with scratch:true works even when product switch is off', async t => {
  await withHost(t, ({ host, store }) => {
    host.setScratchConversations(false);
    assert.equal(host.scratchConversationsEnabled(), false);
    assert.throws(() => host.createConversationFromRequest({}), error => error.code === 'host_project_required');
    const conversation = host.createConversationFromRequest({ scratch: true });
    assert.equal(store.project(conversation.projectId).kind, 'scratch');
  });
});

test('explicit projectId path is unchanged for existing project-bound sessions', async t => {
  await withHost(t, ({ host, store }) => {
    const project = store.createProject('Durable workspace');
    const conversation = host.createConversationFromRequest({ projectId: project.id, title: 'Bound chat' });
    assert.equal(conversation.projectId, project.id);
    assert.equal(store.project(project.id).kind, 'project');
    assert.equal(store.conversations(project.id).length, 1);
  });
});

test('HTTP POST /api/conversations without projectId + promote round-trip', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-ux-scratch-http-'));
  const store = new HostStore(join(directory, 'host.sqlite'));
  const host = new AgentHost({ dataDir: directory, store });
  const web = await serveHost(host);
  t.after(async () => { await web.close(); await host.close(); await rm(directory, { recursive: true, force: true }); });
  const headers = { 'Content-Type': 'application/json', 'X-Oviraptor-Session': web.token };

  const created = await fetch(`${web.origin}/api/conversations`, { method: 'POST', headers, body: JSON.stringify({ scratch: true }) });
  assert.equal(created.status, 200);
  const conversation = await created.json();
  assert.ok(conversation.projectId);

  const projects = await (await fetch(`${web.origin}/api/projects`, { headers })).json();
  const scratch = projects.items.find(p => p.id === conversation.projectId);
  assert.equal(scratch.kind, 'scratch');

  const promoted = await fetch(`${web.origin}/api/projects/${conversation.projectId}/promote`, {
    method: 'POST', headers, body: JSON.stringify({ name: 'Named project' }),
  });
  assert.equal(promoted.status, 200);
  const body = await promoted.json();
  assert.equal(body.kind, 'project');
  assert.equal(body.name, 'Named project');

  const settings = await (await fetch(`${web.origin}/api/settings`, { headers })).json();
  assert.equal(settings.scratchConversations, true);
  const toggled = await fetch(`${web.origin}/api/settings`, {
    method: 'POST', headers, body: JSON.stringify({ scratchConversations: false }),
  });
  assert.equal((await toggled.json()).scratchConversations, false);
});

test('existing project create via POST /api/projects still defaults to kind=project', async t => {
  await withHost(t, ({ store }) => {
    const project = store.createProject('Normal');
    assert.equal(project.kind, 'project');
  });
});
