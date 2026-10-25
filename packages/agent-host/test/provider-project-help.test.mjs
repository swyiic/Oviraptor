// Exercise the production Provider loop with scripted SSE and a scoped Host
// contract. No provider, real project, native task or external network is used.
import test from 'node:test';
import assert from 'node:assert/strict';
import { OpenAiCompatibleRuntime, providerToolName } from '../src/provider.mjs';

const config={provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture',apiKey:'synthetic-project-help-key'};
const text=value=>({choices:[{delta:{content:value},finish_reason:'stop'}]});
const call=(name,args={})=>({name,arguments:JSON.stringify(args)});
const turn=calls=>({choices:[{delta:{tool_calls:calls.map((item,index)=>({index,id:`call-${index}-${item.name}`,function:item}))},finish_reason:'tool_calls'}]});
const stream=chunk=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify(chunk)}\n\n`;})()});
const schema=name=>({name,inputSchema:{type:'object',properties:{}}});
const offered=body=>body.tools?.map(tool=>tool.function.name)??[];
const assertPaired=messages=>assert.deepEqual(messages.filter(message=>message.role==='tool').map(message=>message.tool_call_id).sort(),
  messages.flatMap(message=>message.tool_calls??[]).map(item=>item.id).sort());

async function fixture(t,{tools=[],step,execute=async()=>({structuredContent:{total:3}}),permissionOffer=()=>({offers:[]}),authorize=()=>false,takeSteer=()=>[]}={}){
 const requests=[],calls=[],authorizations=[],offers=[];
 const runtime=new OpenAiCompatibleRuntime({config,fetch:async(_url,init)=>{const body=JSON.parse(init.body);requests.push(body);return stream(await step(body,requests.length));}});
 runtime.bindHost({takeSteer,async mcp(_token,_conversation,method,params){
   if(method==='tools/list')return{tools:(typeof tools==='function'?tools():tools).map(schema)};
   calls.push(params);return execute(params);
  },permissionOffer(_conversation,names){offers.push(names);return permissionOffer(names);},requestAuthorization(_conversation,details){authorizations.push(details);return authorize(details);}});
 const{sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'chat'}]}]});
 t.after(()=>runtime.close());
 return{runtime,sessionId,requests,calls,authorizations,offers,prompt:(userText,extra={})=>runtime.prompt({sessionId,prompt:userText,userText,...extra})};
}

test('project report reads retain granted workspace and Skill readers while hiding every mutation',async t=>{
 const readers=['workspace_list','workspace_read','skills_list','skills_read','skills_list_resources','skills_read_resource'];
 const f=await fixture(t,{tools:[...readers,'assets__inventory_stats','workspace_write','workspace_run','assets__inventory_import'],step:(body,n)=>{
  for(const reader of readers)assert(offered(body).includes(reader));
  for(const mutation of ['workspace_write','workspace_run','assets__inventory_import'])assert(!offered(body).includes(mutation));
  if(n===1)return turn([call('workspace_list')]);
  if(n===2)return turn([call('workspace_read',{path:'reports/certificate.md'})]);
  return text('报告说明 TLS 握手失败，尚无可读取的证书原文。');
 },execute:async params=>({content:[{type:'text',text:params.name==='workspace_list'?'reports/certificate.md':'TLS 握手失败，未保存证书原文。'}]})});
 const result=await f.prompt('读取当前项目的报告文件并给我摘要');
 assert.deepEqual(f.calls.map(item=>item.name),['workspace_list','workspace_read']);
 assert.equal(result.stopReason,'end_turn');assert.match(result.assistantText,/TLS/);
 assert.equal(f.authorizations.length,0);assert(f.requests.length<=4);
 for(const request of f.requests)assertPaired(request.messages);
});

test('inventory totals keep granted business readers when inventory_stats is unavailable, without using unrelated files',async t=>{
 for(const name of ['assets__project_overview','assets__saved_assets_stats','assets__inventory_query']){
  const f=await fixture(t,{tools:[name,'workspace_read','skills_read'],step:(body,n)=>{
   assert(offered(body).includes(name));assert(!offered(body).includes('workspace_read'));assert(!offered(body).includes('skills_read'));
   return n===1?turn([call(name)]):text(`根据 ${name} 返回，本地资产记录共 3 条。`);
  }});
  const result=await f.prompt('我的资产有几个？');
  assert.deepEqual(f.calls.map(item=>item.name),[name]);assert.equal(f.authorizations.length,0);
  assert.equal(result.stopReason,'end_turn');assert.match(result.assistantText,/3 条/);
 }
});

test('an ordinary stale tool name is corrected once to a current granted read without asking for permission',async t=>{
 const f=await fixture(t,{tools:['assets__inventory_stats','assets__inventory_query'],step:(body,n)=>{
  if(n===1)return turn([call('assets__old_inventory_read')]);
  if(n===2){assert.match(body.messages.at(-1).content,/合法名称/);assert.match(body.messages.find(message=>message.tool_call_id?.includes('old_inventory')).content,/host_tool_not_offered/);return turn([call('assets__inventory_query')]);}
  return text('根据 inventory_query 已读取项目资产，本次记录未提供归属确认。');
 }});
 const result=await f.prompt('读取当前项目资产并说明异常');
 assert.deepEqual(f.calls.map(item=>item.name),['assets__inventory_query']);assert.equal(f.requests.length,3);
 assert.equal(f.authorizations.length,0);assert.equal(result.stopReason,'end_turn');
 for(const request of f.requests)assertPaired(request.messages);
});

test('repeating an invalid selection gets at most one correction and does not fabricate a permission error',async t=>{
 const f=await fixture(t,{tools:['assets__inventory_query'],step:()=>turn([call('assets__old_inventory_read')])});
 const result=await f.prompt('读取当前项目资产并说明异常');
 assert.equal(f.requests.length,2);assert.equal(f.calls.length,0);assert.equal(f.authorizations.length,0);
 assert.match(result.assistantText,/没有出现在本轮可用列表/);assert.doesNotMatch(result.assistantText,/未获准|检查.{0,8}权限|没有授权/);
 assertPaired(f.runtime.sessions.get(f.sessionId).messages);
});

test('a wholly unknown tool with no Host offer is a capability gap and never prompts for authority',async t=>{
 const f=await fixture(t,{step:()=>turn([call('never_existed')])});
 const result=await f.prompt('帮我处理这个项目的问题');
 assert.equal(f.requests.length,1);assert.equal(f.calls.length,0);assert.equal(f.authorizations.length,0);
 assert.match(result.assistantText,/工具能力或选择缺口/);assert.doesNotMatch(result.assistantText,/未授权|没有授权|检查.{0,8}权限/);
 const messages=f.runtime.sessions.get(f.sessionId).messages;assertPaired(messages);
 assert.match(messages.find(message=>message.role==='tool').content,/host_tool_not_offered/);
});

test('a Host-declined operation and its provider alias never ask again while another granted read completes',async t=>{
 const denied='org.oviraptor.assets::inventory_reindex',mcpDenied=denied.replace('::','__'),read='org.oviraptor.assets__inventory_query';
 const f=await fixture(t,{tools:[read],step:(body,n)=>{
  assert(!offered(body).includes(providerToolName(mcpDenied)));assert(offered(body).includes(providerToolName(read)));
  assert(body.messages.some(message=>/本轮权限核对后仍不可用/.test(message.content??'')));
  assert(body.messages.every(message=>message._hostAuthorizationContext===undefined));
  if(n===1)return turn([call(providerToolName(mcpDenied))]);
  if(n===2)return turn([call(providerToolName(read))]);
  return text('根据 inventory_query 已读取项目记录；本轮未执行索引整理。');
 }});
 const result=await f.prompt('核对当前项目资产并执行索引整理',{authorizationContext:{unavailableTools:[denied],authorizationDeclined:true}});
 assert.deepEqual(f.calls.map(item=>item.name),[read]);assert.equal(f.authorizations.length,0);assert.equal(f.offers.length,0);
 assert.equal(result.stopReason,'end_turn');
 for(const request of f.requests)assertPaired(request.messages);
});

test('restricting one app never hides or denies another app with the same short tool name',async t=>{
 const other='org.example.other__inventory_query';
 const f=await fixture(t,{tools:[other],step:(body,n)=>{
  assert(offered(body).includes(providerToolName(other)));assert(!offered(body).includes(providerToolName('org.oviraptor.assets__inventory_query')));
  return n===1?turn([call(providerToolName(other))]):text('根据查询已读取另一应用获准的记录。');
 }});
 await f.prompt('读取当前项目的相关记录',{authorizationContext:{unavailableTools:['org.oviraptor.assets::inventory_query'],authorizationDeclined:true}});
 assert.deepEqual(f.calls.map(item=>item.name),[other]);assert.equal(f.authorizations.length,0);
});

test('a real same-turn App capability grant supersedes the earlier decline and continues without another prompt',async t=>{
 let granted=false;const steers=[];
 const f=await fixture(t,{tools:()=>['assets__discovery_cleanup_status',...(granted?['assets__discovery_cleanup_start']:[])],takeSteer:()=>steers.splice(0),step:(body,n)=>{
  if(n===1){assert(!offered(body).includes('assets__discovery_cleanup_start'));return turn([call('assets__discovery_cleanup_status')]);}
  if(n===2){assert(offered(body).includes('assets__discovery_cleanup_start'));assert(body.messages.some(message=>/最新 tools\/list 已实际提供/.test(message.content??'')));assert(!body.messages.some(message=>/本轮权限核对后仍不可用/.test(message.content??'')));return turn([call('assets__discovery_cleanup_start',{organization:'中国移动',autoExcludeUnrelated:true})]);}
  if(n===3)return turn([call('assets__discovery_cleanup_status')]);
  return text('根据新授权后的真实回执已启动原项目种子清洗，后台继续处理。');
 },execute:async params=>{
  if(!granted){granted=true;steers.push('我已在配置中心批准清洗权限，本轮请启动种子清洗。');return{structuredContent:{runId:'existing',phase:'paused'}};}
  return{structuredContent:{runId:'authorized-run',phase:'running'}};
 }});
 const result=await f.prompt('按中国移动清洗项目种子，明确无关的自动排除',{authorizationContext:{unavailableTools:['assets::discovery_cleanup_start'],authorizationDeclined:true}});
 assert.deepEqual(f.calls.map(item=>item.name),['assets__discovery_cleanup_status','assets__discovery_cleanup_start','assets__discovery_cleanup_status']);
 assert.equal(f.authorizations.length,0);assert.equal(result.stopReason,'end_turn');
 for(const request of f.requests)assertPaired(request.messages);
});

test('a steer claiming permission without a new Host capability cannot clear a declined operation',async t=>{
 const steers=[];
 const f=await fixture(t,{tools:['assets__discovery_cleanup_status'],takeSteer:()=>steers.splice(0),step:(body,n)=>{
  assert(!offered(body).includes('assets__discovery_cleanup_start'));
  return turn([call(n===1?'assets__discovery_cleanup_status':'assets__discovery_cleanup_start',n===1?{}:{organization:'中国移动',autoExcludeUnrelated:true})]);
 },execute:async()=>{steers.push('我已经授权了，现在直接启动种子清洗。');return{structuredContent:{runId:'existing',phase:'paused'}};}});
 const result=await f.prompt('按中国移动清洗项目种子，明确无关的自动排除',{authorizationContext:{unavailableTools:['assets::discovery_cleanup_start'],authorizationDeclined:true}});
 assert.deepEqual(f.calls.map(item=>item.name),['assets__discovery_cleanup_status']);assert.equal(f.authorizations.length,0);
 assert.match(result.assistantText,/未获准/);assert(!f.requests.some(body=>body.messages.some(message=>/最新 tools\/list 已实际提供/.test(message.content??''))));
 assertPaired(f.runtime.sessions.get(f.sessionId).messages);
});

test('a permanently advertised core workspace schema is not proof of new authorization',async t=>{
 const f=await fixture(t,{tools:['workspace_write','workspace_read'],step:(body,n)=>{
  assert(!offered(body).includes('workspace_write'));
  return n===1?turn([call('workspace_read',{path:'reports/input.md'})]):text('已读取原报告，本轮没有写入。');
 }});
 await f.prompt('检查现有报告并保存结果',{authorizationContext:{unavailableTools:['workspace_write'],authorizationDeclined:true}});
 assert.deepEqual(f.calls.map(item=>item.name),['workspace_read']);assert.equal(f.authorizations.length,0);
});

test('a declined cleanup control still reaches granted seed status and inventory reads through the dedicated workflow',async t=>{
 const f=await fixture(t,{tools:['assets__discovery_cleanup_status','assets__inventory_query'],step:(body,n)=>{
  assert(!offered(body).includes('assets__discovery_cleanup_start'));
  if(n===1){assert(offered(body).includes('assets__discovery_cleanup_status'));assert(offered(body).includes('assets__inventory_query'));return turn([call('assets__discovery_cleanup_status'),call('assets__inventory_query')]);}
  assert.equal(body.tools,undefined);return text('根据 discovery_cleanup_status 和 inventory_query 已读取现有状态；新增种子清洗本轮未获批准，未启动。');
 },execute:async params=>({structuredContent:params.name.endsWith('cleanup_status')?{runId:'existing',phase:'paused'}:{total:3}})});
 const result=await f.prompt('按中国移动清理当前项目全部种子，明确无关的自动排除',{authorizationContext:{unavailableTools:['assets::discovery_cleanup_start'],authorizationDeclined:true}});
 assert.deepEqual(f.calls.map(item=>item.name),['assets__discovery_cleanup_status','assets__inventory_query']);
 assert.equal(f.authorizations.length,0);assert.equal(result.stopReason,'end_turn');assert.match(result.assistantText,/未获批准，未启动/);
 for(const request of f.requests)assertPaired(request.messages);
});

test('a real newly missing scope is requested once and executes only after tools/list reflects its grant',async t=>{
 let granted=false;
 const f=await fixture(t,{tools:()=>['workspace_read',...(granted?['writer__save_report']:[])],step:(_body,n)=>
  n<=2?turn([call('writer__save_report',{path:'reports/result.md'})]):text('已根据保存回执保存报告。'),
 permissionOffer:()=>({projectId:3,identity:'runtime:1',offers:[{appId:'writer',permissions:[{permission:'report.write',tools:['save_report']}]}]}),
 authorize:()=>{granted=true;return true;}});
 const result=await f.prompt('请整理报告并保存');
 assert.equal(f.authorizations.length,1);assert.deepEqual(f.calls.map(item=>item.name),['writer__save_report']);
 assert.equal(result.stopReason,'end_turn');assert.equal(f.requests.length,3);
 for(const request of f.requests)assertPaired(request.messages);
});

test('a declined unqualified name is resolved by the exact Host offer without reopening consent',async t=>{
 const f=await fixture(t,{tools:['workspace_read'],step:(_body,n)=>{
  if(n===1)return turn([call('save_report')]);
  if(n===2)return turn([call('workspace_read',{path:'reports/input.md'})]);
  return text('已读取当前报告；本轮没有保存新报告。');
 },permissionOffer:()=>({projectId:3,identity:'runtime:1',offers:[{appId:'writer',permissions:[{permission:'report.write',tools:['save_report']}]}]})});
 await f.prompt('请检查报告并保存结果',{authorizationContext:{unavailableTools:['writer::save_report'],authorizationDeclined:true}});
 assert.equal(f.authorizations.length,0);
 assert.deepEqual(f.calls.map(item=>item.name),['workspace_read']);assert.equal(f.offers.length,1);
});

test('turn-local authorization notes disappear on the next request and fresh grants can execute in the same session',async t=>{
 let granted=false;
 const f=await fixture(t,{tools:()=>['workspace_read',...(granted?['writer__save_report']:[])],step:(body,n)=>{
  if(n===1){assert(body.messages.some(message=>/本轮权限核对后仍不可用/.test(message.content??'')));return text('本次继续保留当前报告。');}
  assert(!body.messages.some(message=>message.role==='user'&&/本轮权限核对后仍不可用|本轮不得再次询问|此选择纠正仅一次/.test(message.content??'')));
  assert(body.messages.every(message=>message._hostAuthorizationContext===undefined));
  return n===2?turn([call('writer__save_report')]):text('已根据真实回执保存报告。');
 }});
 await f.prompt('请保留当前报告',{authorizationContext:{unavailableTools:['writer::save_report'],authorizationDeclined:true}});
 granted=true;await f.prompt('现在保存报告');
 assert.deepEqual(f.calls.map(item=>item.name),['writer__save_report']);assert.equal(f.authorizations.length,0);
 for(const request of f.requests)assertPaired(request.messages);
});

test('a new user request can obtain a previously declined scope once without inheriting the old turn restriction',async t=>{
 let granted=false;
 const f=await fixture(t,{tools:()=>['workspace_read',...(granted?['writer__save_report']:[])],step:(body,n)=>{
  if(n===1)return text('本轮保留现有报告。');
  assert(!body.messages.some(message=>message.role==='user'&&/本轮权限核对后仍不可用|本轮不得再次询问/.test(message.content??'')));
  return n<=3?turn([call('writer__save_report')]):text('已依据新请求取得的权限保存报告。');
 },permissionOffer:()=>({projectId:3,identity:'runtime:1',offers:[{appId:'writer',permissions:[{permission:'report.write',tools:['save_report']}]}]}),authorize:()=>{granted=true;return true;}});
 await f.prompt('请保留现有报告',{authorizationContext:{unavailableTools:['writer::save_report'],authorizationDeclined:true}});
 const result=await f.prompt('请重新申请保存报告并保存');
 assert.equal(f.authorizations.length,1);assert.deepEqual(f.calls.map(item=>item.name),['writer__save_report']);assert.equal(result.stopReason,'end_turn');
 for(const request of f.requests)assertPaired(request.messages);
});

for(const reason of ['refusal','content_filter'])test(`upstream explicit ${reason} remains terminal without tool correction, authorization or replay`,async t=>{
 const f=await fixture(t,{tools:['workspace_read'],step:()=>({choices:[{delta:{...(reason==='refusal'?{refusal:'Provider policy stop'}:{content:'Provider policy stop'}),tool_calls:[{index:0,id:'not-executable',function:call('workspace_read')}]},finish_reason:reason}]})});
 const result=await f.prompt('读取当前项目报告');
 assert.equal(result.stopReason,'refusal');assert.equal(f.requests.length,1);assert.equal(f.calls.length,0);assert.equal(f.authorizations.length,0);
});

test('a cleanup control with an unknown receipt is never replayed through selection correction',async t=>{
 const f=await fixture(t,{tools:['assets__discovery_cleanup_start','assets__discovery_cleanup_status'],step:(_body,n)=>turn([call(n===2?'assets__discovery_cleanup_status':'assets__discovery_cleanup_start',n===2?{}:{organization:'中国移动',autoExcludeUnrelated:true})]),execute:async params=>{
  if(params.name.endsWith('_start'))throw Object.assign(Error('unknown'),{code:'host_discovery_task_outcome_unknown',details:{outcomeUnknown:true,operationId:'original-operation'}});
  return{structuredContent:{operationId:'original-operation',outcomeUnknown:true}};
 }});
 const result=await f.prompt('按中国移动清洗项目种子，明确无关的自动排除');
 assert.equal(f.calls.filter(item=>item.name.endsWith('_start')).length,1);assert.equal(f.calls.filter(item=>item.name.endsWith('_status')).length,1);
 assert.equal(f.authorizations.length,0);assert.match(result.assistantText,/执行结果未知/);assert.match(result.assistantText,/没有自动重发/);
 assertPaired(f.runtime.sessions.get(f.sessionId).messages);
});
