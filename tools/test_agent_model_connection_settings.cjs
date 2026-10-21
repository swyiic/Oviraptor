const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const presets=loadPure('src/features/agentHost/providerPresets.ts');
const filename='src/features/agentHost/components/AgentRuntimeSettingsPanel.vue';
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'model-connections'});
async function mount(t){
  const mounted=[],calls=[];
  const settings={runtime:{id:'xai-api'},apiProvider:{configured:true,apiKeyConfigured:true,oauthConfigured:false,oauthSaved:true,provider:'openai-compatible',baseUrl:presets.DEEPSEEK_BASE_URL,model:'deepseek-chat'}};
  const client={getSettings:async()=>settings,setProvider:async input=>{calls.push(input);return settings;},selectOAuth:async()=>{calls.push({action:'select-saved-oauth'});return settings;}};
  const exports={};
  new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return{...vue,onMounted:fn=>mounted.push(fn),onUnmounted:()=>{}};
    if(name==='@lucide/vue')return{};
    if(name==='@tauri-apps/plugin-opener')return{openUrl:async()=>{throw Error('No external calls in test');}};
    if(name.endsWith('/i18n'))return{useI18n:()=>({tr:zh=>zh})};
    if(name==='../api')return{openAgentHostSettingsClient:async()=>({endpoint:{origin:'http://127.0.0.1:9'},client})};
    if(name==='../providerPresets')return presets;
    throw Error(`Unexpected model-settings import: ${name}`);
  },exports);
  const scope=vue.effectScope();const b=scope.run(()=>exports.default.setup({}, {expose(){},emit(){}}));
  t.after(()=>scope.stop());for(const callback of mounted)await callback();
  return{b,calls};
}
test('explicit custom model endpoint is preserved for both Grok and DeepSeek names',()=>{
  for(const model of ['grok-4.7','deepseek-chat'])assert.deepEqual(presets.resolveProviderForModel(model,'openai-compatible','https://fixture.invalid/v1'),{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model});
});
test('provider selection and saved OAuth have independent visible state',async t=>{
  const {b}=await mount(t);
  assert.equal(b.providerKindDraft.value,'deepseek');assert.equal(b.oauthLoggedIn.value,true);assert.match(b.oauthLabel.value,/保留/);
  b.apiKeyDraft.value='synthetic';b.providerKindDraft.value='custom';b.selectProvider();
  assert.equal(b.apiKeyDraft.value,'');assert.equal(b.baseUrlDraft.value,'');
});
test('saving custom connection sends endpoint and provider explicitly; saved OAuth activation sends no API key',async t=>{
  const {b,calls}=await mount(t);
  b.providerKindDraft.value='custom';b.baseUrlDraft.value='https://fixture.invalid/v1';b.modelDraft.value='grok-4.7';b.apiKeyDraft.value='synthetic-key';
  await b.saveProvider();assert.deepEqual(calls[0],{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'grok-4.7',apiKey:'synthetic-key'});
  await b.useSavedOAuth();assert.deepEqual(calls[1],{action:'select-saved-oauth'});
});
test('settings client selects saved OAuth with an explicit authenticated operation',async()=>{
  const calls=[];
  const {createHostSettingsClient}=loadPure('src/features/agentHost/settingsClient.ts');
  const client=createHostSettingsClient({origin:'http://127.0.0.1:9',session:'synthetic-session',fetchImpl:async(url,init)=>{
    calls.push({url,init});return new Response(JSON.stringify({runtime:{id:'none'},apiProvider:{authentication:'oauth',configured:true}}));
  }});
  await client.selectOAuth();
  assert.equal(calls.length,1);assert.equal(calls[0].url,'http://127.0.0.1:9/api/settings/provider/oauth/select');
  assert.equal(calls[0].init.method,'POST');assert.equal(calls[0].init.body,'{}');assert.equal(calls[0].init.headers['X-Oviraptor-Session'],'synthetic-session');
});
