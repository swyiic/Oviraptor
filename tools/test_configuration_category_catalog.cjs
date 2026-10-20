const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const sections=loadPure('src/features/configuration/configProfileSections.ts');
const helpers=loadPure('src/features/configuration/categoryState.ts');
const defaults=loadPure('src/features/configuration/configProfileDefaults.ts');
global.crypto=require('node:crypto').webcrypto;
const clone=x=>JSON.parse(JSON.stringify(x));
const filename='src/features/configuration/sections/ConfigDomainSettingsSection.vue';
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'category-catalog'});
const tick=()=>new Promise(resolve=>setImmediate(resolve));
const reference=(revision=1,id='probe-one')=>({policyId:id,revision,kind:'probe'});
function category(section='probe',overrides={}){
  const settings=sections.pickSectionSettings(defaults.CONFIG_PROFILE_SETTING_DEFAULTS,section);
  return {section,source:'builtin',reference:null,defaultSelection:null,settings:clone(settings),builtinSettings:clone(settings),legacyAvailable:true,legacyProfileId:null,entries:[],...overrides};
}
function harness(t,initial=category(),profiles=[]){
  const calls=[],events=[],stored=new Map(),unmounted=[];
  global.localStorage={getItem:k=>stored.get(k)??null,setItem:(k,v)=>stored.set(k,v),removeItem:k=>stored.delete(k)};
  let current=clone(initial),failure=null,reads=null;
  const versions=new Map();if(current.reference)versions.set(helpers.categoryReferenceKey(current.reference),clone(current));
  const api={
    getCategory:async(section,ref)=>{
      calls.push(['read',section,clone(ref??null)]);
      if(reads)return reads(section,ref);
      if(section!==current.section)return category(section);
      if(ref){const found=versions.get(helpers.categoryReferenceKey(ref));if(!found)throw Error('unknown fixture version');return clone({...found,defaultSelection:current.defaultSelection,entries:current.entries});}
      return clone(current);
    },
    saveCategory:async input=>{
      calls.push(['save',clone(input)]);if(failure)throw failure;
      const ref={policyId:input.mode==='revise'?input.expectedReference.policyId:'fresh-only-category',revision:input.mode==='revise'?input.expectedReference.revision+1:1,kind:input.section};
      const entry={name:input.name,reference:ref,versions:[ref]};
      if(input.makeDefault)current={...current,source:'owner',reference:ref,settings:clone(input.settings),defaultSelection:{revision:(current.defaultSelection?.revision??0)+1,reference:ref}};
      current.entries=[...current.entries.filter(e=>e.reference.policyId!==ref.policyId),entry];
      versions.set(helpers.categoryReferenceKey(ref),clone({...current,reference:ref,source:'owner',settings:input.settings}));
      return {receipt:{schemaVersion:1,requestId:input.requestId,section:input.section,action:input.mode,reference:ref,defaultSelection:current.defaultSelection},currentDefault:current.defaultSelection,currentState:'unchanged'};
    },
    setCategoryDefault:async input=>{calls.push(['select',clone(input)]);current={...current,source:input.reference?'owner':'builtin',reference:input.reference,defaultSelection:{revision:(current.defaultSelection?.revision??0)+1,reference:input.reference}};return {receipt:{schemaVersion:1,requestId:input.requestId,section:input.section,action:'select',reference:input.reference,defaultSelection:current.defaultSelection},currentDefault:current.defaultSelection,currentState:'unchanged'};},
    getCategoryReceipt:async id=>{calls.push(['receipt',id]);return null;},
    saveProfile:()=>{throw Error('global profile writer forbidden');},saveProfileSection:()=>{throw Error('legacy writer forbidden');},adoptProfilePolicy:()=>{throw Error('legacy writer forbidden');},
  };
  const exports={};new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return {...vue,onUnmounted:fn=>unmounted.push(fn)};
    if(name==='../api')return {configurationApi:api};
    if(name==='../configProfileSections')return sections;
    if(name==='../categoryState')return helpers;
    if(name.endsWith('/i18n'))return {useI18n:()=>({tr:(_zh,en)=>en})};
    if(name.endsWith('.vue'))return {default:{}};
    throw Error(`Unexpected import ${name}`);
  },exports);
  const props=vue.reactive({section:initial.section,profiles,revision:0}),scope=vue.effectScope();
  const state=scope.run(()=>exports.default.setup(props,{expose(){},emit:(...x)=>events.push(x)}));
  t.after(()=>{unmounted.forEach(fn=>fn());scope.stop();});
  return {state,props,calls,events,stored,api,versions,ready:tick,setFailure:value=>failure=value,setReads:value=>reads=value};
}
test('legacy-fallback first save uses actual effective CAS and creates only this category default',async t=>{
  const initial=category('probe',{source:'legacy-fallback',legacyProfileId:4});initial.settings.workers=96;
  const h=harness(t,initial);await h.ready();const draft={...h.state.view.value.settings,workers:4};await h.state.persist(draft);
  const input=h.calls.find(x=>x[0]==='save')[1];assert.equal(input.mode,'create');assert.equal(input.section,'probe');assert.equal(input.makeDefault,true);assert.equal(input.expectedSettings.workers,96);assert.equal(input.settings.workers,4);assert.equal(input.expectedReference,null);assert.equal(input.expectedDefault,null);
  assert.equal(Object.hasOwn(input,'profileId'),false);assert.equal(Object.hasOwn(input.settings,'modelProfiles'),false);assert.equal(h.stored.size,0);assert.deepEqual(h.events,[['changed','probe']]);
});
test('editing a nondefault latest entry revises its exact version without switching any default',async t=>{
  const ref=reference(),initial=category('probe',{source:'owner',reference:ref,defaultSelection:{revision:2,reference:ref},entries:[{name:'Original immutable name',reference:ref,versions:[ref]}]});
  const h=harness(t,initial);await h.ready();h.state.defaultView.value.defaultSelection={revision:3,reference:reference(1,'other')};h.state.defaultView.value.reference=reference(1,'other');
  await h.state.persist({...initial.settings,workers:4});const input=h.calls.find(x=>x[0]==='save')[1];
  assert.equal(input.mode,'revise');assert.equal(input.name,'Original immutable name');assert.deepEqual(input.expectedReference,ref);assert.deepEqual(input.expectedSettings,initial.settings);assert.equal(input.makeDefault,false);assert.equal(h.calls.some(x=>x[0]==='select'),false);
});
test('old version remains immutable and fork creates only the selected category',async t=>{
  const old=reference(1),head=reference(2),initial=category('probe',{source:'owner',reference:old,defaultSelection:{revision:1,reference:old},entries:[{name:'Probe owner',reference:head,versions:[head,old]}]});
  const h=harness(t,initial);await h.ready();assert.equal(h.state.editingHead.value,false);
  h.state.prepareFork({...initial.settings,workers:5});h.state.forkName.value='Probe new';await h.state.saveFork();
  const input=h.calls.find(x=>x[0]==='save')[1];assert.equal(input.mode,'fork');assert.deepEqual(input.expectedReference,old);assert.equal(input.makeDefault,false);assert.equal(input.settings.workers,5);assert.equal(input.section,'probe');assert.equal(Object.hasOwn(input.settings,'proxyUrl'),false);
});
test('builtin preview is not confused with null-ref historical fallback; switching affects only this category',async t=>{
  const h=harness(t,category('probe',{source:'legacy-fallback',legacyProfileId:7}));await h.ready();assert.equal(h.state.selectedIsDefault.value,true);
  await h.state.chooseConfiguration('builtin');assert.equal(h.state.selectedIsDefault.value,false);await h.state.useAsDefault();
  const input=h.calls.find(x=>x[0]==='select')[1];assert.equal(input.section,'probe');assert.equal(input.reference,null);assert.equal(input.expectedDefault,null);
});
test('explicit historical import excludes other categories and disabled credentials',async t=>{
  const historical={id:9,name:'old',settings:{workers:6,fofaKey:'fixture-not-copied',hackerOneToken:'fixture-disabled',unknown:'fixture-not-copied'}};
  const h=harness(t,category(),[historical]);await h.ready();h.state.importLegacy();
  assert.equal(h.state.initialSettings.value.workers,6);assert.equal(JSON.stringify(h.state.initialSettings.value).includes('fixture-'),false);assert.equal(h.calls.some(x=>x[0]==='save'),false);
  await h.state.persist(h.state.initialSettings.value);const input=h.calls.find(x=>x[0]==='save')[1];assert.equal(input.mode,'create');assert.equal(input.section,'probe');assert.equal(input.makeDefault,true);assert.deepEqual(historical.settings,{workers:6,fofaKey:'fixture-not-copied',hackerOneToken:'fixture-disabled',unknown:'fixture-not-copied'});
});
test('unconfirmed command freezes the private payload and retries same UUID without persisting secrets',async t=>{
  const h=harness(t,category('assetApis'));await h.ready();h.setFailure('configuration_category_commit_unconfirmed_retry_same_request');
  await assert.rejects(h.state.persist({...h.state.view.value.settings,fofaKey:'fixture-key-private'}));
  const first=h.calls.find(x=>x[0]==='save')[1];assert.deepEqual([...h.stored.values()],[first.requestId]);assert.equal(JSON.stringify([...h.stored]).includes('fixture-key-private'),false);assert.ok(h.state.pending.value);
  await h.state.retryPending();assert.deepEqual(h.calls.filter(x=>x[0]==='save').map(x=>x[1]),[first,first]);
  h.setFailure(null);await h.state.retryPending();assert.equal(h.state.pending.value,undefined);assert.equal(h.stored.size,0);
});
test('definite CAS rejection keeps draft visible and permits correction using same request id',async t=>{
  const h=harness(t);await h.ready();h.setFailure('configuration_category_rejected:configuration_category_settings_conflict');const draft={...h.state.view.value.settings,workers:6};
  await assert.rejects(h.state.persist(draft));assert.equal(h.state.pending.value,undefined);assert.equal(h.state.view.value.settings.workers,96);assert.match(h.state.error.value,/settings_conflict/);
  const id=[...h.stored.values()][0];h.setFailure(null);await h.state.persist(draft);assert.deepEqual(h.calls.filter(x=>x[0]==='save').map(x=>x[1].requestId),[id,id]);
});
test('late category read cannot replace another page and real category template compiles',async t=>{
  const h=harness(t);await h.ready();let finish;
  h.setReads(section=>section==='probe'?new Promise(resolve=>finish=resolve):Promise.resolve(category(section)));
  const old=h.state.loadCategory(false);h.props.section='collection';await vue.nextTick();await tick();finish(category('probe'));await old;
  assert.equal(h.state.view.value.section,'collection');
  const result=compileTemplate({filename,id:'category-catalog',source:descriptor.template.content,compilerOptions:{bindingMetadata:script.bindings}});assert.deepEqual(result.errors,[]);
});
