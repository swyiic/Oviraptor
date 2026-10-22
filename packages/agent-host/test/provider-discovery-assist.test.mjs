import assert from 'node:assert/strict';
import test from 'node:test';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { discoveryAssistIntent } from '../src/quality-prompt.mjs';

const names = ['discovery_assist_start','discovery_assist_status','discovery_assist_pause','discovery_review_read','discovery_scope_propose'];
function response(requestNumber, entries, text = '后台任务状态已核对，新增范围请在面板一次确认。') {
  return { ok: true, status: 200, body: (async function* () {
    yield `data: ${JSON.stringify({ choices: [{ delta: entries ? { tool_calls: entries.map(([name,args],index) => ({ index, id: `${requestNumber}-${index}`, function: { name: 'assets__' + name, arguments: JSON.stringify(args) } })) } : { content: text }, finish_reason: entries ? 'tool_calls' : 'stop' }] })}\n\n`;
  })() };
}
async function run(text, fetchStep, execute = async () => ({ structuredContent: { runId: 'saved-review', phase: 'analyzing', review: { run: { requests: 1 } } } })) {
  const requests = [], calls = [];
  const runtime = new OpenAiCompatibleRuntime({ config: { provider: 'openai-compatible', baseUrl: 'https://fixture.invalid/v1', model: 'fixture-model', apiKey: 'synthetic' }, fetch: async (_url, input) => {
    const body = JSON.parse(input.body); requests.push(body); return fetchStep(body, requests.length);
  } });
  runtime.bindHost({ async mcp(_token, _conversation, method, params) {
    if (method === 'tools/list') return { tools: [...names.map(name => ({ name: 'assets__' + name, inputSchema: { type: 'object', properties: {} } })), { name: 'assets__certificate_task_start' }, { name: 'assets__discovery_enrichment_start' }, { name: 'hunter' }, { name: 'host_task_update' }] };
    calls.push(params); return execute(params);
  } });
  const { sessionId } = await runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'scope' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'conversation' }] }] });
  const result = await runtime.prompt({ sessionId, prompt: text, userText: text });
  await runtime.close(); return { result, requests, calls };
}
test('whole-project AI start performs one owner control and one status, suppresses duplicate controls and exposes no paid or scope-confirm tool', async () => {
  const text = '开始 AI 发现，按中国移动集团从整个项目证书提炼种子';
  const result = await run(text, (body, n) => {
    assert.equal(body.parallel_tool_calls, n < 3 ? false : undefined);
    if (n === 1) {
      assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_start']);
      return response(n, [['discovery_assist_start',{ organization: '中国移动集团', includeIsolated: false }],['discovery_assist_start',{ organization: '中国移动集团', includeIsolated: false }]]);
    }
    if (n === 2) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_status']); return response(n, [['discovery_assist_status',{ projectId: 999, taskId: 'guessed' }]]); }
    assert.equal(body.tools, undefined); return response(n, null);
  });
  assert.equal(result.result.stopReason, 'end_turn'); assert.equal(result.requests.length, 3);
  assert.deepEqual(result.calls.map(c => c.name), ['assets__discovery_assist_start','assets__discovery_assist_status']);
  assert.deepEqual(result.calls[0].arguments, { organization: '中国移动集团', includeIsolated: false });
  assert.deepEqual(result.calls[1].arguments, {});
});
test('read-only AI progress requests cannot expose start or pause even when the old task goal mentioned collection', async () => {
  const text = '只读查看 AI 发现进度，不要启动或暂停任务';
  assert.equal(discoveryAssistIntent(text).action, 'status');
  const result = await run(text, (body, n) => {
    if (n === 1) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_status']); return response(n, [['discovery_assist_status',{}]]); }
    assert.equal(body.tools, undefined); return response(n, null, '已读取当前 AI 发现状态。');
  });
  assert.equal(result.requests.length, 2); assert.deepEqual(result.calls.map(c => c.name), ['assets__discovery_assist_status']);
});
test('an AI discovery control failure is checked once and never automatically replayed', async () => {
  const result = await run('暂停全项目 AI 发现', (body, n) => {
    if (n === 1) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_pause']); return response(n, [['discovery_assist_pause',{ projectId: 999 }]]); }
    if (n === 2) { assert.deepEqual(body.tools.map(t => t.function.name), ['assets__discovery_assist_status']); return response(n, [['discovery_assist_status',{}]]); }
    assert.equal(body.tools, undefined); return response(n, null, '暂停请求响应未知，已核对后台状态，不重发。');
  }, async params => {
    if (params.name.endsWith('_pause')) throw Object.assign(Error('unknown'), { code: 'host_discovery_task_outcome_unknown', details: { outcomeUnknown: true, operationId: 'original' } });
    return { structuredContent: { phase: 'pausing', runId: 'saved-review' } };
  });
  assert.equal(result.requests.length, 3); assert.equal(result.calls.filter(c => c.name.endsWith('_pause')).length, 1);
  assert.deepEqual(result.calls[0].arguments, {});
});

test('one session can move from old read-only work to explicit AI start and pause; a mention never promotes to control', async () => {
  const calls = [], requests = []; let turn = 0, step = 0;
  const toolTurn = (name, args = {}) => ({ ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:{tool_calls:[{index:0,id:`${turn}-${step}`,function:{name,arguments:JSON.stringify(args)}}]},finish_reason:'tool_calls'}]})}\n\n`;})() });
  const runtime = new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic'},fetch:async (_url,input)=>{
    const body=JSON.parse(input.body);requests.push(body);step++;
    const offered=(body.tools??[]).map(t=>t.function.name);
    const short=name=>String(name).split('__').at(-1);
    if(turn===0&&step===1){assert.deepEqual(offered.map(short),['inventory_stats']);return toolTurn(offered[0]);}
    if(turn===1&&step===1){assert.deepEqual(offered.map(short),['discovery_assist_status']);return toolTurn(offered[0]);}
    if(turn===2){
      if(step===1){assert.deepEqual(offered.map(short),['discovery_assist_start']);assert.equal(body.tool_choice,'required');return toolTurn('host_task_read');}
      if(step===2){assert.deepEqual(offered.map(short),['discovery_assist_start']);assert.match(body.messages.at(-1).content,/合法名称/);return toolTurn(offered[0],{organization:'中国移动集团及其下属单位',includeIsolated:true});}
      if(step===3){assert.deepEqual(offered.map(short),['discovery_assist_status']);return toolTurn(offered[0]);}
    }
    if(turn===3){
      assert.deepEqual(offered.map(short),['discovery_assist_status']);
      return toolTurn('discovery_assist_start',{organization:'未经本轮要求启动'});
    }
    if(turn===4&&step<=2){assert.deepEqual(offered.map(short),[step===1?'discovery_assist_pause':'discovery_assist_status']);return toolTurn(offered[0]);}
    assert.deepEqual(offered,[]);return response(step,null,'已依据本轮工具回执回答。');
  }});
  runtime.bindHost({async mcp(_token,_conversation,method,params){
    if(method==='tools/list')return{tools:['inventory_stats',...names].map(name=>({name:'org.oviraptor.assets__'+name,inputSchema:{type:'object',properties:{}}}))};
    calls.push(params);return{structuredContent:{total:3,runId:'saved-review',phase:'analyzing'},content:[{type:'text',text:'{"total":3,"runId":"saved-review"}'}]};
  },permissionOffer(){return{offers:[]};}});
  const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'conversation'}]}]});
  const texts=['当前项目有多少资产？','只读查看 AI 发现进度，不要启动或暂停任务','开始 AI 发现资产。目标单位中国移动集团及其下属单位，includeIsolated=true；不确认新增范围、不调用 Hunter。','AI 发现有哪些线索？','暂停全项目 AI 发现'];
  for(turn=0;turn<texts.length;turn++){step=0;await runtime.prompt({sessionId,prompt:texts[turn],userText:texts[turn],taskContext:{goal:texts[0]}});}
  await runtime.close();
  assert.deepEqual(calls.map(c=>c.name.split('__').at(-1)),['inventory_stats','discovery_assist_status','discovery_assist_start','discovery_assist_status','discovery_assist_pause','discovery_assist_status']);
  assert.deepEqual(calls.find(c=>c.name.endsWith('_start')).arguments,{organization:'中国移动集团及其下属单位',includeIsolated:true});
  assert.equal(discoveryAssistIntent(texts[3]).action,'status');
  assert.equal(discoveryAssistIntent('AI 发现启动了吗？').action,'status');
  assert.equal(discoveryAssistIntent('不要停止 AI 发现').action,'status');
  assert(requests.length<15,'tool selection recovery is bounded and never replays a completed control');
});
