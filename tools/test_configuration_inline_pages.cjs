const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {renderToString}=require('@vue/server-renderer');
const {parse,compileScript}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const sections=loadPure('src/features/configuration/configProfileSections.ts');
const defaults=loadPure('src/features/configuration/configProfileDefaults.ts');
const center=loadPure('src/features/configuration/configCenter.ts');
global.crypto=require('node:crypto').webcrypto;
function compile(filename,{api={},inline=false,children={},mounted=[],unmounted=[]}={}){
  const source=fs.readFileSync(filename,'utf8');
  const {descriptor}=parse(source,{filename});
  const script=compileScript(descriptor,{id:filename,inlineTemplate:inline});
  const exports={};
  new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return inline?vue:{...vue,onMounted:fn=>mounted.push(fn),onUnmounted:fn=>unmounted.push(fn)};
    if(name==='@lucide/vue')return new Proxy({},{get:()=>vue.defineComponent({render:()=>vue.h('svg')})});
    if(name.endsWith('/i18n'))return {useI18n:()=>({tr:(_zh,en)=>en})};
    if(name==='../api')return {configurationApi:api};
    if(name==='../configProfileDefaults')return defaults;
    if(name==='../configProfileSections')return sections;
    if(name.endsWith('/rulePacksApi'))return {scanningRulePacksApi:{listSecurityRulePacks:async()=>[]}};
    if(children[name])return {default:children[name]};
    if(name.endsWith('.vue'))return {default:vue.defineComponent({render:()=>vue.h('span')})};
    throw Error(`Unexpected import ${name}`);
  },exports);
  return exports.default;
}
test('all eight category editors render inline controls without modal or forced scope choice',async()=>{
  const editor=compile('src/features/configuration/components/ProfileSectionEditor.vue',{inline:true});
  const baseline={...defaults.CONFIG_PROFILE_SETTING_DEFAULTS,modelProfiles:[]};
  for(const item of sections.CONFIG_PROFILE_SECTIONS){
    const settings=sections.pickSectionSettings(baseline,item.key);
    const category={section:item.key,source:'builtin',reference:null,defaultSelection:null,settings,builtinSettings:settings,entries:[],legacyAvailable:false,legacyProfileId:null};
    const html=await renderToString(vue.createSSRApp(editor,{section:item.key,category,saveLabel:"Save this category",persist:async()=>{}}));
    assert.match(html,/Save this category/);
    assert.doesNotMatch(html,/<dialog|role="dialog"|class="tab-strip"|Profile name/);
  }
  for(const route of center.PROFILE_SETTINGS_NAV) assert.equal(center.resolveConfigSection(route.route),'profile-settings');
  assert.equal(center.resolveConfigSection('scan-models'),'runtime-model');
  const app=fs.readFileSync('src/App.vue','utf8');
  assert.doesNotMatch(app,/<ConfigDialog|<ConfigProfileMetadataDialog|function openProfile\(|function cloneDefaultProfile\(/);
  assert.match(app,/ConfigDomainSettingsSection/);assert.match(app,/ConfigModelsSection/);
  assert.doesNotMatch(app,/data-testid="config-nav-advanced"/);
});
test('preview hides unknown nested credentials and proxy userinfo but keeps real budgets and content keywords',()=>{
  const input={headers:{Authorization:'fixture-auth'},api_key:'fixture-key',credential:'fixture-credential',agentQuickTokenLimit:200000,negativeKeywords:['visible-keyword'],proxyUrl:'socks5h://fixture-user:p%40ss@[::1]:1080'};
  const view=sections.redactedConfigPreview(input),text=JSON.stringify(view);
  for(const secret of ['fixture-auth','fixture-key','fixture-credential','fixture-user','p%40ss'])assert.equal(text.includes(secret),false);
  assert.equal(view.agentQuickTokenLimit,200000);assert.deepEqual(view.negativeKeywords,['visible-keyword']);
  const controlled=sections.configPolicyPreviewSettings({modelProfiles:[{id:'m',apiKey:'fixture-active',metadata:{arbitraryName:'fixture-hidden'}}]},'models');
  assert.equal(JSON.stringify(controlled).includes('fixture-hidden'),false);
});
