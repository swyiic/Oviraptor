import test from 'node:test';
import assert from 'node:assert/strict';
import { isProjectReadQuestion, isInventoryCountQuestion } from '../src/quality-prompt.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';

const question = '请重新调用工具读取当前项目的资产总数和已保存的证书 SHA-256 去重数量，仅用两行回答。';
test('existing saved certificate counts are reads; writes and filtered counts keep their workflows', () => {
  for (const text of [question, '已保存的证书指纹有多少？', '当前项目已采集的资产总数是多少']) {
    assert.equal(isProjectReadQuestion(text), true, text);
    assert.equal(isInventoryCountQuestion(text), true, text);
  }
  for (const text of ['读取当前项目资产数量并保存到文件', '开始执行当前项目证书补采', '删除已保存的证书', '修改当前项目范围']) {
    assert.equal(isProjectReadQuestion(text), false, text);
    assert.equal(isInventoryCountQuestion(text), false, text);
  }
  for (const text of ['当前项目 example.com 下有多少资产', '当前项目按状态分组的资产数量', '为什么当前项目只有一个证书指纹',
    '当前项目状态码 200 的资产有多少', '当前项目端口 443 的资产数量', '当前项目 FOFA 来源的资产数量']) {
    assert.equal(isInventoryCountQuestion(text), false, text);
  }
});

const sse = delta => (async function* () {
  yield Buffer.from(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: delta.tool_calls ? 'tool_calls' : 'stop' }] })}\n\n`);
})();

test('the production question reads once and presents results without task DSL or repeated verification', async () => {
  const requests = [], executions = [];
  const runtime = new OpenAiCompatibleRuntime({
    config: { provider: 'openai-compatible', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-v4-flash', apiKey: 'synthetic-test-key-000' },
    fetch: async (_url, init) => {
      const body = JSON.parse(init.body); requests.push(body);
      if (requests.length === 1) return { ok: true, body: sse({ tool_calls: [{ index: 0, id: 'stats', function: { name: 'assets__inventory_stats', arguments: '{}' } }] }) };
      return { ok: true, body: sse({ content: '根据 inventory_stats，资产 30227 条，证书指纹 1 条。' }) };
    },
  });
  runtime.bindHost({ async mcp(_token, _conversation, method, params) {
    if (method === 'tools/list') return { tools: [
      { name: 'assets__inventory_stats', inputSchema: { type: 'object' } },
      { name: 'assets__project_overview', inputSchema: { type: 'object' } },
      { name: 'host_task_update', inputSchema: { type: 'object' } },
      { name: 'host_task_verify', inputSchema: { type: 'object' } },
    ] };
    executions.push(params.name);
    return { structuredContent: { total: 30227, discoverySeeds: { certificateFingerprints: 1 } } };
  } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [
    { name: 'OVIRAPTOR_MCP_TOKEN', value: 'test' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'test' },
  ] }] });
  const result = await runtime.prompt({ sessionId, prompt: question, userText: question });
  assert.equal(result.stopReason, 'end_turn');
  assert.deepEqual(executions, ['assets__inventory_stats']);
  assert.equal(requests.length, 2);
  assert.deepEqual(requests[0].tools.map(tool => tool.function.name), ['assets__inventory_stats']);
  assert.equal(requests[1].tools, undefined);
  assert.ok(requests[1].messages.some(message => message.role === 'tool' && message.content.includes('30227')));
});
test('a local script request mentioning prohibited scanning keeps file, execution and verification tools throughout',async()=>{
 const {scanningHandoffIntent,isReadOnlyDataRequest}=await import('../src/quality-prompt.mjs');
 const text='读取当前项目已有资产统计，不发起 FOFA/Hunter/CT/DNS/TLS/扫描。使用 workspace_write 创建 check.mjs，用 workspace_run 运行脚本，失败后修正、复测并保存报告。';
 assert.equal(scanningHandoffIntent(text),null);assert.equal(isReadOnlyDataRequest(text),false);
 for(const input of ['读取当前项目资产并创建扫描草稿，不启动','把当前项目资产交给扫描，先做预检'])assert.ok(scanningHandoffIntent(input),input);
 const requests=[],calls=[],names=['assets__inventory_query','scanning__scan_from_assets','workspace_write','workspace_run','host_task_read','host_task_verify','host_task_criteria','host_task_update'];
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://example.invalid/v1',model:'fixture',apiKey:'synthetic-test-key-000'},fetch:async(_url,init)=>{const body=JSON.parse(init.body);requests.push(body);const next=['assets__inventory_query','workspace_write','workspace_run'][requests.length-1];return{ok:true,body:sse(next?{tool_calls:[{index:0,id:'call'+requests.length,function:{name:next,arguments:'{}'}}]}:{content:'已读取工具回执并完成本地脚本复测。'})};}});
 runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:names.map(name=>({name,description:'test',inputSchema:{type:'object',properties:{}}}))};calls.push(params.name);return{content:[{type:'text',text:JSON.stringify(params.name==='workspace_run'?{success:true,exitCode:0,stdout:'PASS'}:{total:12,path:'check.mjs'})}]};}});
 const {sessionId}=await runtime.newSession({cwd:'/tmp',mcpServers:[{name:'host',env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'test'},{name:'OVIRAPTOR_CONVERSATION',value:'conversation'}]}]});await runtime.prompt({sessionId,prompt:text,turnId:'run',userText:text});
 assert.deepEqual(calls,['assets__inventory_query','workspace_write','workspace_run']);for(const request of requests){const tools=request.tools.map(t=>t.function.name);assert.ok(tools.includes('workspace_run'));assert.ok(tools.includes('host_task_verify'));}
});
test('quality mode can finish a read-write-repair-verification sequence beyond twelve turns while explicit request caps remain hard',async()=>{
 for(const override of [undefined,2]){let requests=0;const runtime=new OpenAiCompatibleRuntime({...(override?{maxSteps:override}:{}),config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic-test-key-000'},fetch:async()=>{requests++;return{ok:true,body:sse(requests<=12?{tool_calls:[{index:0,id:'operation-'+requests,function:{name:'workspace_run',arguments:JSON.stringify({path:'check.mjs',runtime:'node',args:[String(requests)]})}}]}:{content:'本轮脚本及有限验收已完成。'})};}});runtime.bindHost({async mcp(_t,_c,method){return method==='tools/list'?{tools:[{name:'workspace_run',inputSchema:{type:'object'}}]}:{content:[{type:'text',text:'{"success":true,"exitCode":0}'}]};}});const{sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'chat'}]}]});const result=await runtime.prompt({sessionId,prompt:'执行本地脚本，按结果修复并完成有限验收。',userText:'执行本地脚本，按结果修复并完成有限验收。'});assert.equal(requests,override??13);assert.equal(result.stopReason,override?'max_turn_requests':'end_turn');}
});

test('seed totals plus organization classification never collapse into the simple inventory-count shortcut', () => {
  for (const text of [
    '你读取我当前项目空间的项目种子文件，看看总数量多少，然后明确是移动的项目又是多少',
    '当前项目资产总数多少，明确属于中国移动的多少？',
    '读取整个项目种子，分析哪些属于中国移动，不要修改数据。',
  ]) {
    assert.equal(isProjectReadQuestion(text), true, text);
    assert.equal(isInventoryCountQuestion(text), false, text);
  }
});

test('real combined seed question retains seed reads after inventory statistics and never exposes mutations', async () => {
  const text='你读取我当前项目空间的项目种子文件，看看总数量多少，然后明确是移动的项目又是多少';
  const requests=[],calls=[];
  const names=['inventory_stats','inventory_query','discovery_seed_list','discovery_seed_detail','discovery_assist_status','discovery_assist_start','inventory_import'];
  const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture',apiKey:'synthetic-test-key-000'},fetch:async(_url,init)=>{
    const body=JSON.parse(init.body);requests.push(body);
    const tool=['inventory_stats','discovery_seed_list','discovery_seed_detail'][requests.length-1];
    for(const hidden of ['discovery_assist_start','inventory_import'])assert(!body.tools.some(t=>t.function.name.endsWith('__'+hidden)));
    assert(body.tools.some(t=>t.function.name==='assets__discovery_seed_list'));
    return{ok:true,body:sse(tool?{tool_calls:[{index:0,id:'seed-'+requests.length,function:{name:'assets__'+tool,arguments:JSON.stringify(tool==='discovery_seed_detail'?{seedId:17}:{})}}]}:{content:'根据种子台账与对应证据，未核验的项目不能列为已确认归属。'})};
  }});
  runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:names.map(name=>({name:'assets__'+name,inputSchema:{type:'object'}}))};calls.push(params);return{structuredContent:params.name.endsWith('inventory_stats')?{total:30227,discoverySeeds:{total:29977}}:params.name.endsWith('discovery_seed_list')?{total:1,items:[{id:17,value:'example.test',trust:'needs_evidence'}],page:1,pageSize:50}:{seed:{id:17,value:'example.test'},evidence:[],observations:[]}};}});
  const{sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'chat'}]}]});
  const result=await runtime.prompt({sessionId,prompt:text,userText:text});await runtime.close();
  assert.equal(result.stopReason,'end_turn');assert.deepEqual(calls.map(c=>c.name),['assets__inventory_stats','assets__discovery_seed_list','assets__discovery_seed_detail']);
  assert.equal(requests.length,4);
});

test('full seed ledger pagination can pass eight reads while still respecting model-request limits', async () => {
  const text='只读逐页读取当前项目全部种子清单及总数，不修改任何数据。';
  const calls=[];let step=0;
  const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture',apiKey:'synthetic-test-key-000'},fetch:async(_url,init)=>{
    const body=JSON.parse(init.body);step++;
    assert(body.tools.some(t=>t.function.name==='assets__discovery_seed_list'),'paginated seed tool remains offered after page eight');
    assert(!body.tools.some(t=>t.function.name==='assets__discovery_assist_start'));
    return{ok:true,body:sse(step<=10?{tool_calls:[{index:0,id:'page-'+step,function:{name:'assets__discovery_seed_list',arguments:JSON.stringify({page:step,pageSize:50})}}]}:{content:'已根据 discovery_seed_list 读取 10 页全部记录。'})};
  }});
  runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:['discovery_seed_list','discovery_assist_status','discovery_assist_start'].map(name=>({name:'assets__'+name,inputSchema:{type:'object'}}))};calls.push(params.arguments.page);return{structuredContent:{page:params.arguments.page,total:500,items:[{id:params.arguments.page}],hasMore:params.arguments.page<10}};}});
  const{sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'chat'}]}]});const result=await runtime.prompt({sessionId,prompt:text,userText:text});await runtime.close();
  assert.equal(result.stopReason,'end_turn');assert.deepEqual(calls,[1,2,3,4,5,6,7,8,9,10]);
});
