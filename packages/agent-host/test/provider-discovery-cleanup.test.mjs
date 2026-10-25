import assert from 'node:assert/strict';
import test from 'node:test';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
const names=['discovery_cleanup_start','discovery_cleanup_status','discovery_cleanup_pause','discovery_cleanup_resume','discovery_cleanup_restore','discovery_seed_list','discovery_assist_start','discovery_assist_status','certificate_task_start','inventory_import'];
const sse=entries=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:Array.isArray(entries)?{tool_calls:entries.map(([name,args],index)=>({index,id:'call-'+index+'-'+name,function:{name:'assets__'+name,arguments:JSON.stringify(args)}}))}:{content:entries},finish_reason:Array.isArray(entries)?'tool_calls':'stop'}]})}\n\n`;})()});
async function run(text,step,execute=async params=>({structuredContent:{runId:'saved-cleanup',phase:'running',counts:{excluded:31,conflict:2},assetsDeleted:0,hunterRequests:0}})){
 const requests=[],calls=[];
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture',apiKey:'synthetic-test-key-000'},fetch:async(_url,init)=>{const body=JSON.parse(init.body);requests.push(body);return step(body,requests.length);}});
 runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:names.map(name=>({name:'assets__'+name,inputSchema:{type:'object',properties:{runId:{type:'string'}}}}))};calls.push(params);return execute(params);},permissionOffer(){return{offers:[]};}});
 const{sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'chat'}]}]});
 const result=await runtime.prompt({sessionId,prompt:text,userText:text});await runtime.close();return{requests,calls,result};
}
test('explicit whole-project cleanup reaches one native start and one status without old discovery, paid queries or per-seed clicks',async()=>{
 const r=await run('按中国移动集团及其下属单位，联网分析当前项目全部种子，明确无关的自动排除，不确定的继续补证，真正冲突汇总给我。',(body,n)=>{
  const offered=body.tools?.map(t=>t.function.name);
  if(n===1){assert.deepEqual(offered,['assets__discovery_cleanup_start']);assert.equal(body.tool_choice,'required');assert.equal(body.parallel_tool_calls,false);return sse([['discovery_cleanup_start',{organization:'中国移动集团及其下属单位',autoExcludeUnrelated:true}],['discovery_cleanup_start',{organization:'中国移动集团及其下属单位',autoExcludeUnrelated:true}]]);}
  if(n===2){assert.deepEqual(offered,['assets__discovery_cleanup_status']);return sse([['discovery_cleanup_status',{runId:'guessed-other'}]]);}
  assert.equal(body.tools,undefined);return sse('后台清洗已启动，31条已可逆排除；补证继续运行，仅两组证据冲突需汇总。');
 });
 assert.deepEqual(r.calls.map(c=>c.name),['assets__discovery_cleanup_start','assets__discovery_cleanup_status']);assert.equal(r.calls[1].arguments.runId,'saved-cleanup');assert.equal(r.result.stopReason,'end_turn');
});
test('pause/resume/restore first read the real project job then bind that same run without accepting model-guessed identities',async()=>{
 for(const[action,text]of [['pause','暂停种子清洗'],['resume','继续种子清洗'],['restore','撤销本次种子清洗的排除批次']]){
  const r=await run(text,(body,n)=>{
   const offered=body.tools?.map(t=>t.function.name);
   if(n===1||n===3){assert.deepEqual(offered,['assets__discovery_cleanup_status']);return sse([['discovery_cleanup_status',{}]]);}
   if(n===2){assert.deepEqual(offered,['assets__discovery_cleanup_'+action]);assert.deepEqual(body.tools[0].function.parameters.properties.runId.enum,['saved-cleanup']);return sse([['discovery_cleanup_'+action,{runId:'forged',...(action==='restore'?{batchId:'batch-1'}:{})}]]);}
   assert.equal(body.tools,undefined);return sse('已根据真实回执核对当前任务。');
  });
  assert.deepEqual(r.calls.map(c=>c.name),['assets__discovery_cleanup_status','assets__discovery_cleanup_'+action,'assets__discovery_cleanup_status']);assert.equal(r.calls[1].arguments.runId,'saved-cleanup');
  if(action==='restore')assert.equal(r.calls[1].arguments.batchId,'batch-1');
 }
});
test('unknown start is not replayed or misreported as not started even if the model claims success or failure',async()=>{
 const r=await run('清理整个项目中国移动的种子，明确无关的自动排除',(body,n)=>n===1?sse([['discovery_cleanup_start',{organization:'中国移动',autoExcludeUnrelated:true}]]):n===2?sse([['discovery_cleanup_status',{}]]):sse('确定没有执行，你需要重新启动。'),async p=>{if(p.name.endsWith('_start'))throw Object.assign(Error('unknown'),{code:'host_discovery_task_outcome_unknown',details:{outcomeUnknown:true,operationId:'operation-original',phase:'unknown'}});return{structuredContent:{runId:'other-latest',phase:'running'}};});
 assert.equal(r.calls.filter(c=>c.name.endsWith('_start')).length,1);assert.equal(r.calls.filter(c=>c.name.endsWith('_status')).length,1);assert.match(r.result.assistantText,/执行结果未知/);assert.match(r.result.assistantText,/没有自动重发/);assert.doesNotMatch(r.result.assistantText,/确定没有执行/);
});
test('read-only cleanup status exposes no mutation and a categorical refusal gets a bounded correction to the actually available tool',async()=>{
 const r=await run('只读查看种子清洗进度，不要启动或排除',(body,n)=>{
  if(n===1){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_status']);return sse('我不能读取这些数据，因为没有工具。');}
  if(n===2){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_status']);assert.match(body.messages.at(-1).content,/已通过权限筛选/);return sse([['discovery_cleanup_status',{}]]);}
  assert.equal(body.tools,undefined);return sse('已根据 discovery_cleanup_status 读取任务进度。');
 });
 assert.equal(r.calls.length,1);assert.equal(r.calls[0].name,'assets__discovery_cleanup_status');assert.equal(r.result.stopReason,'end_turn');
});

test('an unknown control is queried by its original operation ID and reconciled only by the Host-bound owner witness',async()=>{
 const r=await run('清理项目种子，中国移动无关项自动排除',(body,n)=>{
  if(n===1)return sse([['discovery_cleanup_start',{organization:'中国移动',autoExcludeUnrelated:true}]]);
  if(n===2){assert.deepEqual(body.tools[0].function.parameters.properties.operationId.enum,['operation-original']);return sse([['discovery_cleanup_status',{runId:'unrelated-latest',operationId:'guessed'}]]);}
  assert.equal(body.tools,undefined);return sse('已按原操作回执核对，原清洗任务已持久保存并继续运行。');
 },async p=>{if(p.name.endsWith('_start'))throw Object.assign(Error('unknown'),{code:'host_discovery_task_outcome_unknown',details:{outcomeUnknown:true,operationId:'operation-original',phase:'unknown'}});assert.deepEqual(p.arguments,{operationId:'operation-original'});return{structuredContent:{operationId:'operation-original',operationFound:true,outcomeUnknown:false,receipt:{operationId:'operation-original',runId:'saved-cleanup'},reconciliation:{operationId:'operation-original',argumentDigest:'a'.repeat(64)}}};});
 assert.equal(r.calls.filter(c=>c.name.endsWith('_start')).length,1);assert.doesNotMatch(r.result.assistantText,/执行结果未知/);assert.match(r.result.assistantText,/持久保存/);
});

test('the real installed-App request with reversible bulk exclusion offers start and completes one control/status path',async()=>{
 const text='现在按“中国移动集团及其下属单位”对当前项目全量种子启动持久化自动清洗。我授权本轮联网查询公开 CT/证书、DNS/CNAME 和单位关系，使用当前模型；明确无关且通过独立证据交叉核验的自动可撤销批量排除，缺证的继续自动补证，真正冲突集中汇总给我。保留原始资产、种子、历史、配置和已确认探测范围，不调用 FOFA/Hunter，不进行范围外主动采集，不改 VPN。请直接使用 discovery_cleanup_start，启动后读取原任务状态并告诉我 runId、全量快照数量与实际进度，不要让我逐条点击，也不要把这个后台任务误称已全量完成。';
 const r=await run(text,(body,n)=>{
  if(n===1){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_start']);return sse([['discovery_cleanup_start',{organization:'中国移动集团及其下属单位',autoExcludeUnrelated:true}]]);}
  if(n===2){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_status']);return sse([['discovery_cleanup_status',{}]]);}
  assert.equal(body.tools,undefined);return sse('任务 saved-cleanup 已启动，后台仍在分析；没有删除原始数据，没有调用 Hunter。');
 });
 assert.deepEqual(r.calls.map(c=>c.name),['assets__discovery_cleanup_start','assets__discovery_cleanup_status']);assert.equal(r.result.stopReason,'end_turn');
});

test('cleanup count questions with negated controls never offer or execute a mutation, even when the model requests start',async()=>{
 for(const text of ['种子清洗还有多少，先不要暂停','种子清洗多少条了，不要撤销']){
  const r=await run(text,(body,n)=>{
   if(n<=2){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_status']);return n===1?sse([['discovery_cleanup_start',{organization:'中国移动',autoExcludeUnrelated:true}]]):sse([['discovery_cleanup_status',{}]]);}
   assert.equal(body.tools,undefined);return sse('已读取当前种子清洗数量，原任务保持现有状态。');
  });
  assert.deepEqual(r.calls.map(c=>c.name),['assets__discovery_cleanup_status'],text);assert.equal(r.result.stopReason,'end_turn');
 }
});

test('an object-first continuation binds the existing paused cleanup run and cannot start another snapshot',async()=>{
 const r=await run('已暂停的种子清洗继续执行',(body,n)=>{
  if(n===1||n===3){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_status']);return sse([['discovery_cleanup_status',{}]]);}
  if(n===2){assert.deepEqual(body.tools.map(t=>t.function.name),['assets__discovery_cleanup_resume']);assert.deepEqual(body.tools[0].function.parameters.properties.runId.enum,['saved-cleanup']);return sse([['discovery_cleanup_resume',{runId:'model-guessed-new-run'}]]);}
  assert.equal(body.tools,undefined);return sse('已继续原种子清洗任务，没有新建快照。');
 });
 assert.deepEqual(r.calls.map(c=>c.name),['assets__discovery_cleanup_status','assets__discovery_cleanup_resume','assets__discovery_cleanup_status']);assert.equal(r.calls[1].arguments.runId,'saved-cleanup');assert.equal(r.result.stopReason,'end_turn');
});
