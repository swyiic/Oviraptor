import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile,mkdtemp,rm,realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { requestedBusinessTools } from '../src/task-intent-permissions.mjs';
import { AgentHost } from '../src/service.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const repo=fileURLToPath(new URL('../../../',import.meta.url));
async function contracts(){
 return Promise.all(['assets','scanning'].map(async name=>{
  const {manifest}=JSON.parse(await readFile(join(repo,'apps',name,'app.package.json'),'utf8'));
  return{id:manifest.id,enabled:true,manifest};
 }));
}

test('combined first read offers both owners with exact qualified read scopes, reuses grants and never asks for execution',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-first-read-'));const host=new AgentHost({dataDir:join(root,'data'),runtimeSelection:'none'});t.after(async()=>{await host.close();await rm(root,{recursive:true,force:true});});
 for(const name of ['assets','scanning']){
  const sourceDirectory=join(repo,'apps',name),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,name)});await host.apps.install(pack.directory);await host.apps.setEnabled(spec.manifest.id,true);
 }
 const project=host.store.createProject('one',{externalId:'3'}),conversation=host.store.createConversation(project.id,'read');
 const request=()=>requestedBusinessTools('核对当前项目证书任务和扫描结果。',{applications:host.store.applications(),certificateAvailable:true,scanningAvailable:true});
 const tools=request();assert(tools.includes('org.oviraptor.assets::certificate_task_status'));assert(tools.includes('org.oviraptor.scanning::scan_results'));
 let offer=host.permissionOffer(conversation.id,tools);assert.deepEqual(offer.offers.map(x=>x.permissions.map(p=>p.permission)),[['inventory.read'],['investigation.read']]);
 host.apps.setGrants('org.oviraptor.assets',project.id,['inventory.read'],0,host.runtimeIdentity());
 offer=host.permissionOffer(conversation.id,request());assert.deepEqual(offer.offers.map(x=>x.appId),['org.oviraptor.scanning']);
 host.apps.setGrants('org.oviraptor.scanning',project.id,['investigation.read'],0,host.runtimeIdentity());
 assert.deepEqual(host.permissionOffer(conversation.id,request()).offers,[]);
 const other=host.store.createProject('other',{externalId:'4'}),otherChat=host.store.createConversation(other.id,'other');assert.equal(host.permissionOffer(otherChat.id,request()).offers.length,2);
});

test('first asset handoff distinguishes draft-only, explicit execution, prohibited handoff and unavailable owners',async()=>{
 const applications=await contracts(),options={applications,certificateAvailable:true,scanningAvailable:true};
 const draft=requestedBusinessTools('找到当前项目库存资产，交给扫描，新建草稿，先不要启动；启动前检查模型。',options);
 assert(draft.includes('org.oviraptor.scanning::scan_from_assets'));assert(!draft.includes('org.oviraptor.scanning::scan_start'));
 const execute=requestedBusinessTools('把库存资产交给扫描，创建草稿并启动。',options);assert(execute.includes('org.oviraptor.scanning::scan_start'));
 const explanation=requestedBusinessTools('如何把当前项目资产交给扫描，创建草稿并启动？',options);assert(!explanation.some(n=>/scan_(start|resume|pause|from_assets)$/.test(n)));
 const denied=requestedBusinessTools('查看当前项目资产，不要交接扫描。',options);assert(!denied.some(n=>n.endsWith('scan_from_assets')));
 const read=requestedBusinessTools('核对当前项目扫描任务。只读取数据，不启动、恢复或新建。',options);assert(!read.some(n=>/scan_(start|resume|pause|from_assets)$/.test(n)));
 assert(!requestedBusinessTools('核对扫描结果。只读取数据。',{...options,scanningAvailable:false}).some(n=>n.includes('scanning')));
});

test('seed reading offers the existing exact read categories, without collection or mutation grants',async()=>{
 const applications=await contracts(),options={applications,certificateAvailable:true};
 for(const text of ['你读取我当前项目空间的项目种子文件，看看总数量多少，然后明确是移动的项目又是多少','只读读取整个项目种子证据，不启动 AI 发现、不执行任何排除']){
  const names=requestedBusinessTools(text,options);
  assert(names.includes('org.oviraptor.assets::discovery_seed_list'),text);
  assert(names.includes('org.oviraptor.assets::discovery_seed_detail'),text);
  assert(!names.some(name=>/_(start|pause|resume)$/.test(name)),text);
  for(const name of names){const tool=applications.find(app=>name.startsWith(app.id+'::')).manifest.tools.find(tool=>name.endsWith('::'+tool.name));assert.equal(tool.permission,'inventory.read');}
 }
 assert(!requestedBusinessTools('读取当前项目种子清单',{applications,certificateAvailable:false}).some(name=>name.includes('discovery_seed')));
});

// Uses real Host authorization, manifests, MCP scope checks and application
// grants with disposable data; native execution and the model are scripted.
async function partialAuthorizationFixture(t){
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-partial-authorization-'));
 const events=[],prompts=[],ownerCalls=[];
 const host=new AgentHost({dataDir:join(root,'data'),runtimeSelection:'none',providerFetch:async()=>{throw Error('external network is not used');},onEvent:event=>events.push(event)});
 t.after(async()=>{await host.close();await rm(root,{recursive:true,force:true});});
 await host.setProvider({provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture',apiKey:'synthetic-partial-authorization-key'});
 await host.setRuntime('xai-api');
 for(const name of ['assets','scanning']){
  const sourceDirectory=join(repo,'apps',name),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,name)});
  await host.apps.install(pack.directory);await host.apps.setEnabled(spec.manifest.id,true);
 }
 const project=host.store.createProject('Partial authorization',{externalId:'91'}),conversation=host.store.createConversation(project.id,'Read before execution');
 const owner={available:true,async call(_store,projectId,name){ownerCalls.push(name);return{structuredContent:{projectId,tasks:[],source:'scripted_native_owner'}};}};
 host.apps.certificateTasks=owner;host.apps.scanningTasks=owner;
 host.runtime.prompt=async input=>{prompts.push(input);return{stopReason:'end_turn',metrics:{}};};
 const send=async text=>{await host.send(conversation.id,{projectId:project.id,text});await host.runs.get(conversation.id)?.completion;};
 const waitForApproval=async()=>{
  for(let i=0;i<100;i++){const event=events.find(event=>event.type==='authorization.requested');if(event)return event;await new Promise(setImmediate);}
  assert.fail('Expected one real Host approval request');
 };
 return{host,project,conversation,events,prompts,ownerCalls,send,waitForApproval};
}

test('declining new execution permission keeps existing reads usable and passes exact gaps outside user text',async t=>{
 const f=await partialAuthorizationFixture(t),{host,project,conversation}=f;
 host.apps.setGrants('org.oviraptor.assets',project.id,['inventory.read'],0,host.runtimeIdentity());
 const before=host.store.grant('org.oviraptor.assets',project.id);
 host.runtime.prompt=async input=>{
  f.prompts.push(input);
  const run=host.runs.get(conversation.id),listed=await host.mcp(run.scopeToken,conversation.id,'tools/list',{});
  assert(listed.tools.some(tool=>tool.name==='org.oviraptor.assets__certificate_task_status'));
  assert(!listed.tools.some(tool=>tool.name==='org.oviraptor.assets__certificate_task_start'));
  await host.mcp(run.scopeToken,conversation.id,'tools/call',{name:'org.oviraptor.assets__certificate_task_status',arguments:{}});
  await assert.rejects(host.mcp(run.scopeToken,conversation.id,'tools/call',{name:'org.oviraptor.assets__certificate_task_start',arguments:{}}),{code:'host_tool_permission_denied'});
  return{stopReason:'end_turn',metrics:{}};
 };
 const text='核对当前项目证书任务，再启动批量证书补采',sending=f.send(text),request=await f.waitForApproval();
 assert.equal(host.resolveAuthorization(conversation.id,false,{requestId:request.requestId,runId:request.runId}),true);
 await sending;
 assert.equal(f.prompts.length,1);assert.equal(f.prompts[0].userText,text);
 assert.deepEqual(f.prompts[0].authorizationContext,{unavailableTools:['org.oviraptor.assets::certificate_task_start'],authorizationDeclined:true});
 assert(!f.prompts[0].prompt.includes('authorizationContext'));
 assert.deepEqual(f.ownerCalls,['certificate_task_status']);assert.deepEqual(host.store.grant('org.oviraptor.assets',project.id),before);
 assert.equal(f.events.filter(event=>event.type==='authorization.requested').length,1);
 assert.equal(host.store.messages(conversation.id).at(-1).status,'completed');
});

test('partial approval reports only still-unavailable tools and never expands the approved categories',async t=>{
 const f=await partialAuthorizationFixture(t),{host,project,conversation}=f;
 host.apps.setGrants('org.oviraptor.assets',project.id,['inventory.read'],0,host.runtimeIdentity());
 const before=host.store.grant('org.oviraptor.assets',project.id);
 const text='把当前项目库存资产交给扫描，新建草稿并启动',sending=f.send(text),request=await f.waitForApproval();
 host.apps.setGrants('org.oviraptor.scanning',project.id,['investigation.read'],0,host.runtimeIdentity());
 assert.equal(host.resolveAuthorization(conversation.id,true,{requestId:request.requestId,runId:request.runId}),true);
 await sending;
 const requested=requestedBusinessTools(text,{applications:host.store.applications(),certificateAvailable:true,scanningAvailable:true});
 const available=new Set(host.apps.capabilities(project.id,{runtimeIdentity:host.runtimeIdentity()}).map(tool=>tool.name));
 assert.equal(f.prompts.length,1);
 assert.deepEqual(f.prompts[0].authorizationContext,{unavailableTools:requested.filter(name=>!available.has(name)),authorizationDeclined:false});
 assert(f.prompts[0].authorizationContext.unavailableTools.includes('org.oviraptor.scanning::scan_start'));
 assert(!f.prompts[0].authorizationContext.unavailableTools.includes('org.oviraptor.scanning::scan_status'));
 assert.deepEqual(host.store.grant('org.oviraptor.assets',project.id),before);
 assert.deepEqual(host.store.grant('org.oviraptor.scanning',project.id).permissions,['investigation.read']);
 assert.deepEqual(f.ownerCalls,[]);
});

test('declining every new category still reaches the runtime to explain the gap without granting or executing it',async t=>{
 const f=await partialAuthorizationFixture(t),{host,project,conversation}=f;
 const text='核对当前项目证书任务，再启动批量证书补采',sending=f.send(text),request=await f.waitForApproval();
 assert.equal(host.resolveAuthorization(conversation.id,false,{requestId:request.requestId,runId:request.runId}),true);
 await sending;
 assert.equal(f.prompts.length,1);
 assert.deepEqual(f.prompts[0].authorizationContext,{unavailableTools:requestedBusinessTools(text,{applications:host.store.applications(),certificateAvailable:true,scanningAvailable:true}),authorizationDeclined:true});
 assert.deepEqual(host.store.grant('org.oviraptor.assets',project.id).permissions,[]);assert.deepEqual(f.ownerCalls,[]);
});

test('cancelling an authorization wait still terminates before any runtime prompt or business call',async t=>{
 const f=await partialAuthorizationFixture(t),{host,project,conversation}=f;
 host.apps.setGrants('org.oviraptor.assets',project.id,['inventory.read'],0,host.runtimeIdentity());
 const before=host.store.grant('org.oviraptor.assets',project.id);
 const sending=f.send('核对当前项目证书任务，再启动批量证书补采');await f.waitForApproval();
 await host.cancel(conversation.id);await sending;
 assert.equal(f.prompts.length,0);assert.deepEqual(f.ownerCalls,[]);
 assert.deepEqual(host.store.grant('org.oviraptor.assets',project.id),before);
 assert.equal(host.store.messages(conversation.id).at(-1).status,'cancelled');
 assert.equal(host.status().authorizations.length,0);
});
