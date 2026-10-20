import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, symlink, link, realpath, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { hostError } from '../src/store.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { WORKSPACE_TOOL_SPECS, resolveInside, runWorkspaceTool } from '../src/workspace-tools.mjs';

const errorCode = expected => error => error?.code === expected;
async function directory(t) {
  const path = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-workspace-authority-'));
  t.after(() => rm(path, { recursive: true, force: true }));
  return path;
}
async function fixture(t) {
  const temporary = await directory(t);
  const events = [];
  const host = new AgentHost({ dataDir: join(temporary, 'host'), runtime: { status: { pid: 4242 }, close: async () => {}, cancel: async () => {} }, onEvent: event => events.push(event) });
  t.after(async () => {
    for (const run of host.runs.values()) await host.cancel(run.conversationId);
    host.runs.clear(); await host.close();
  });
  const project = host.store.createProject('Temporary workspace authority');
  const conversation = host.store.createConversation(project.id);
  const root = join(host.dataDir, 'workspaces', String(project.id), 'agent');
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'read.txt'), 'temporary read');
  const run = { id: 'run-one', conversationId: conversation.id, state: 'running', userText: 'temporary request' };
  host.runs.set(conversation.id, run);
  host.mcpScopes.set('test-scope', { conversationId: conversation.id, runtimePid: 4242 });
  const call = (name, args) => host.mcp('test-scope', conversation.id, 'tools/call', { name, arguments: args });
  const requests = () => events.filter(event => event.type === 'authorization.requested');
  const respond = (request, accepted) => host.resolveAuthorization(conversation.id, accepted, { requestId: request.requestId, runId: request.runId });
  return { host, temporary, project, conversation, root, run, call, requests, respond };
}

test('parallel read and write require separate request-bound approvals', async t => {
  const f = await fixture(t);
  const reading = f.call('workspace_read', { path: 'read.txt' });
  const writing = f.call('workspace_write', { path: 'write.txt', content: 'must not write on read grant' });
  const writeOutcome = writing.then(value => ({ value }), error => ({ error }));
  const first = f.requests()[0];
  assert.deepEqual(first.offers[0].permissions.map(permission => permission.permission), ['workspace.read']);
  assert.equal(f.requests().length, 1);
  assert.equal(f.host.status().authorizations[0].requestId, first.requestId);
  assert.equal(f.host.resolveAuthorization(f.conversation.id, true), false);
  assert.equal(f.host.resolveAuthorization(f.conversation.id, true, { requestId: 'another-request', runId: first.runId }), false);
  assert.equal(f.host.resolveAuthorization(f.conversation.id, true, { requestId: first.requestId, runId: 'another-run' }), false);
  assert.equal(f.respond(first, true), true);
  assert.match((await reading).content[0].text, /temporary read/);
  const second = f.requests()[1];
  assert.notEqual(second.requestId, first.requestId);
  assert.deepEqual(second.offers[0].permissions.map(permission => permission.permission), ['workspace.write']);
  await assert.rejects(stat(join(f.root, 'write.txt')), errorCode('ENOENT'));
  assert.equal(f.respond(first, true), false);
  assert.equal(f.respond(second, false), true);
  assert.equal((await writeOutcome).error.code, 'host_tool_permission_denied');
  await assert.rejects(stat(join(f.root, 'write.txt')), errorCode('ENOENT'));
  assert.deepEqual([...f.run.workspaceAllow], ['workspace.read']);
});

test('same-permission parallel operations share only that explicit run grant', async t => {
  const f = await fixture(t);
  const reading = f.call('workspace_read', { path: 'read.txt' });
  const listing = f.call('workspace_list', {});
  assert.equal(f.requests().length, 1);
  assert.equal(f.respond(f.requests()[0], true), true);
  await Promise.all([reading, listing]);
  await f.call('workspace_read', { path: 'read.txt' });
  assert.equal(f.requests().length, 1);
  assert.deepEqual([...f.run.workspaceAllow], ['workspace.read']);
});

test('authorization is registered before a synchronous approval observer responds', async t => {
  const f = await fixture(t);
  f.host.onEvent = event => {
    if (event.type === 'authorization.requested') assert.equal(f.respond(event, true), true);
  };
  assert.match((await f.call('workspace_read', { path: 'read.txt' })).content[0].text, /temporary read/);
  assert.equal(f.host.status().authorizations.length, 0);
});

test('cancel releases pending read/write approvals and rejects late responses in a new run', async t => {
  const f = await fixture(t);
  const reading = f.call('workspace_read', { path: 'read.txt' }).then(value => ({ value }), error => ({ error }));
  const writing = f.call('workspace_write', { path: 'late.txt', content: 'must not exist' }).then(value => ({ value }), error => ({ error }));
  const oldRequest = f.requests()[0];
  await f.host.cancel(f.conversation.id);
  assert.equal((await reading).error.code, 'host_tool_permission_denied');
  assert.equal((await writing).error.code, 'host_tool_permission_denied');
  assert.equal(f.respond(oldRequest, true), false);
  assert.equal(f.host.status().authorizations.length, 0);
  await assert.rejects(stat(join(f.root, 'late.txt')), errorCode('ENOENT'));
  const nextRun = { id: 'run-two', conversationId: f.conversation.id, state: 'running' };
  f.host.runs.set(f.conversation.id, nextRun);
  const next = f.call('workspace_write', { path: 'late.txt', content: 'new explicit request' }).then(value => ({ value }), error => ({ error }));
  const nextRequest = f.requests().at(-1);
  assert.equal(nextRequest.runId, 'run-two');
  assert.equal(f.respond(oldRequest, true), false);
  assert.equal(f.respond(nextRequest, false), true);
  assert.equal((await next).error.code, 'host_tool_permission_denied');
  await assert.rejects(stat(join(f.root, 'late.txt')), errorCode('ENOENT'));
});

test('approval is revalidated against the MCP scope before any file write', async t => {
  const f = await fixture(t);
  const writing = f.call('workspace_write', { path: 'revoked.txt', content: 'must not exist' });
  const outcome = writing.then(value => ({ value }), error => ({ error }));
  assert.equal(f.respond(f.requests()[0], true), true);
  f.host.mcpScopes.delete('test-scope');
  assert.equal((await outcome).error.code, 'host_mcp_scope_unavailable');
  await assert.rejects(stat(join(f.root, 'revoked.txt')), errorCode('ENOENT'));
});

test('cancellation immediately after approval cannot write a file', async t => {
  const f = await fixture(t);
  const writing = f.call('workspace_write', { path: 'cancelled.txt', content: 'must not exist' }).then(value => ({ value }), error => ({ error }));
  assert.equal(f.respond(f.requests()[0], true), true);
  await f.host.cancel(f.conversation.id);
  assert.equal((await writing).error.code, 'host_mcp_scope_unavailable');
  await assert.rejects(stat(join(f.root, 'cancelled.txt')), errorCode('ENOENT'));
});

test('workspace file operations reject links to outside files and directories', async t => {
  const temporary = await directory(t), root = join(temporary, 'workspace'), outside = join(temporary, 'outside');
  await mkdir(root); await mkdir(outside);
  await writeFile(join(outside, 'sentinel.txt'), 'outside sentinel');
  await symlink(outside, join(root, 'directory-link'));
  await symlink(join(outside, 'sentinel.txt'), join(root, 'file-link'));
  for (const path of ['directory-link/sentinel.txt', 'file-link']) {
    await assert.rejects(runWorkspaceTool(root, 'workspace_read', { path }), errorCode('workspace_path_denied'));
    await assert.rejects(runWorkspaceTool(root, 'workspace_write', { path, content: 'bad overwrite' }), errorCode('workspace_path_denied'));
  }
  await assert.rejects(runWorkspaceTool(root, 'workspace_list', { path: 'directory-link' }), errorCode('workspace_path_denied'));
  await assert.rejects(runWorkspaceTool(root, 'workspace_write', { path: 'directory-link/new/deep.txt', content: 'bad create' }), errorCode('workspace_path_denied'));
  assert.equal(await readFile(join(outside, 'sentinel.txt'), 'utf8'), 'outside sentinel');
  await assert.rejects(stat(join(outside, 'new')), errorCode('ENOENT'));
});

test('workspace anchor rejects a redirected workspace parent and root symlink', async t => {
  const temporary = await directory(t), anchor = join(temporary, 'host'), outside = join(temporary, 'outside');
  await mkdir(anchor); await mkdir(outside);
  await symlink(outside, join(anchor, 'workspaces'));
  await assert.rejects(runWorkspaceTool(join(anchor, 'workspaces/1/agent'), 'workspace_write', { path: 'bad.txt', content: 'bad' }, { anchor }), errorCode('workspace_path_denied'));
  await assert.rejects(stat(join(outside, '1')), errorCode('ENOENT'));
  await symlink(outside, join(temporary, 'root-link'));
  await assert.rejects(runWorkspaceTool(join(temporary, 'root-link'), 'workspace_write', { path: 'bad.txt', content: 'bad' }), errorCode('workspace_path_denied'));
});

test('workspace handles regular nested files, bounded reads and atomic overwrite', async t => {
  const temporary = await directory(t), root = join(temporary, 'workspace');
  await runWorkspaceTool(root, 'workspace_write', { path: 'nested/deep/file.txt', content: 'first' });
  await runWorkspaceTool(root, 'workspace_write', { path: 'nested/deep/file.txt', content: 'second' });
  assert.equal((await runWorkspaceTool(root, 'workspace_read', { path: 'nested/deep/file.txt' })).content, 'second');
  await writeFile(join(root, 'large.txt'), 'x'.repeat(128 * 1024));
  const value = await runWorkspaceTool(root, 'workspace_read', { path: 'large.txt' });
  assert.equal(value.truncated, true); assert.equal(Buffer.byteLength(value.content), 64 * 1024);
  assert.equal(resolveInside(root, '..safe.txt'), join(root, '..safe.txt'));
  await assert.rejects(runWorkspaceTool(root, 'workspace_write', { path: '../bad.txt', content: 'bad' }), errorCode('workspace_path_denied'));
});

test('hard-linked files are not read and overwriting never mutates the outside inode', async t => {
  const temporary = await directory(t), root = join(temporary, 'workspace'), outside = join(temporary, 'outside.txt');
  await mkdir(root); await writeFile(outside, 'outside value');
  await link(outside, join(root, 'linked.txt'));
  await assert.rejects(runWorkspaceTool(root, 'workspace_read', { path: 'linked.txt' }), errorCode('workspace_path_denied'));
  await runWorkspaceTool(root, 'workspace_write', { path: 'linked.txt', content: 'workspace value' });
  assert.equal(await readFile(outside, 'utf8'), 'outside value');
  assert.equal((await runWorkspaceTool(root, 'workspace_read', { path: 'linked.txt' })).content, 'workspace value');
});

test('unsandboxed execution is absent from discovery and rejected before approval', async t => {
  const f = await fixture(t);
  assert.equal(WORKSPACE_TOOL_SPECS.some(tool => tool.name === 'workspace_run'), false);
  const listed = await f.host.mcp('test-scope', f.conversation.id, 'tools/list');
  assert.equal(listed.tools.some(tool => tool.name === 'workspace_run'), false);
  await assert.rejects(f.call('workspace_run', { command: 'node', args: ['-p', 'process.env'] }), errorCode('host_workspace_execution_unsupported'));
  await assert.rejects(f.call('workspace_unknown', {}), errorCode('host_workspace_tool_unknown'));
  assert.equal(f.requests().length, 0);
});

test('logging out OAuth preserves an active API-key provider and clears dormant OAuth', async t => {
  const f = await fixture(t);
  f.host.runs.clear();
  f.host.providerStore.saveOAuth({ accessToken: 'temporary-oauth-token', refreshToken: 'temporary-refresh-token' });
  f.host.providerStore.save({ provider: 'openai-compatible', baseUrl: 'http://127.0.0.1:9/v1', model: 'temporary-model', apiKey: 'temporary-api-key' });
  await f.host.setRuntime('xai-api');
  const identity = f.host.runtimeIdentity();
  const runtime = f.host.runtime;
  await f.host.logoutProviderOAuth();
  assert.equal(f.host.runtimeIdentity(), identity);
  assert.equal(f.host.runtime, runtime);
  assert.equal(f.host.providerStore.bearer(), 'temporary-api-key');
  assert.equal(f.host.providerStore.read().authentication, 'api-key');
  assert.equal(f.host.providerStore.publicView().oauthConfigured, false);
});

test('a timed-out write is not retried and keeps a single operation id', async t => {
  const f = await fixture(t), attempts = [], effects = new Set();
  f.host.apps.processFactory = () => ({
    stopped: false, start: async () => {}, stop: async () => {},
    call: async (name, args, context) => {
      attempts.push(context.operationId);
      effects.add(context.operationId);
      if (attempts.length < 3) throw hostError('host_app_timeout');
      return { content: [{ type: 'text', text: JSON.stringify({ effects: effects.size }) }] };
    },
  });
  const sourceDirectory = fileURLToPath(new URL('../../../examples/echo-app/', import.meta.url));
  const specification = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const packaged = await createDirectoryPackage({ ...specification, sourceDirectory, outputDirectory: join(f.temporary, 'echo-package') });
  await f.host.apps.install(packaged.directory); await f.host.apps.setEnabled('org.oviraptor.echo', true);
  f.host.apps.setGrants('org.oviraptor.echo', f.project.id, ['echo.write'], undefined, f.host.runtimeIdentity());
  await assert.rejects(f.call('org.oviraptor.echo__echo', { text: 'temporary operation' }), error => error.code === 'host_app_timeout');
  assert.equal(attempts.length, 1);
  assert.equal(effects.size, 1);
});

test('explicit conversation file grant survives turns and restart, with isolation and revocation', async t => {
  const f = await fixture(t);
  const reading = f.call('workspace_read', { path: 'read.txt' });
  const request = f.requests()[0];
  assert.equal(f.host.resolveAuthorization(f.conversation.id, true, { requestId: request.requestId, runId: request.runId, remember: true }), true);
  await reading;
  assert.equal(f.host.store.db.prepare('SELECT COUNT(*) n FROM conversation_workspace_grants').get().n, 1);
  f.run.workspaceAllow.clear();
  f.run.id = 'second-turn';
  await f.call('workspace_read', { path: 'read.txt' });
  assert.equal(f.requests().length, 1);
  const other = new AgentHost({ dataDir: f.host.dataDir, runtime: { status: { pid: 4242 }, close: async () => {} } });
  t.after(() => other.close());
  other.runs.set(f.conversation.id, { id: 'restarted-turn', conversationId: f.conversation.id, state: 'running' });
  other.mcpScopes.set('scope', { conversationId: f.conversation.id, runtimePid: 4242 });
  await other.mcp('scope', f.conversation.id, 'tools/call', { name: 'workspace_read', arguments: { path: 'read.txt' } });
  other.runs.clear();
  const anotherProject = f.host.store.createProject('Other project');
  assert.throws(() => f.host.clearWorkspaceGrants(f.conversation.id, anotherProject.id));
  assert.equal(f.host.store.db.prepare('SELECT COUNT(*) n FROM conversation_workspace_grants').get().n, 1);
  f.host.clearWorkspaceGrants(f.conversation.id, f.project.id);
  const pending = f.call('workspace_read', { path: 'read.txt' }).then(value => ({value}), error => ({error}));
  assert.equal(f.requests().length, 2);
  f.respond(f.requests().at(-1), false);
  assert.equal((await pending).error.code, 'host_tool_permission_denied');
  f.host.clearWorkspaceGrants(f.conversation.id, f.project.id);
});

test('remembered file read grants do not permit writes, another conversation or another runtime', async t => {
  const f = await fixture(t);
  f.host.store.db.prepare('INSERT INTO conversation_workspace_grants VALUES(?,?,?)').run(f.conversation.id, f.host.runtimeIdentity(), 'workspace.read');
  const writing = f.call('workspace_write', { path: 'not-approved.txt', content: 'blocked' }).then(value => ({value}), error => ({error}));
  assert.equal(f.requests()[0].offers[0].permissions[0].permission, 'workspace.write');
  f.respond(f.requests()[0], false); assert.equal((await writing).error.code, 'host_tool_permission_denied');
  f.host.runtimeRevision++; f.run.workspaceAllow.clear();
  const reading = f.call('workspace_read', { path: 'read.txt' }).then(value => ({value}), error => ({error}));
  assert.equal(f.requests().length, 2);
  f.respond(f.requests().at(-1), false); assert.equal((await reading).error.code, 'host_tool_permission_denied');
  const conversation = f.host.store.createConversation(f.project.id);
  assert.equal(f.host.store.db.prepare('SELECT COUNT(*) n FROM conversation_workspace_grants WHERE conversation_id=?').get(conversation.id).n, 0);
});

test('desktop asset questions request the actual inventory grant once before the model, instead of asking for unrelated files', async t => {
  const f = await fixture(t);f.host.runs.clear();
  f.host.store.db.prepare("UPDATE projects SET external_id='42' WHERE id=?").run(f.project.id);
  const sourceDirectory = fileURLToPath(new URL('../../../apps/assets/', import.meta.url));
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const built = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(f.temporary, 'assets') });
  await f.host.apps.install(built.directory);await f.host.apps.setEnabled(spec.manifest.id,true);
  let prompts=0;
  f.host.runtimeId='xai-api';
  Object.assign(f.host.runtime,{start:async()=>{},newSession:async()=>({sessionId:'data-session'}),loadSession:async()=>{},prompt:async()=>{prompts++;return{stopReason:'end_turn'};}});
  const sent=await f.host.send(f.conversation.id,{projectId:f.project.id,text:'当前项目有多少资产和证书线索？'});
  const run=f.host.runs.get(f.conversation.id);
  for(let i=0;i<200&&!f.requests().length;i++)await new Promise(resolve=>setTimeout(resolve,5));
  const request=f.requests()[0];assert.ok(request);assert.equal(prompts,0);
  assert.equal(request.offers[0].appId,'org.oviraptor.assets');
  assert.equal(request.offers[0].permissions[0].permission,'inventory.read');
  f.host.apps.setGrants(spec.manifest.id,f.project.id,['inventory.read'],request.offers[0].revision,request.identity);
  assert.equal(f.host.resolveAuthorization(f.conversation.id,true,{requestId:request.requestId,runId:sent.run.id}),true);
  await run.completion;assert.equal(prompts,1);
  await f.host.send(f.conversation.id,{projectId:f.project.id,text:'当前项目有多少资产？'});
  await f.host.runs.get(f.conversation.id).completion;
  assert.equal(prompts,2);assert.equal(f.requests().length,1);
  f.host.runtimeRevision++;
  const offer=f.host.permissionOffer(f.conversation.id,['org.oviraptor.assets__inventory_stats']);
  assert.equal(offer.offers.length,1,'a changed model identity must offer renewed disclosure even when permission IDs already exist');
  assert.equal(f.host.permissionOffer(f.conversation.id,['different.application__inventory_stats']).offers.length,0,'a qualified name cannot request a different application scope');
});

test('desktop scanning requests only read and control scopes once and reuses the persisted runtime grant',async t=>{
 const f=await fixture(t);f.host.runs.clear();f.host.store.db.prepare("UPDATE projects SET external_id='42' WHERE id=?").run(f.project.id);
 const sourceDirectory=fileURLToPath(new URL('../../../apps/scanning/',import.meta.url));const spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const built=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(f.temporary,'scanning')});await f.host.apps.install(built.directory);await f.host.apps.setEnabled(spec.manifest.id,true);
 f.host.apps.scanningTasks={available:true};let prompts=0;f.host.runtimeId='xai-api';Object.assign(f.host.runtime,{start:async()=>{},newSession:async()=>({sessionId:'scan-session'}),loadSession:async()=>{},prompt:async()=>{prompts++;return{stopReason:'end_turn'};}});
 const sent=await f.host.send(f.conversation.id,{projectId:f.project.id,text:'暂停当前项目扫描任务，并核对进度。'});const run=f.host.runs.get(f.conversation.id);
 for(let i=0;i<200&&!f.requests().length;i++)await new Promise(resolve=>setTimeout(resolve,5));
 const request=f.requests()[0];assert.ok(request);assert.equal(prompts,0);assert.equal(request.offers.length,1);
 assert.equal(request.offers[0].appId,spec.manifest.id);const permissions=request.offers[0].permissions.map(item=>item.permission);assert.deepEqual(permissions.sort(),['investigation.control','investigation.read']);
 f.host.apps.setGrants(spec.manifest.id,f.project.id,permissions,request.offers[0].revision,request.identity);assert.equal(f.respond({...request,runId:sent.run.id},true),true);await run.completion;
 await f.host.send(f.conversation.id,{projectId:f.project.id,text:'查看当前扫描任务进度。'});await f.host.runs.get(f.conversation.id).completion;
 assert.equal(prompts,2);assert.equal(f.requests().length,1);
});

test('a structured outcomeUnknown receipt remains unresolved evidence and blocks repeating the same write',async t=>{
 const f=await fixture(t);const task=f.host.store.createTask(f.conversation.id,'Run one durable control');f.run.taskId=task.id;
 const sourceDirectory=fileURLToPath(new URL('../../../apps/scanning/',import.meta.url));const spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const built=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(f.temporary,'scanning')});await f.host.apps.install(built.directory);await f.host.apps.setEnabled(spec.manifest.id,true);
 let calls=0;f.host.apps.scanningTasks={available:true,async call(){calls++;return{content:[{type:'text',text:'uncertain'}],structuredContent:{outcomeUnknown:true,retryable:false,taskId:'scan'}};}};
 f.host.apps.setGrants(spec.manifest.id,f.project.id,['investigation.execute'],undefined,f.host.runtimeIdentity());
 const result=await f.call('org.oviraptor.scanning__scan_start',{taskId:'scan'});assert.equal(result.structuredContent.outcomeUnknown,true);
 const evidence=f.host.task(f.conversation.id).evidence[0];assert.equal(evidence.outcomeUnknown,true);assert.notEqual(evidence.status,'completed');
 await assert.rejects(f.call('org.oviraptor.scanning__scan_start',{taskId:'scan'}),errorCode('host_operation_unknown'));assert.equal(calls,1);
});

test('file grant inspection remains conversation/project/runtime bound over authenticated HTTP',async t=>{
 const f=await fixture(t);const {serveHost}=await import('../src/server.mjs');const web=await serveHost(f.host);t.after(()=>web.close());
 f.host.store.db.prepare('INSERT INTO conversation_workspace_grants VALUES(?,?,?)').run(f.conversation.id,f.host.runtimeIdentity(),'workspace.read');
 const url=project=>`${web.origin}/api/runtime/workspace-grants?conversationId=${f.conversation.id}&projectId=${project}`;
 assert.equal((await fetch(url(f.project.id))).status,403);
 const headers={'X-Oviraptor-Session':web.token};
 assert.deepEqual(await(await fetch(url(f.project.id),{headers})).json(),{permissions:['workspace.read']});
 const other=f.host.store.createProject('Other');assert.equal((await fetch(url(other.id),{headers})).status,404);
 f.host.runtimeRevision++;
 assert.deepEqual(await(await fetch(url(f.project.id),{headers})).json(),{permissions:[]});
});
