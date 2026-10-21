const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const presets=loadPure('src/features/agentHost/providerPresets.ts');
const filename='src/features/agentHost/components/AgentRuntimeSettingsPanel.vue';
const source=fs.readFileSync(filename,'utf8');
const {descriptor}=parse(source,{filename});
const script=compileScript(descriptor,{id:'host-network-settings'});
const network=()=>({valid:true,mode:'system',noProxy:'',manualProxyConfigured:true,manualProxyEndpoint:'http://proxy.test:7890',manualProxyCredentialsConfigured:true,routes:{oauth:{mode:'system',target:'https://auth.x.ai',source:'inherited-environment',reason:'environment-proxy',environmentVariable:'HTTPS_PROXY',proxyEndpoint:'http://proxy.test:7890'}}});

async function mount(t){
  const mounted=[],calls=[];
  let settings={runtime:{id:'none'},apiProvider:{configured:false,apiKeyConfigured:false},network:network()};
  const client={getSettings:async()=>settings,setNetwork:async input=>{calls.push(input);settings={...settings,network:{...network(),mode:input.mode,noProxy:input.noProxy||'',manualProxyConfigured:!input.clearProxy}};return settings;}};
  const exports={};new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return{...vue,onMounted:fn=>mounted.push(fn),onUnmounted:()=>{}};
    if(name==='@lucide/vue')return{};
    if(name==='@tauri-apps/plugin-opener')return{openUrl:async()=>{throw Error('No external calls in test');}};
    if(name.endsWith('/i18n'))return{useI18n:()=>({tr:zh=>zh})};
    if(name==='../api')return{openAgentHostSettingsClient:async()=>({endpoint:{origin:'http://127.0.0.1:9'},client})};
    if(name==='../providerPresets')return presets;
    throw Error(`Unexpected network-settings import: ${name}`);
  },exports);
  const scope=vue.effectScope(),b=scope.run(()=>exports.default.setup({}, {expose(){},emit(){}}));t.after(()=>scope.stop());
  for(const callback of mounted)await callback();for(let i=0;i<4;i++)await Promise.resolve();
  return{b,calls};
}

test('network UI saves its independent policy and clears the proxy credential draft after save',async t=>{
  const {b,calls}=await mount(t);
  assert.equal(b.networkProxyDraft.value,'','saved endpoint is never copied into a credential input');
  b.networkMode.value='manual';b.networkProxyDraft.value='http://synthetic-user:synthetic-password@proxy.test:7890';b.networkNoProxy.value='.internal.test';
  await b.saveNetwork();
  assert.deepEqual(calls,[{mode:'manual',proxyUrl:'http://synthetic-user:synthetic-password@proxy.test:7890',noProxy:'.internal.test'}]);
  assert.equal(b.networkProxyDraft.value,'');assert.equal(b.networkMode.value,'manual');assert.equal(b.networkNoProxy.value,'.internal.test');
  assert(!JSON.stringify(b.settings.value).includes('synthetic-password'));
  await b.saveNetwork(true);assert.deepEqual(calls[1],{mode:'system',noProxy:'.internal.test',clearProxy:true});
});

test('unrelated settings refresh preserves an unsaved network draft and inherited routing is labelled honestly',async t=>{
  const {b}=await mount(t);b.networkMode.value='direct';b.networkNoProxy.value='draft.internal';
  b.applySettings({runtime:{id:'none'},network:network()});
  assert.equal(b.networkMode.value,'direct');assert.equal(b.networkNoProxy.value,'draft.internal');
  assert.match(b.networkRouteLabel(network().routes.oauth),/继承环境代理.*HTTPS_PROXY/);
  assert.match(source,/桌面启动器可能注入系统代理，当前不能区分其原始来源/);
  assert.match(source,/data-testid="agent-network-last-request"/);
});

test('both settings and chat clients use the authenticated network API',async()=>{
  const {createHostSettingsClient}=loadPure('src/features/agentHost/settingsClient.ts');
  const {createAgentHostChatClient}=loadPure('src/features/agentHost/chatClient.ts');
  const calls=[];const settings={runtime:{id:'none'},network:network()};
  const fetchImpl=async(url,init={})=>{calls.push({url,init});return{ok:true,status:200,text:async()=>JSON.stringify(init.method==='POST'?settings:network()),json:async()=>init.method==='POST'?settings:network()};};
  const endpoint={origin:'http://127.0.0.1:9',session:'synthetic-session'};
  const settingsClient=createHostSettingsClient({...endpoint,fetchImpl});await settingsClient.getNetwork();await settingsClient.setNetwork({mode:'direct'});
  const chat=createAgentHostChatClient(endpoint,fetchImpl);await chat.network();await chat.setNetwork({mode:'system'});
  assert.equal(calls.length,4);for(const call of calls){assert.equal(new URL(call.url).pathname,'/api/settings/network');assert.equal(call.init.headers['X-Oviraptor-Session'],'synthetic-session');}
  assert.deepEqual(JSON.parse(calls[1].init.body),{mode:'direct'});assert.deepEqual(JSON.parse(calls[3].init.body),{mode:'system'});
});
