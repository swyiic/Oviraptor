import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,writeFile,mkdir,rm,realpath} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {AgentHost} from '../src/service.mjs';
import {serveHost} from '../src/server.mjs';
import {createDirectoryPackage} from '../../app-sdk/src/index.mjs';
const repo=fileURLToPath(new URL('../../../',import.meta.url));
const tick=()=>new Promise(setImmediate);
async function fixture(t){
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-access-policy-')),events=[],decisions=[];
 const runtime={status:{pid:4242},async close(){},async cancel(){},async resolvePermission(...args){decisions.push(args);return true;}};
 const host=new AgentHost({dataDir:root,runtime,onEvent:e=>events.push(e)});
 let cleanupHost=()=>host.close();t.after(async()=>{await cleanupHost();await rm(root,{recursive:true,force:true});});
 const spec=JSON.parse(await readFile(join(repo,'examples/echo-app/app.package.json'),'utf8'));
 const pkg=await createDirectoryPackage({...spec,sourceDirectory:join(repo,'examples/echo-app'),outputDirectory:join(root,'package')});
 await host.apps.install(pkg.directory);await host.apps.setEnabled(spec.manifest.id,true);
 const project=host.store.createProject('A'),other=host.store.createProject('B'),conversation=host.store.createConversation(project.id,'A');
 const mode=(value,overrides={})=>host.setAccessPolicy({projectId:project.id,mode:value,operatorConfirmed:true,expectedRevision:host.accessPolicyView(project.id).revision,expectedRuntimeIdentity:host.runtimeIdentity(),...overrides});
 const run={id:'run-one',sessionId:'session',conversationId:conversation.id,state:'running',userText:'temporary test',text:'',message:{id:'message'},cancelled:false};host.runs.set(conversation.id,run);host.mcpScopes.set('scope',{conversationId:conversation.id,runtimePid:4242});
 const call=(name,args={})=>host.mcp('scope',conversation.id,'tools/call',{name,arguments:args});
 return{host,root,events,decisions,project,other,conversation,run,mode,call,setCleanup:fn=>{cleanupHost=fn;},id:spec.manifest.id};
}
test('full policy exposes actual enabled tools and file operations, without changing manual grants or another project',async t=>{
 const f=await fixture(t);assert.equal(f.host.apps.capabilities(f.project.id,{runtimeIdentity:f.host.runtimeIdentity()}).length,0);
 f.mode('full');assert.equal(f.host.apps.capabilities(f.project.id,{runtimeIdentity:f.host.runtimeIdentity()}).length,2);
 assert.deepEqual(f.host.store.grant(f.id,f.project.id).permissions,[]);assert.equal(f.host.accessPolicyView(f.other.id).mode,'ask');
 assert.equal(f.host.apps.capabilities(f.other.id,{runtimeIdentity:f.host.runtimeIdentity()}).length,0);
 await f.call('workspace_write',{path:'proof.txt',content:'local write'});assert.match((await f.call('workspace_read',{path:'proof.txt'})).content[0].text,/local write/);
 await f.call('org.oviraptor.echo__echo',{text:'actual worker'});assert.match(JSON.stringify(await f.call('org.oviraptor.echo__history')),/actual worker/);
 assert.equal(f.events.filter(e=>e.type==='authorization.requested').length,0);
 await assert.rejects(f.call('workspace_read',{path:'../outside'}),e=>e.code==='host_workspace_path_denied');
 await assert.rejects(f.call('workspace_run',{command:'node',args:[]}),e=>e.code==='host_workspace_execution_invalid');
 const next=f.host.store.createConversation(f.project.id,'new chat');
 const nextRun={...f.run,id:'run-two',conversationId:next.id};f.host.runs.set(next.id,nextRun);f.host.mcpScopes.set('second-scope',{conversationId:next.id,runtimePid:4242});
 assert.match(JSON.stringify(await f.host.mcp('second-scope',next.id,'tools/call',{name:'workspace_read',arguments:{path:'proof.txt'}})),/local write/);
 assert.deepEqual(f.host.permissionOffer(next.id,['org.oviraptor.echo__history']).offers,[]);
 f.mode('ask');assert.equal(f.host.apps.capabilities(f.project.id,{runtimeIdentity:f.host.runtimeIdentity()}).length,0);
 const reading=f.call('workspace_read',{path:'proof.txt'});const request=f.events.filter(e=>e.type==='authorization.requested').at(-1);assert.ok(request);
 f.host.resolveAuthorization(f.conversation.id,false,{requestId:request.requestId,runId:request.runId});await assert.rejects(reading,e=>e.code==='host_tool_permission_denied');
});
test('switching to full resolves an already displayed approval; withdrawing it does not leave a hidden file grant',async t=>{
 const f=await fixture(t);const dir=join(f.root,'workspaces',String(f.project.id),'agent');await mkdir(dir,{recursive:true});await writeFile(join(dir,'proof.txt'),'read');
 const reading=f.call('workspace_read',{path:'proof.txt'});const request=f.events.find(e=>e.type==='authorization.requested');assert.ok(request);
 f.mode('full');await reading;assert.ok(f.events.some(e=>e.type==='authorization.resolved'&&e.requestId===request.requestId));assert.equal(f.host.status().authorizations.length,0);
 f.mode('ask');const again=f.call('workspace_read',{path:'proof.txt'});const next=f.events.filter(e=>e.type==='authorization.requested').at(-1);assert.notEqual(next.requestId,request.requestId);f.host.resolveAuthorization(f.conversation.id,false,{requestId:next.requestId,runId:next.runId});await assert.rejects(again,e=>e.code==='host_tool_permission_denied');
});
test('full access requires explicit operator consent, exact recipient and revision; mode persists over restart',async t=>{
 const f=await fixture(t);assert.throws(()=>f.mode('full',{operatorConfirmed:false}),e=>e.code==='host_access_policy_confirmation_required');assert.throws(()=>f.mode('full',{expectedRuntimeIdentity:'other'}),e=>e.code==='host_runtime_identity_changed');assert.throws(()=>f.mode('full',{expectedRevision:12}),e=>e.code==='host_access_policy_conflict');
 f.mode('full');f.host.runs.clear();await f.host.close();const reopened=new AgentHost({dataDir:f.root,runtime:{status:{pid:4243},async close(){}}});f.setCleanup(()=>reopened.close());assert.equal(reopened.accessPolicyView(f.project.id).mode,'full');assert.equal(reopened.accessPolicyView(f.other.id).mode,'ask');
});
test('same authenticated provider model changes retain consent; a different recipient invalidates it',async t=>{
 const f=await fixture(t);f.host.runs.clear();const base={provider:'openai-compatible',baseUrl:'https://one.invalid/v1',model:'first',apiKey:'synthetic-key'};
 await f.host.setProvider(base);await f.host.setRuntime('xai-api');f.mode('full');const identity=f.host.runtimeIdentity();
 await f.host.setProvider({...base,model:'second'});assert.equal(f.host.runtimeIdentity(),identity);assert.equal(f.host.accessPolicyView(f.project.id).mode,'full');
 await f.host.setProvider({...base,baseUrl:'https://other.invalid/v1'});assert.notEqual(f.host.runtimeIdentity(),identity);assert.equal(f.host.accessPolicyView(f.project.id).mode,'ask');assert.equal(f.host.accessPolicyView(f.project.id).needsReview,true);
});
test('policy changes invalidate an in-flight app authority witness without replaying its operation',async t=>{
 const f=await fixture(t);f.mode('full');const worker=f.host.apps.workers.get(f.id);let release,started;const pending=new Promise(r=>release=r),entered=new Promise(r=>started=r);let calls=0;worker.call=async()=>{calls++;started();return pending;};
 const result=f.host.apps.call(f.id,'history',f.project.id,{},0,'one-operation',undefined,f.host.runtimeIdentity());await entered;f.mode('ask');release({content:[{type:'text',text:'already read'}]});await assert.rejects(result,e=>e.code==='host_tool_authority_changed');assert.equal(calls,1);
});
test('runtime full access chooses allow-once, and Ask mode never auto-resolves the next request',async t=>{
 const f=await fixture(t);f.mode('full');const event={type:'permission_requested',sessionId:'session',turnId:f.run.id,requestId:'first',options:[{optionId:'one',kind:'allow_once',name:'Allow once'},{optionId:'all',kind:'allow_always',name:'Always'}]};
 f.host.runtimeEvent(event);await tick();assert.deepEqual(f.decisions,[['first','one']]);f.mode('ask');f.host.runtimeEvent({...event,requestId:'second'});await tick();assert.equal(f.decisions.length,1);assert.ok(f.host.status().permissions.some(x=>x.id==='second'));
});
test('access-policy HTTP shares project state and rejects unauthenticated, stale and unconfirmed updates',async t=>{
 const f=await fixture(t),web=await serveHost(f.host);t.after(()=>web.close());const url=web.origin+'/api/runtime/access-policy';
 assert.equal((await fetch(url+'?projectId='+f.project.id)).status,403);
 const headers={'X-Oviraptor-Session':web.token,'Content-Type':'application/json'};
 const get=()=>fetch(url+'?projectId='+f.project.id,{headers}).then(r=>r.json());assert.equal((await get()).mode,'ask');
 const body={projectId:f.project.id,mode:'full',expectedRevision:0,expectedRuntimeIdentity:f.host.runtimeIdentity(),operatorConfirmed:true};assert.equal((await fetch(url,{method:'POST',headers,body:JSON.stringify(body)})).status,200);assert.equal((await get()).mode,'full');assert.equal((await fetch(url,{method:'POST',headers,body:JSON.stringify(body)})).status,409);
});
test('full access never bypasses a hard tool budget or silently includes a newly declared permission',async t=>{
 const f=await fixture(t);f.mode('full');const app=f.host.store.application(f.id);const manifest=structuredClone(app.manifest);manifest.permissions.push({id:'new.execute',description:'new permission'});manifest.tools.push({...manifest.tools[0],name:'new_tool',permission:'new.execute'});f.host.store.db.prepare('UPDATE applications SET manifest=? WHERE id=?').run(JSON.stringify(manifest),f.id);
 assert.equal(f.host.apps.capabilities(f.project.id,{runtimeIdentity:f.host.runtimeIdentity()}).some(t=>t.permission==='new.execute'),false);
 assert.equal(f.host.permissionOffer(f.conversation.id,[f.id+'__new_tool']).offers[0].permissions[0].permission,'new.execute');
 f.host.store.setToolCallBudget(f.project.id,0);await assert.rejects(f.call('org.oviraptor.echo__history'),e=>e.code==='host_budget_hard_stop');
});
