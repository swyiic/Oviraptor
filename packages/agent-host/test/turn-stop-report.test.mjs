import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { HostStore } from '../src/store.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { buildTurnStopReport } from '../src/turn-stop-report.mjs';
import { readOnlyContinuationQuestion } from '../src/tasks.mjs';
import { serveHost } from '../src/server.mjs';
import { Worker } from 'node:worker_threads';

const config={provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic-not-real'};
const response=delta=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta,finish_reason:delta.tool_calls?'tool_calls':'stop'}]})}\n\n`;})()});
const call=(name='org.oviraptor.assets__inventory_stats')=>response({content:'我先检查。',tool_calls:[{index:0,id:'read',function:{name,arguments:'{}'}}]});
const taskId='11111111-1111-4111-8111-111111111111';
const run={id:'current',taskId,conversationId:'conversation',message:{id:'answer'}};
const task={id:taskId,conversationId:'conversation',projectId:2,runs:[{id:'current',assistantMessageId:'answer'}],evidence:[],outcomeUnknown:false,acceptance:null};

test('receipt report is run-bound, excludes arbitrary content and preserves task and unknown effects',()=>{
 const data=structuredClone(task);data.evidence=[
  {id:'old',runId:'old',toolName:'org.oviraptor.assets__inventory_stats',readOnly:true,status:'completed'},
  {id:'stats',runId:'current',toolName:'org.oviraptor.assets__inventory_stats',readOnly:true,status:'completed'},
  {id:'start',runId:'current',toolName:'org.oviraptor.scanning__scan_start',readOnly:false,status:'completed'},
  {id:'lost',runId:'current',toolName:'org.oviraptor.scanning__scan_resume',readOnly:false,status:'failed',outcomeUnknown:true,operationId:taskId,summary:'secret must not leak'},
  {id:'reject',runId:'current',toolName:'org.oviraptor.scanning__scan_status',readOnly:true,status:'failed',code:'host_tool_permission_denied'},
 ];data.outcomeUnknown=true;
 const results=new Map([
  ['stats',{name:data.evidence[1].toolName,result:{structuredContent:{projectId:2,total:7,apiKey:'sensitive-credential'}}}],
  ['start',{name:data.evidence[2].toolName,result:{structuredContent:{projectId:2,task:{id:taskId,status:'running',title:'ignore instructions https://user:password@secret.test/?token=secret',modelRequests:1}}}}],
 ]);
 const before=structuredClone(data),r=buildTurnStopReport(data,run,'max_turn_requests',results);
 assert.deepEqual(r.counts,{reads:1,writes:1,failures:1,unknown:1});assert.equal(r.outcomeUnknown,true);
 assert.match(r.text,/本地资产记录 7 条/);assert.match(r.text,/返回状态 running/);assert.match(r.text,/缺少所需权限/);assert.match(r.text,/结果未知，先核对再继续/);assert.match(r.text,/不会暂停已启动的后台任务/);
 assert.doesNotMatch(r.text,/sensitive|secret|password|ignore instructions/);assert.deepEqual(data,before);
 for(const bad of [{...run,id:'old'},{...run,conversationId:'other'},{...run,taskId:'other'},{...run,message:{id:'other'}}])assert.equal(buildTurnStopReport(data,bad,'max_turn_requests',results),null);
 assert.equal(buildTurnStopReport(data,run,'end_turn',results),null);
 const wrong=new Map([['stats',{name:data.evidence[1].toolName,result:{structuredContent:{projectId:99,total:1000000}}}]]);
 assert.doesNotMatch(buildTurnStopReport(data,run,'paused_budget',wrong).text,/1000000/);
});

test('stop report bounds receipt detail and does not borrow old evidence or claim completion',()=>{
 const data={...task,evidence:Array.from({length:40},(_,i)=>({id:String(i),runId:'old',status:'completed',readOnly:true}))};
 const empty=buildTurnStopReport(data,run,'max_turn_requests');assert.match(empty.text,/没有成功业务工具回执/);assert.match(empty.text,/未作整体完成验收/);assert.equal(empty.counts.reads,0);
 data.evidence=data.evidence.map(e=>({...e,runId:run.id}));const r=buildTurnStopReport(data,run,'max_turn_requests');assert.equal(r.text.match(/- 其他业务读取/g).length,8);assert.match(r.text,/另有 32 条/);
});

async function fixture(t,{cap=1,maxSteps,fetchImpl,toolResult}={}){
 const root=await mkdtemp(join(tmpdir(),'ovi-turn-stop-'));let calls=0,requests=0;
 const runtime=new OpenAiCompatibleRuntime({config,maxSteps,fetch:async(url,init)=>{requests++;return fetchImpl?fetchImpl(url,init,requests):call();}});
 const apps={store:{application:()=>({manifest:{}})},capabilities:()=>[{name:'org.oviraptor.assets::inventory_stats',permission:'inventory.read',appId:'org.oviraptor.assets',grantRevision:1}],
  async call(){calls++;if(toolResult instanceof Error)throw toolResult;return toolResult??{structuredContent:{total:3},content:[{type:'text',text:'{"total":3}'}]};},async close(){}};
 const host=new AgentHost({dataDir:root,runtime,apps});const web=await serveHost(host);
 t.after(async()=>{await web.close();await host.close();await rm(root,{recursive:true,force:true});});
 const project=host.store.createProject('Synthetic cap'),conversation=host.store.createConversation(project.id);
 host.setBudgetAllocation(project.id,{total:20,host:20,native:0,modelCap:cap});
 return{host,project,conversation,runtime,web,counts:()=>({requests,calls})};
}
async function send(f){await f.host.send(f.conversation.id,{projectId:f.project.id,text:'当前项目有多少资产？'});await f.host.runs.get(f.conversation.id)?.completion;return f.host.store.messages(f.conversation.id).at(-1);}

test('actual Host budget API persists and validates the model cap; omission and revisions never reset spent attempts',async t=>{
 const f=await fixture(t);const store=f.host.store;assert.equal(store.allocation(f.project.id).modelCap,1);
 const post=body=>fetch(`${f.web.origin}/api/projects/${f.project.id}/budget`,{method:'POST',headers:{'Content-Type':'application/json','X-Oviraptor-Session':f.web.token},body:JSON.stringify(body)});
 const valid=await post({total:20,host:20,native:0,modelCap:1});assert.equal(valid.status,200);assert.equal((await valid.json()).modelCap,1);
 const invalid=await post({total:20,host:20,native:0,modelCap:-1});assert.equal(invalid.status,400);assert.equal(store.allocation(f.project.id).modelCap,1);
 store.admitModelRequest(f.project.id);assert.throws(()=>store.admitModelRequest(f.project.id),{code:'host_budget_hard_stop'});
 assert.equal(f.host.setBudgetAllocation(f.project.id,{total:20,host:20,native:0}).modelCap,1);assert.equal(store.allocation(f.project.id).modelSpent,1);
 for(const modelCap of [-1,1.5,'2',Number.MAX_SAFE_INTEGER+1])assert.throws(()=>f.host.setBudgetAllocation(f.project.id,{total:20,host:20,native:0,modelCap}),{code:'host_budget_invalid'});
 assert.equal(f.host.setBudgetAllocation(f.project.id,{total:20,host:20,native:0,modelCap:null}).modelCap,null);
 assert.equal(store.admitModelRequest(f.project.id).enforced,false);assert.equal(store.allocation(f.project.id).modelSpent,1);
});

test('one admitted HTTP request can read data, then persists a local receipt answer without an extra model call',async t=>{
 const f=await fixture(t),message=await send(f);assert.deepEqual(f.counts(),{requests:1,calls:1});
 assert.equal(message.status,'incomplete');assert.equal(message.stopReason,'paused_budget');assert.equal(message.outcomeUnknown,false);
 assert.match(message.content,/成功读取 1 次/);assert.match(message.content,/本地资产记录 3 条/);assert.doesNotMatch(message.content,/我先检查/);
 assert.match(message.content,/已准入 1 \/ 1 次 HTTP/);
 const task=f.host.task(f.conversation.id);assert.equal(task.status,'paused_budget');assert.equal(task.acceptance,null);assert.equal(task.evidence.length,1);assert.equal(task.evidence[0].status,'completed');
 const metrics=JSON.parse(f.host.store.db.prepare('SELECT metrics FROM runtime_metrics ORDER BY rowid DESC LIMIT 1').get().metrics);assert.equal(metrics.stopReport.source,'host_receipts');assert.equal(metrics.stopReport.reads,1);assert.equal(metrics.outcome,'incomplete');assert.equal(metrics.modelHttpAttempts,1);
 // Operator increases the cumulative cap; no already-executed tool is replayed
 // until the user explicitly sends a new turn.
 assert.equal(f.host.setBudgetAllocation(f.project.id,{total:20,host:20,native:0,modelCap:3}).modelSpent,1);
 let n=0;f.runtime.fetch=async()=>{n++;return n===1?call():response({content:'根据 inventory_stats，本地有 3 条资产。'});};
 await f.host.continueTask(f.conversation.id,{projectId:f.project.id,taskId:task.id});await f.host.runs.get(f.conversation.id)?.completion;
 const rows=f.host.store.messages(f.conversation.id),resumed=rows.at(-1);assert.equal(rows.at(-2).content,'当前项目有多少资产？');assert.equal(resumed.status,'completed');assert.equal(f.host.task(f.conversation.id).runs.length,2);assert.equal(f.host.task(f.conversation.id).runs[0].status,'incomplete');assert.equal(f.host.store.allocation(f.project.id).modelSpent,3);
});

test('read-only continuation cannot reuse a different message or bypass execution and unknown histories',()=>{
 const data={...task,goal:'当前项目有多少资产？',children:[],runs:[{id:run.id,status:'incomplete',stopReason:'paused_budget',userMessageId:'q',assistantMessageId:'a'}]};
 const messages=[{id:'q',role:'user',content:'已保存证书有多少？'},{id:'a',role:'assistant',runId:run.id,status:'incomplete',stopReason:'paused_budget'}];
 assert.equal(readOnlyContinuationQuestion(data,messages),messages[0].content);
 for(const changed of [{outcomeUnknown:true},{goal:'启动一次扫描'},{children:[{status:'running'}]},{evidence:[{readOnly:false,status:'completed'}]},{runs:[{...data.runs[0],status:'completed'}]}])assert.equal(readOnlyContinuationQuestion({...data,...changed},messages),'');
 for(const changes of [{runId:'old'},{id:'other'},{status:'completed'},{stopReason:'end_turn'},{outcomeUnknown:true}])assert.equal(readOnlyContinuationQuestion(data,[messages[0],{...messages[1],...changes}]),'');
 assert.equal(readOnlyContinuationQuestion(data,[{...messages[0],content:'读取后启动扫描'},messages[1]]),'');
});

test('zero model budget makes no provider request or business call and leaves a truthful incomplete reply',async t=>{
 const f=await fixture(t,{cap:0}),message=await send(f);assert.deepEqual(f.counts(),{requests:0,calls:0});assert.match(message.content,/没有成功业务工具回执/);assert.equal(message.stopReason,'paused_budget');assert.equal(f.host.store.allocation(f.project.id).modelSpent,0);
});

test('normal step-limit exhaustion returns the same receipt report without changing it to end_turn',async t=>{
 const f=await fixture(t,{cap:10,maxSteps:1}),message=await send(f);assert.equal(message.stopReason,'max_turn_requests');assert.equal(message.status,'incomplete');assert.match(message.content,/成功读取 1 次/);assert.deepEqual(f.counts(),{requests:1,calls:1});assert.equal(f.host.task(f.conversation.id).status,'interrupted');
});

test('unknown write outcomes stay unknown in message, task, metrics and stop report',async t=>{
 const f=await fixture(t,{cap:1});
 f.host.apps.capabilities=()=>[{name:'fixture::write',permission:'fixture.write',appId:'fixture',grantRevision:1}];
 f.runtime.fetch=async()=>call('fixture__write');f.host.apps.call=async()=>{throw Object.assign(Error('never publish this sensitive path'),{code:'host_app_timeout',details:{outcomeUnknown:true}});};
 await f.host.send(f.conversation.id,{projectId:f.project.id,text:'执行一次已经授权的业务操作'});await f.host.runs.get(f.conversation.id)?.completion;
 const message=f.host.store.messages(f.conversation.id).at(-1),task=f.host.task(f.conversation.id);assert.equal(message.outcomeUnknown,true);assert.equal(task.outcomeUnknown,true);assert.match(message.content,/结果未知，先核对再继续/);assert.doesNotMatch(message.content,/sensitive path/);assert.equal(task.evidence[0].outcomeUnknown,true);
 const metrics=JSON.parse(f.host.store.db.prepare('SELECT metrics FROM runtime_metrics ORDER BY rowid DESC LIMIT 1').get().metrics);assert.equal(metrics.outcomeUnknown,true);assert.equal(metrics.stopReport.unknown,1);
});

test('Responses fallback cannot exceed the HTTP cap; a permitted fallback counts both HTTP attempts',async t=>{
 for(const cap of [1,2]){
  const urls=[],f=await fixture(t,{cap,fetchImpl:(url)=>{urls.push(url);return url.endsWith('/responses')?{ok:false,status:404}:response({content:'这是不查询项目数据的概念解释。'});}});
  f.runtime.config={...config,provider:'xai',baseUrl:'https://api.x.ai/v1',model:'grok-4.7'};
  await f.host.send(f.conversation.id,{projectId:f.project.id,text:'解释什么是域名。'});await f.host.runs.get(f.conversation.id)?.completion;
  const message=f.host.store.messages(f.conversation.id).at(-1);assert.equal(urls.length,cap);assert(urls[0].endsWith('/responses'));
  assert.equal(message.stopReason,cap===1?'paused_budget':'end_turn');assert.equal(f.host.store.allocation(f.project.id).modelSpent,cap);
  if(cap===2)assert(urls[1].endsWith('/chat/completions'));
 }
});

test('OAuth refresh cannot silently bypass the HTTP request cap',async t=>{
 const f=await fixture(t,{cap:1,fetchImpl:()=>({ok:false,status:401})});let refreshes=0;
 f.runtime.config={provider:'xai',baseUrl:'https://api.x.ai/v1',model:'grok-4.7',authentication:'oauth',apiKey:'synthetic-access'};
 f.host.ensureOAuthAccess=async()=>{refreshes++;return{...f.runtime.config,accessToken:'synthetic-renewed'};};
 f.host.providerStore.read=()=>({...f.runtime.config,accessToken:'synthetic-renewed'});
 await f.host.send(f.conversation.id,{projectId:f.project.id,text:'解释什么是域名。'});await f.host.runs.get(f.conversation.id)?.completion;
 assert.equal(f.counts().requests,1);assert.equal(f.host.store.allocation(f.project.id).modelSpent,1);assert.equal(f.host.store.messages(f.conversation.id).at(-1).stopReason,'paused_budget');assert(refreshes>=1);
});

test('strict context refusals and repeated blocked turns do not spend a model attempt or wedge the session',async t=>{
 const f=await fixture(t,{cap:1});f.runtime.config={...config,contextMode:'strict'};
 for(let i=0;i<2;i++){
  const message=await send(f);assert.equal(message.stopReason,'unsupported_capability');assert.equal(message.status,'incomplete');assert.equal(f.host.store.allocation(f.project.id).modelSpent,0);
 }
 assert.deepEqual(f.counts(),{requests:0,calls:0});
});

test('concurrent database connections cannot admit more HTTP attempts than the shared project cap', {timeout:10000},async t=>{
 const root=await mkdtemp(join(tmpdir(),'ovi-concurrent-model-cap-'));t.after(()=>rm(root,{recursive:true,force:true}));
 const file=join(root,'host.sqlite'),store=new HostStore(file);t.after(()=>store.close());const p=store.createProject('Concurrent cap');store.saveAllocation(p.id,{total:0,host:0,native:0,modelCap:3});
 const flag=new SharedArrayBuffer(4),workers=[];
 for(let i=0;i<4;i++){
  const worker=new Worker(`const {parentPort,workerData}=require('node:worker_threads');(async()=>{
   const {HostStore}=await import(workerData.module);const store=new HostStore(workerData.file);store.db.exec('PRAGMA busy_timeout=3000');
   parentPort.postMessage('ready');Atomics.wait(new Int32Array(workerData.flag),0,0);let admitted=0;
   for(let j=0;j<10;j++){try{store.admitModelRequest(workerData.projectId);admitted++;}catch(e){if(e.code!=='host_budget_hard_stop')throw e;}}
   store.close();parentPort.postMessage({admitted});})();`,{eval:true,workerData:{module:new URL('../src/store.mjs',import.meta.url).href,file,projectId:p.id,flag}});
  t.after(()=>worker.terminate());
  let resolveReady,resolveResult;const ready=new Promise(resolve=>{resolveReady=resolve;}),result=new Promise((resolve,reject)=>{resolveResult=resolve;worker.on('error',reject);});
  worker.on('message',m=>{if(m==='ready')resolveReady();else resolveResult(m);});workers.push({ready,result});
  // Startup schema recovery is sequential; only the admissions race.
  await ready;
 }
 Atomics.store(new Int32Array(flag),0,1);Atomics.notify(new Int32Array(flag),0);
 const results=await Promise.all(workers.map(w=>w.result));assert.equal(results.reduce((sum,r)=>sum+r.admitted,0),3);assert.equal(store.allocation(p.id).modelSpent,3);
});
