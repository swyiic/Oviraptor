import test from 'node:test';
import assert from 'node:assert/strict';
import {exposureTaskIntent,isReadOnlyDataRequest} from '../src/quality-prompt.mjs';
import {exposureTurnTools} from '../src/exposure-turn.mjs';
import {OpenAiCompatibleRuntime} from '../src/provider.mjs';
const tool=name=>({name:'assets__'+name,_hostName:'assets__'+name,function:{name:'assets__'+name,parameters:{type:'object',properties:{runId:{type:'integer'}},additionalProperties:false}}});
const names=['exposure_task_start','exposure_task_status','exposure_task_resume','exposure_task_cancel','exposure_findings','exposure_finding','inventory_stats','host_task_update'];
const result=(name,value,ok=true)=>({name:'assets__'+name,ok,content:JSON.stringify({structuredContent:value})});
test('exposure intent keeps questions and explicit read-only requests from becoming collection controls',()=>{for(const value of ['暴露面采集启动了吗？','不要启动暴露面采集，只看状态','请只读梳理公开暴露面线索，不修改结论'])assert(['status','review'].includes(exposureTaskIntent(value).action),value);assert.equal(exposureTaskIntent('如何开始公开暴露面采集'),null);assert.equal(exposureTaskIntent('开始公开暴露面被动采集').action,'start');assert.equal(isReadOnlyDataRequest('请只读复核当前工作空间的公开暴露面，不启动采集'),true);});
test('exposure controls are offered once, status binds the actual run and an unknown control is never reoffered',()=>{const listed=names.map(tool),intent={action:'start'};assert.deepEqual(exposureTurnTools(intent,'开始公开暴露面采集',listed,[]).map(t=>t._hostName),['assets__exposure_task_start']);const started=[result('exposure_task_start',{runId:27,accepted:true,completed:false})];const next=exposureTurnTools(intent,'开始公开暴露面采集',listed,started);assert.deepEqual(next.map(t=>t._hostName),['assets__exposure_task_status']);assert.deepEqual(next[0].function.parameters.properties.runId.enum,[27]);assert.deepEqual(exposureTurnTools(intent,'开始公开暴露面采集',listed,[...started,result('exposure_task_status',{id:27,status:'running'})]),[]);assert(!exposureTurnTools(intent,'开始公开暴露面采集',listed,[result('exposure_task_start',{},false)]).some(t=>t._hostName.endsWith('start')));});
test('a requested older task can never be silently replaced by the latest run',()=>{const listed=names.map(tool),r=[result('exposure_task_status',{latest:{id:28,status:'running'}})];const next=exposureTurnTools({action:'cancel'},'停止暴露面任务 #27',listed,r);assert.deepEqual(next.map(t=>t._hostName),['assets__exposure_task_status']);assert.deepEqual(next[0].function.parameters.properties.runId.enum,[27]);});
const response=(name,args={},text='')=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:name?{tool_calls:[{index:0,id:'call-'+name,function:{name:'assets__'+name,arguments:JSON.stringify(args)}}]}:{content:text},finish_reason:name?'tool_calls':'stop'}]})}\n\n`;})()});
test('actual provider loop does exposure start -> one status -> answer without a task-plan or a duplicate start',async()=>{const requests=[],calls=[];const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',apiKey:'fixture',model:'fixture'},maxSteps:5,fetch:async(_url,options)=>{const body=JSON.parse(options.body);requests.push(body);if(requests.length===1){return response('exposure_task_start');}if(requests.length===2){return response('exposure_task_status',{runId:27});}return response(null,{},'任务 27 已受理，当前仍在后台运行，不能视为采集完成。');}});runtime.bindHost({async mcp(_token,_id,method,params){if(method==='tools/list')return{tools:names.map(name=>({name:'assets__'+name,inputSchema:{type:'object'}}))};calls.push(params.name);return{structuredContent:params.name.endsWith('start')?{runId:27,accepted:true,completed:false}:{id:27,status:'running'}};}});const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'c'}]}]});const done=await runtime.prompt({sessionId,prompt:'开始公开暴露面被动采集',userText:'开始公开暴露面被动采集'});assert.equal(done.stopReason,'end_turn');assert.deepEqual(requests.map(body=>body.tools?.map(t=>t.function.name)),[['assets__exposure_task_start'],['assets__exposure_task_status'],undefined]);assert.deepEqual(calls,['assets__exposure_task_start','assets__exposure_task_status']);});
test('long Chinese exposure start phrasing is execution, never misclassified as a read-only question',()=>{assert.equal(isReadOnlyDataRequest('开始当前项目公开暴露面被动采集'),false);assert.equal(isReadOnlyDataRequest('只查看公开暴露面任务状态，不开始采集'),true);});
test('future separately requested controls do not replace the current exposure command',()=>{
 const goal='开始当前项目的公开暴露面被动采集。只使用已批准的入口，不扩大范围。接收回执后只核对一次同一任务的状态；启动回执不等于采集完成。后续我会单独要求暂停和恢复。';
 assert.equal(exposureTaskIntent(goal).action,'start');
 assert.equal(isReadOnlyDataRequest(goal),false);
 assert.deepEqual(exposureTurnTools(exposureTaskIntent(goal),goal,names.map(tool),[]).map(t=>t._hostName),['assets__exposure_task_start']);
 assert.equal(exposureTaskIntent('暂停当前公开暴露面采集。稍后我将单独要求恢复。').action,'cancel');
 assert.equal(exposureTaskIntent('恢复公开暴露面任务 #27。之后我会另行要求停止。').action,'resume');
 assert.equal(exposureTaskIntent('只看公开暴露面状态。后续我会另行要求开始。').action,'status');
});
test('an unlisted model tool name gets one correction without repeating an executed exposure control',async()=>{
 const requests=[],calls=[];
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',apiKey:'fixture',model:'fixture'},maxSteps:6,fetch:async(_url,options)=>{
  requests.push(JSON.parse(options.body));
  if(requests.length===1)return response('inventory_query');
  if(requests.length===2)return response('exposure_task_start');
  if(requests.length===3)return response('exposure_task_status',{runId:27});
  return response(null,{},'任务 27 已受理，仍在执行。');
 }});
 runtime.bindHost({async mcp(_token,_id,method,params){
  if(method==='tools/list')return{tools:names.map(name=>({name:'assets__'+name,inputSchema:{type:'object'}}))};
  calls.push(params.name);return{structuredContent:params.name.endsWith('start')?{runId:27,accepted:true,completed:false}:{id:27,status:'running'}};
 }});
 const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'c'}]}]});
 const goal='开始公开暴露面被动采集。后续我会单独要求暂停和恢复。';
 const done=await runtime.prompt({sessionId,prompt:goal,userText:goal});
 assert.equal(done.stopReason,'end_turn');
 assert.deepEqual(calls,['assets__exposure_task_start','assets__exposure_task_status']);
 assert.deepEqual(requests.map(body=>body.tools?.map(t=>t.function.name)),[['assets__exposure_task_start'],['assets__exposure_task_start'],['assets__exposure_task_status'],undefined]);
});
