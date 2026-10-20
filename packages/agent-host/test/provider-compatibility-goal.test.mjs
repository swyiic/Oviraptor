import test from 'node:test';
import assert from 'node:assert/strict';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
const original='org.oviraptor.assets__inventory_stats';
const config={provider:'openai-compatible',baseUrl:'https://api.deepseek.com',model:'deepseek-v4-flash',apiKey:'synthetic-test-key'};
const response=value=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify(value)}\n\n`;yield 'data: [DONE]\n\n';})()});
const output=text=>response({choices:[{delta:{content:text},finish_reason:'stop'}]});
const session=runtime=>runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'test-scope'},{name:'OVIRAPTOR_CONVERSATION',value:'test-conversation'}]}]});
test('DeepSeek receives legal tool aliases, auto choice, and private reasoning on the second tool step',async()=>{
 const requests=[],executed=[],events=[];let n=0;
 const runtime=new OpenAiCompatibleRuntime({config,onEvent:event=>events.push(event),fetch:async(_url,init)=>{
  const body=JSON.parse(init.body);requests.push(body);
  assert(body.tools.every(tool=>/^[a-zA-Z0-9_-]{1,64}$/.test(tool.function.name)),'tool names must obey the actual provider contract');
  assert.notEqual(body.tool_choice,'required','thinking mode rejects required choice');
  assert.equal(body.thinking?.type,'enabled');
  if(++n===1)return response({choices:[{delta:{reasoning_content:'private provider state',tool_calls:[{index:0,id:'read-1',function:{name:body.tools[0].function.name,arguments:'{}'}}]},finish_reason:'tool_calls'}]});
  assert.equal(body.messages.find(message=>message.tool_calls)?.reasoning_content,'private provider state');
  assert(!JSON.stringify(body.tools).includes('_hostName'));
  return output('根据 inventory_stats，当前有 3 个资产。');
 }});
 runtime.bindHost({async mcp(_token,_conversation,method,params){if(method==='tools/list')return{tools:[{name:original}]};executed.push(params.name);return{structuredContent:{total:3},content:[{type:'text',text:'{"total":3}'}]};}});
 const active=await session(runtime),result=await runtime.prompt({sessionId:active.sessionId,prompt:'当前项目有多少资产？',userText:'当前项目有多少资产？'});
 assert.equal(result.stopReason,'end_turn');assert.deepEqual(executed,[original]);assert.equal(requests.length,2);
 assert(!JSON.stringify(events).includes('private provider state'),'private provider state must never enter UI events');
});
test('a new question uses its own goal while retaining the durable task context',async()=>{
 const requests=[];
 const runtime=new OpenAiCompatibleRuntime({config:{...config,baseUrl:'https://fixture.invalid/v1',model:'fixture-model'},fetch:async(_url,init)=>{requests.push(JSON.parse(init.body));return output('CNAME 是域名别名记录。');}});
 const active=await session(runtime);
 const result=await runtime.prompt({sessionId:active.sessionId,prompt:'用一句话解释 CNAME。',userText:'用一句话解释 CNAME。',taskContext:{goal:'统计项目中所有证书种子'}});
 assert.match(result.harness.goal,/用一句话解释 CNAME/);assert.equal(requests.length,1,'a conceptual question must not loop on the old data request');
});
test('data verification never forces a write or advisory task tool when a project read exists',async()=>{
 let calls=0;
 const runtime=new OpenAiCompatibleRuntime({config:{...config,baseUrl:'https://fixture.invalid/v1',model:'fixture-model'},fetch:async(_url,init)=>{
  const body=JSON.parse(init.body);assert.deepEqual(body.tools.map(tool=>tool.function.name),['inventory_stats']);calls++;
  return output('当前查询不可用，未得到资产数量。');
 },maxSteps:1});
 runtime.bindHost({async mcp(){return{tools:[{name:'workspace_write'},{name:'host_task_read'},{name:'inventory_stats'}]};}});
 const active=await session(runtime);await runtime.prompt({sessionId:active.sessionId,prompt:'当前有多少资产？',userText:'当前有多少资产？'});assert.equal(calls,1);
});

test('an exact failure executes at most twice and cannot burn the whole turn in an identical loop',async()=>{
 let executed=0,requests=0;
 const runtime=new OpenAiCompatibleRuntime({config:{...config,baseUrl:'https://fixture.invalid/v1',model:'fixture-model'},fetch:async()=>{requests++;return response({choices:[{delta:{tool_calls:[{index:0,id:'retry-'+requests,function:{name:'inventory_stats',arguments:'{}'}}]},finish_reason:'tool_calls'}]});}});
 runtime.bindHost({async mcp(_token,_conversation,method){if(method==='tools/list')return{tools:[{name:'inventory_stats'}]};executed++;throw Object.assign(Error('temporary read unavailable'),{code:'host_app_tool_rejected'});}});
 const active=await session(runtime),result=await runtime.prompt({sessionId:active.sessionId,prompt:'当前有多少资产？',userText:'当前有多少资产？'});
 assert.equal(executed,2);assert.equal(requests,3);assert.equal(result.stopReason,'tool_loop_stalled');assert.match(result.assistantText,/未取得所需结果/);
});
test('a failed read can recover through another read without poisoning successful evidence',async()=>{
 let requests=0;const executed=[];
 const runtime=new OpenAiCompatibleRuntime({config:{...config,baseUrl:'https://fixture.invalid/v1',model:'fixture-model'},fetch:async()=>{requests++;return requests<3?response({choices:[{delta:{tool_calls:[{index:0,id:'read-'+requests,function:{name:requests===1?'inventory_stats':'project_overview',arguments:'{}'}}]},finish_reason:'tool_calls'}]}):output('根据 project_overview，当前有 3 个资产。');}});
 runtime.bindHost({async mcp(_token,_conversation,method,params){if(method==='tools/list')return{tools:[{name:'inventory_stats'},{name:'project_overview'}]};executed.push(params.name);if(params.name==='inventory_stats')throw Object.assign(Error('unavailable'),{code:'host_app_tool_rejected'});return{content:[{type:'text',text:'{"total":3}'}]};}});
 const active=await session(runtime),result=await runtime.prompt({sessionId:active.sessionId,prompt:'当前有多少资产？',userText:'当前有多少资产？'});
 assert.deepEqual(executed,['inventory_stats','project_overview']);assert.equal(result.stopReason,'end_turn');assert.equal(result.replaceText,undefined);
});

test('a simple read never advertises task-criteria mutations after the data result is already available',async()=>{
 let requests=0;const names=[];
 const runtime=new OpenAiCompatibleRuntime({config:{...config,baseUrl:'https://fixture.invalid/v1',model:'fixture-model'},fetch:async(_url,init)=>{
  const body=JSON.parse(init.body);assert(body.tools.every(tool=>tool.function.name==='inventory_stats'));requests++;
  return requests===1?response({choices:[{delta:{tool_calls:[{index:0,id:'stats',function:{name:'inventory_stats',arguments:'{}'}}]},finish_reason:'tool_calls'}]}):output('根据 inventory_stats，当前有 3 个资产。');
 }});
 runtime.bindHost({async mcp(_token,_conversation,method,params){if(method==='tools/list')return{tools:[{name:'inventory_stats'},{name:'host_task_criteria'},{name:'host_task_update'}]};names.push(params.name);return{content:[{type:'text',text:'{"total":3}'}]};}});
 const active=await session(runtime);await runtime.prompt({sessionId:active.sessionId,prompt:'当前有多少资产？',userText:'当前有多少资产？'});
 assert.deepEqual(names,['inventory_stats']);assert.equal(requests,2);
});
