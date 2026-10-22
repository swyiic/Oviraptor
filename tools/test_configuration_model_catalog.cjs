const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const sections=loadPure('src/features/configuration/configProfileSections.ts');
const defaults=loadPure('src/features/configuration/configProfileDefaults.ts');
const stateHelpers=loadPure('src/features/configuration/categoryState.ts');
const modelHelpers=loadPure('src/features/configuration/modelConnections.ts');
global.crypto=require('node:crypto').webcrypto;
const clone=value=>JSON.parse(JSON.stringify(value));
const filename='src/features/configuration/sections/ConfigModelsSection.vue';
const source=fs.readFileSync(filename,'utf8');
const {descriptor}=parse(source,{filename});
const script=compileScript(descriptor,{id:'model-catalog'});
function fixture(t){
  const stored=new Map();global.localStorage={getItem:key=>stored.get(key)??null,setItem:(key,value)=>stored.set(key,value),removeItem:key=>stored.delete(key)};
  const calls=[],events=[],mounted=[],unmounted=[];
  const ownerVersions=new Map(),names=new Map();let counter=0,selection=null;
  let host={runtime:{id:'none'},answerMode:'quality',apiProvider:{configured:false,apiKeyConfigured:false,oauthSaved:true,oauthConfigured:false},network:{mode:'system',noProxy:''}};
  let providerFailures=0,runtimeFailures=0,saveFailure=null;
  const builtin=sections.pickSectionSettings(defaults.CONFIG_PROFILE_SETTING_DEFAULTS,'models');
  function currentView(reference){
    const target=reference??selection?.reference??null;
    const settings=target?ownerVersions.get(target.policyId).get(target.revision):builtin;
    return {section:'models',source:target?'owner':'builtin',legacyProfileId:null,reference:target,settings:clone(settings),builtinSettings:clone(builtin),defaultSelection:clone(selection),legacyAvailable:false,entries:[...ownerVersions].map(([id,versions])=>{const revision=Math.max(...versions.keys());return {name:names.get(id),reference:{policyId:id,revision,kind:'models'},versions:[...versions.keys()].sort((a,b)=>b-a).map(revision=>({policyId:id,revision,kind:'models'}))};})};
  }
  function receipt(input,action,reference){return {receipt:{schemaVersion:1,requestId:input.requestId,section:'models',action,reference,defaultSelection:clone(selection)},currentDefault:clone(selection),currentState:'unchanged'};}
  const api={
    getCategory:async(section,reference)=>{calls.push(['get',section,clone(reference??null)]);assert.equal(section,'models');return clone(currentView(reference));},
    saveCategory:async input=>{
      calls.push(['save',clone(input)]);if(saveFailure)throw saveFailure;
      const id=input.mode==='revise'?input.expectedReference.policyId:`owner-${++counter}`;
      const revision=input.mode==='revise'?input.expectedReference.revision+1:1;
      if(!ownerVersions.has(id)){ownerVersions.set(id,new Map());names.set(id,input.name);}
      ownerVersions.get(id).set(revision,clone(input.settings));
      const reference={policyId:id,revision,kind:'models'};
      if(input.makeDefault)selection={revision:(selection?.revision??0)+1,reference};
      return receipt(input,input.mode,reference);
    },
    setCategoryDefault:async input=>{calls.push(['default',clone(input)]);selection={revision:(selection?.revision??0)+1,reference:clone(input.reference)};return receipt(input,'select',input.reference);},
    getCategoryReceipt:async()=>null,
    testModelProfile:async input=>{calls.push(['test',clone(input)]);return {ok:true,message:'fixture-connected'};},
    saveProfile:()=>{throw Error('Whole-profile writer forbidden');},saveProfileSection:()=>{throw Error('Profile section writer forbidden');},
  };
  const client={
    getSettings:async()=>clone(host),
    setProvider:async input=>{calls.push(['host-provider',clone(input)]);if(providerFailures-->0)throw Error('fixture-host-unavailable');host.apiProvider={...host.apiProvider,configured:true,provider:input.provider,model:input.model,baseUrl:input.baseUrl,authType:input.authType,apiKeyConfigured:input.authType==='api-key',oauthConfigured:false,connectionRef:clone(input.connectionRef)};return clone(host);},
    setRuntime:async runtime=>{calls.push(['runtime',runtime]);if(runtimeFailures-->0)throw Error('fixture-runtime-unavailable');host.runtime.id=runtime;return clone(host);},
    selectOAuth:async()=>{calls.push(['oauth-select']);host.apiProvider={...host.apiProvider,configured:true,oauthConfigured:true,apiKeyConfigured:false,connectionRef:null};return clone(host);},
    startOAuth:async()=>({status:'pending',userCode:'fixture-code',verificationUri:'https://example.invalid/login'}),
    pollOAuth:async()=>({status:'authorized'}),logoutOAuth:async()=>clone(host),
    setNetwork:async()=>clone(host),setAnswerMode:async()=>clone(host),
  };
  const exports={};
  new Function('require','exports',transpile(script.content))(name=>{
    if(name.endsWith('/shared/directives/refreshFeedback'))return loadPure('src/shared/directives/refreshFeedback.ts');
    if(name==='vue')return {...vue,onMounted:fn=>mounted.push(fn),onUnmounted:fn=>unmounted.push(fn)};
    if(name==='@lucide/vue')return {};
    if(name==='@tauri-apps/plugin-opener')return {openUrl:async()=>{}};
    if(name.endsWith('/agentHost/api'))return {openAgentHostSettingsClient:async()=>({client,endpoint:{}})};
    if(name.endsWith('/i18n'))return {useI18n:()=>({tr:(_zh,en)=>en})};
    if(name==='../api')return {configurationApi:api};
    if(name==='../configProfileDefaults')return defaults;
    if(name==='../configProfileSections')return sections;
    if(name==='../categoryState')return stateHelpers;
    if(name==='../modelConnections')return modelHelpers;
    throw Error(`Unexpected import ${name}`);
  },exports);
  const scope=vue.effectScope();
  const state=scope.run(()=>exports.default.setup({profiles:[]},{expose(){},emit:(...args)=>events.push(args)}));
  t.after(()=>{unmounted.forEach(fn=>fn());scope.stop();});
  return {state,calls,events,stored,api,client,load:()=>state.loadDirectory(),unmount:()=>unmounted.forEach(fn=>fn()),failHost:count=>providerFailures=count,failRuntime:count=>runtimeFailures=count,failSave:value=>saveFailure=value,setHost:value=>host=value,getHost:()=>host};
}
function configure(state,local=false){Object.assign(state.form,{name:'One connection',llm:'fixture-model',apiBase:'http://127.0.0.1:9/v1',apiKey:local?'':'fixture-api-key',localApiKey:'',deployment:local?'local':'cloud'});}
test('one editor writes one Rust connection before binding that same reference to chat and scan',async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;h.state.useScan.value=true;
  await h.state.testConnection();await h.state.saveConnection();
  assert.equal(h.state.error.value,'');
  const write=h.calls.find(call=>call[0]==='save')[1],host=h.calls.find(call=>call[0]==='host-provider')[1];
  assert.equal(h.calls.filter(call=>call[0]==='save').length,1);
  assert.equal(write.section,'models');assert.equal(write.makeDefault,true);assert.equal(write.mode,'create');
  assert.equal(write.settings.modelProfiles.length,1);assert.equal(write.settings.modelProfiles[0].apiKey,host.apiKey);
  assert.equal(host.connectionRef.policyId,'owner-1');assert.equal(host.connectionRef.revision,1);assert.equal(host.connectionRef.modelProfileId,write.settings.activeModelProfileId);
  assert.ok(h.calls.findIndex(call=>call[0]==='save')<h.calls.findIndex(call=>call[0]==='host-provider'));
  assert.equal(h.stored.size,0);
  assert.equal(h.state.selected.value.reference.policyId,'owner-1');
});
test('Host failure retains the real Rust acknowledgement and retry does not create another owner',async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;h.state.useScan.value=true;h.failHost(1);
  await h.state.testConnection();await h.state.saveConnection();
  assert.match(h.state.error.value,/fixture-host-unavailable/);assert.equal(h.state.workflow.value.rustDone,true);assert.equal(h.state.workflow.value.scanDone,true);
  await h.state.retryChatOnly();
  assert.equal(h.calls.filter(call=>call[0]==='save').length,1);
  assert.equal(h.calls.filter(call=>call[0]==='host-provider').length,2);
  assert.equal(h.state.workflow.value,undefined);assert.equal(h.state.error.value,'');
});
test('local no-auth uses explicit none/clear rather than retaining a prior cloud key',async t=>{
  const h=fixture(t);await h.load();configure(h.state,true);h.state.useChat.value=true;h.state.useScan.value=false;
  await h.state.testConnection();await h.state.saveConnection();
  const provider=h.calls.find(call=>call[0]==='host-provider')[1];
  assert.equal(provider.authType,'none');assert.equal(provider.clearApiKey,true);assert.equal(provider.apiKey,'');
  assert.equal(h.calls.find(call=>call[0]==='save')[1].makeDefault,false);
});
test('chat-only retry refuses to overwrite another Host selection after provider acknowledgement',async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;h.failRuntime(1);
  await h.state.testConnection();await h.state.saveConnection();
  assert.match(h.state.error.value,/fixture-runtime-unavailable/);assert.equal(h.calls.filter(x=>x[0]==='host-provider').length,1);
  const newer=clone(h.getHost());newer.apiProvider.model='other-model';newer.apiProvider.connectionRef={policyId:'other-owner',revision:1,kind:'models',modelProfileId:'other'};h.setHost(newer);
  await h.state.retryChatOnly();
  assert.match(h.state.error.value,/changed elsewhere/);assert.equal(h.calls.filter(x=>x[0]==='host-provider').length,1);assert.equal(h.calls.filter(x=>x[0]==='save').length,1);assert.equal(h.getHost().apiProvider.model,'other-model');
});
const tick=()=>new Promise(resolve=>setImmediate(resolve));
test('unmounted workflow retains a late Rust acknowledgement without starting Host writes',async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;h.state.useScan.value=false;await h.state.testConnection();
  let finish;const save=h.api.saveCategory;
  h.api.saveCategory=async input=>{const acknowledged=await save(input);return new Promise(resolve=>finish=()=>resolve(acknowledged));};
  const pending=h.state.saveConnection();await tick();assert.equal(typeof finish,'function');h.unmount();finish();await pending;
  assert.equal(h.calls.filter(x=>x[0]==='save').length,1);assert.equal(h.state.workflow.value.rustDone,true);assert.equal(h.stored.size,0);
  assert.equal(h.calls.filter(x=>x[0]==='host-provider'||x[0]==='runtime'||x[0]==='default').length,0);assert.match(h.state.error.value,/page was left or reloaded/);
});
for(const invalidation of ['unmount','reload'])test(`${invalidation} stops a late Host read from overwriting a newer connection`,async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;h.state.useScan.value=false;await h.state.testConnection();
  let finish;const oldRead=clone(h.getHost()),get=h.client.getSettings;
  h.client.getSettings=()=>{h.client.getSettings=get;return new Promise(resolve=>finish=resolve);};
  const pending=h.state.saveConnection();await tick();assert.equal(typeof finish,'function');
  const newer=clone(h.getHost());newer.runtime.id='xai-api';newer.apiProvider={configured:true,model:'new-user-selection',connectionRef:{policyId:'newer',revision:1,kind:'models',modelProfileId:'newer'}};h.setHost(newer);
  if(invalidation==='unmount')h.unmount();else await h.load();
  finish(oldRead);await pending;
  assert.equal(h.calls.filter(x=>x[0]==='host-provider'||x[0]==='runtime').length,0);assert.equal(h.getHost().apiProvider.model,'new-user-selection');assert.equal(h.state.workflow.value.rustDone,true);
});
test('provider acknowledgement after unmount never initiates the separate runtime write',async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;h.state.useScan.value=false;await h.state.testConnection();
  let finish;const set=h.client.setProvider;
  h.client.setProvider=async input=>{const acknowledged=await set(input);return new Promise(resolve=>finish=()=>resolve(acknowledged));};
  const pending=h.state.saveConnection();await tick();assert.equal(typeof finish,'function');h.unmount();finish();await pending;
  assert.equal(h.calls.filter(x=>x[0]==='host-provider').length,1);assert.equal(h.calls.filter(x=>x[0]==='runtime').length,0);assert.equal(h.state.workflow.value.rustDone,true);
});
test('OAuth is a real chat-only directory item and never creates or changes a Rust scanning owner',async t=>{
  const h=fixture(t);await h.load();h.state.selectConnection('grok-oauth');h.state.useChat.value=true;h.state.useScan.value=true;
  await h.state.applyOAuth();
  assert.deepEqual(h.calls.filter(call=>['save','default','host-provider'].includes(call[0])),[]);
  assert.equal(h.calls.filter(call=>call[0]==='oauth-select').length,1);
  assert.match(source,/Use for scanning \(unsupported\)/);
});
test('unconfirmed Rust save stores only UUID and prevents Host synchronization',async t=>{
  const h=fixture(t);await h.load();configure(h.state);h.state.useChat.value=true;
  h.failSave('configuration_category_commit_unconfirmed_retry_same_request');
  await h.state.testConnection();await h.state.saveConnection();
  assert.equal(h.calls.filter(call=>call[0]==='host-provider').length,0);assert.equal(h.state.workflow.value.rustDone,false);
  const first=h.calls.find(call=>call[0]==='save')[1];
  assert.deepEqual([...h.stored.values()],[first.requestId]);assert.equal(JSON.stringify([...h.stored]).includes('fixture-api-key'),false);
  await h.state.continueWorkflow();
  assert.deepEqual(h.calls.filter(call=>call[0]==='save').map(call=>call[1]),[first,first]);
});
test('same endpoint/model without an exact typed binding is not silently shown as the same connection',()=>{
  const connection={reference:{policyId:'one',revision:1,kind:'models'},profile:{id:'model'}};
  assert.equal(modelHelpers.matchesModelBinding(undefined,connection),false);
  assert.equal(modelHelpers.matchesModelBinding({policyId:'another',revision:1,kind:'models',modelProfileId:'model'},connection),false);
  assert.equal(modelHelpers.matchesModelBinding({policyId:'one',revision:1,kind:'models',modelProfileId:'model'},connection),true);
});
test('unique Models page template compiles and no separate chat/scan model menu remains',()=>{
  const compiled=compileTemplate({filename,id:'models',source:descriptor.template.content,compilerOptions:{bindingMetadata:script.bindings}});
  assert.deepEqual(compiled.errors,[]);
  const app=fs.readFileSync('src/App.vue','utf8');
  assert.match(app,/<ConfigModelsSection/);assert.doesNotMatch(app,/<ConfigRuntimeModelSection/);
  assert.doesNotMatch(app,/@click="openConfigSection\('scan-models'\)"/);
  assert.doesNotMatch(source,/<AgentRuntimeSettingsPanel|<ProfileSectionEditor/);
});
