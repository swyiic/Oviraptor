import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp,realpath,rm,readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { CertificateTasks } from '../src/certificate-tasks.mjs';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

test('private bridge maps the Host project and never accepts a model-supplied desktop project',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-certificate-bridge-'));const store=new HostStore(join(root,'host.sqlite'));const project=store.createProject('one',{externalId:'91'});let seen;
 const server=createServer(async(req,res)=>{let text='';for await(const chunk of req)text+=chunk;seen={body:JSON.parse(text),authorization:req.headers.authorization};res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,result:{projectId:91,tasks:[{id:'real-native-id',status:'paused'}]}}));});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 t.after(async()=>{await new Promise(resolve=>server.close(resolve));store.close();await rm(root,{recursive:true,force:true});});
 const bridge=new CertificateTasks({origin:`http://127.0.0.1:${server.address().port}`,token:'a'.repeat(64)});
 const output=await bridge.call(store,project.id,'certificate_task_start',{startPaused:true},'trusted-operation');
 assert.equal(seen.body.projectId,91);assert.equal(seen.body.operationId,'trusted-operation');assert.equal(seen.body.action,'start');assert.equal(seen.authorization,'Bearer '+'a'.repeat(64));assert.equal(output.structuredContent.tasks[0].status,'paused');assert(!JSON.stringify(output).includes('a'.repeat(64)));
 const unmapped=store.createProject('unmapped');await assert.rejects(bridge.call(store,unmapped.id,'certificate_task_status',{},'read'),{code:'host_certificate_project_unmapped'});
 for(const origin of ['https://127.0.0.1:80','http://example.com:80','http://127.0.0.1:80/path','http://u:p@127.0.0.1:80'])assert.equal(new CertificateTasks({origin,token:'a'.repeat(64)}).available,false);
});

test('real AppManager enforces dedicated collect grant and routes execution outside the sandboxed Node worker',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-certificate-host-'));const store=new HostStore(join(root,'host.sqlite'));const apps=new AppManager(store,join(root,'apps'));t.after(async()=>{await apps.close();store.close();await rm(root,{recursive:true,force:true});});
 const sourceDirectory=fileURLToPath(new URL('../../../apps/assets/',import.meta.url));const spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,'package')});
 await apps.install(pack.directory);await apps.setEnabled(spec.manifest.id,true);const project=store.createProject('bound',{externalId:'91'});const id=spec.manifest.id;let calls=[];
 apps.certificateTasks={available:true,async call(_store,p,name,args,operation){calls.push({p,name,args,operation});return{structuredContent:{projectId:91,tasks:[{id:'task',status:name.endsWith('pause')?'paused':'running'}]}};}};
 let grant=apps.setGrants(id,project.id,['inventory.read'],undefined,'runtime:1');
 assert(!apps.capabilities(project.id,{runtimeIdentity:'runtime:1'}).some(t=>t.name.endsWith('certificate_task_start')));
 assert(apps.capabilities(project.id,{runtimeIdentity:'runtime:1'}).some(t=>t.name.endsWith('certificate_task_status')));
 await assert.rejects(apps.call(id,'certificate_task_start',project.id,{},grant.revision,'deny'),{code:'host_tool_permission_denied'});assert.equal(calls.length,0);
 grant=apps.setGrants(id,project.id,['inventory.read','certificate.collect'],grant.revision,'runtime:1');
 await apps.call(id,'certificate_task_start',project.id,{startPaused:true},grant.revision,'start-op');
 await apps.call(id,'certificate_task_pause',project.id,{taskId:'task'},grant.revision,'pause-op');
 await apps.call(id,'certificate_task_resume',project.id,{taskId:'task'},grant.revision,'resume-op');
 assert.deepEqual(calls.map(c=>c.name),['certificate_task_start','certificate_task_pause','certificate_task_resume']);assert(calls.every(c=>c.p===project.id));
 await assert.rejects(apps.call(id,'certificate_task_start',project.id,{projectId:999},grant.revision,'spoof'));
 apps.certificateTasks.available=false;assert(!apps.capabilities(project.id,{runtimeIdentity:'runtime:1'}).some(t=>t.name.includes('certificate_task')));
});

test('Agent creates a paused certificate queue, checks the actual status once and exits without task-plan loops',async()=>{
 const {OpenAiCompatibleRuntime}=await import('../src/provider.mjs');
 const requests=[],calls=[];const names=['certificate_task_start','certificate_task_status','certificate_task_pause','certificate_task_resume'];
 const response=(name,args,text)=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:name?{tool_calls:[{index:0,id:'call-'+requests.length,function:{name:'assets__'+name,arguments:JSON.stringify(args)}}]}:{content:text},finish_reason:name?'tool_calls':'stop'}]})}\n\n`;})()});
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic-api-key'},fetch:async(_url,init)=>{
  const body=JSON.parse(init.body);requests.push(body);
  if(requests.length===1){assert(body.tools.some(t=>t.function.name==='assets__certificate_task_start'));assert(!body.tools.some(t=>/host_task|inventory_stats/.test(t.function.name)));return response('certificate_task_start',{startPaused:true});}
  if(requests.length===2){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__certificate_task_status']);return response('certificate_task_status',{taskId:'native-task'});}
  assert.equal(body.tools,undefined);return response(null,null,'证书任务 native-task 已创建并暂停，等待继续。');
 }});
 runtime.bindHost({async mcp(_token,_conversation,method,params){if(method==='tools/list')return{tools:[...names.map(name=>({name:'assets__'+name,inputSchema:{type:'object'}})),{name:'inventory_stats'},{name:'host_task_update'}]};calls.push(params.name);return{structuredContent:{projectId:3,tasks:[{id:'native-task',status:'paused',progress:{total:3,pending:3}}]}};}});
 const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'conversation'}]}]});
 const result=await runtime.prompt({sessionId,prompt:'给当前项目创建证书补采任务，先暂停，返回进度。',userText:'给当前项目创建证书补采任务，先暂停，返回进度。'});
 assert.equal(result.stopReason,'end_turn');assert.deepEqual(calls,['assets__certificate_task_start','assets__certificate_task_status']);assert.equal(requests.length,3);
});

test('certificate intent keeps status reads distinct from starting or resuming a batch',async()=>{
 const {certificateTaskIntent}=await import('../src/quality-prompt.mjs');
 assert.equal(certificateTaskIntent('查看批量补采证书任务进度').action,'status');
 assert.equal(certificateTaskIntent('恢复当前证书批量补采任务').action,'resume');
 assert.equal(certificateTaskIntent('暂停当前证书补采任务').action,'pause');
 assert.equal(certificateTaskIntent('如何暂停证书任务'),null);
 assert.equal(certificateTaskIntent('不要开始证书补采'),null);
 assert.equal(certificateTaskIntent('请查看当前证书任务进度，不启动任务').action,'status');
 assert.equal(certificateTaskIntent('创建证书补采任务，不要立即执行').paused,true);
 assert.equal(certificateTaskIntent('不要暂停当前证书任务').action,'status');
});
