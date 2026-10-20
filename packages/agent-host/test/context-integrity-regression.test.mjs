import test from 'node:test';
import assert from 'node:assert/strict';
import { compressSessionMessages, SESSION_MESSAGE_LIMITS } from '../src/memory.mjs';
import { absorbToolResults, emptyAgentState, setGoal, verifyBeforeEnd } from '../src/agent-state.mjs';
import { isDataReadTool } from '../src/quality-prompt.mjs';
import { OpenAiCompatibleRuntime } from '../src/provider.mjs';

test('budget compression removes complete function exchanges and preserves the latest request', () => {
  for (const padding of [30_000,70_000,100_000]) {
    const messages=[{role:'system',content:'system rules'},{role:'user',content:'original request'},
      {role:'assistant',content:'x'.repeat(padding),tool_calls:[{id:'first',type:'function',function:{name:'inventory_query',arguments:'{}'}}]},
      {role:'tool',tool_call_id:'first',content:'{"total":3}'},{role:'user',content:'y'.repeat(45_000)},{role:'user',content:'latest request'}];
    const result=compressSessionMessages(messages);
    const calls=result.messages.flatMap(message=>message.tool_calls??[]).map(call=>call.id);
    const responses=result.messages.filter(message=>message.role==='tool').map(message=>message.tool_call_id);
    assert.deepEqual(calls,responses);
    assert.ok(result.messages.some(message=>message.content==='latest request'));
    assert.ok(Buffer.byteLength(JSON.stringify(result.messages))<=SESSION_MESSAGE_LIMITS.maxTotalBytes);
  }
});
test('failed or unrelated tool attempts and prior memory cannot verify current data', () => {
  let state=setGoal(emptyAgentState(),'当前有多少资产？');
  state=absorbToolResults(state,[{name:'inventory_query',ok:false,content:'denied'}]);
  assert.equal(state.confirmedFacts.length,0);
  state.confirmedFacts=['[确认·inventory_query] old snapshot'];
  for (const results of [[],[{name:'inventory_query',ok:false}],[{name:'workspace_write',ok:true}]]) {
    assert.equal(verifyBeforeEnd(state,{userText:'当前有多少资产？',assistantText:'根据工具，共有 30 个资产。',usedToolThisPrompt:true,toolNamesThisPrompt:['inventory_query'],toolResultsThisPrompt:results}).ok,false);
  }
  assert.equal(verifyBeforeEnd(state,{userText:'当前有多少资产？',toolResultsThisPrompt:[{name:'inventory_query',ok:true,content:'{"total":3}'}]}).ok,true);
});
test('native read tool names are recognized while mutation tools cannot satisfy read verification', () => {
  for (const name of ['saved_assets_stats','saved_assets_search','scan_list','scan_draft_read','scan_status']) assert.equal(isDataReadTool(`org.oviraptor.assets__${name}`),true);
  for (const name of ['inventory_import','draft_create','draft_cancel','scan_start','workspace_write']) assert.equal(isDataReadTool(name),false);
});
test('raw user provenance survives host wrappers without leaking internal fields into model requests', async () => {
  let request;
  const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'http://127.0.0.1:9/v1',model:'fixture',apiKey:'synthetic-key'},fetch:async(_url,init)=>{
    request=JSON.parse(init.body);
    return{ok:true,status:200,body:(async function*(){yield Buffer.from(`data: ${JSON.stringify({choices:[{delta:{content:'done'},finish_reason:'stop'}]})}\n\n`);})()};
  }});
  const {sessionId}=await runtime.newSession();
  await runtime.prompt({sessionId,prompt:'Host wrapper '.repeat(300)+'original requirement',userText:'original requirement'});
  assert.ok(request.messages.every(message=>!Object.hasOwn(message,'_hostUserText')));
  assert.equal(runtime.sessions.get(sessionId).messages.find(message=>message.role==='user')._hostUserText,'original requirement');
  await runtime.close();
});
