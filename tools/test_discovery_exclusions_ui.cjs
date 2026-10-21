const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/DiscoveryExclusions.vue';
const preview=()=>({domain:'bad.example.net',domainLeads:17,relatedFingerprints:3,protectedSharedFingerprints:2,total:20,scopeConflict:false,digest:'scoped-preview',samples:[{host:'a.bad.example.net',source:'certificate_cn',reference:'asset:1'}]});
async function mount(t,commands){const h=await mountAssetAccuracy(file,{projectId:1,suggestedDomain:'bad.example.net'},{commands,events:['changed']});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[])});return h;}
test('search cleanup requires a domain preview and forwards its exact digest before excluding',async t=>{
 const calls=[];const h=await mount(t,{get_discovery_domain_exclusions:async()=>({domains:[]}),manage_discovery_domain_exclusion:async({input})=>{calls.push(input);return input.action==='preview'?preview():{};}});
 assert.equal(h.calls.length,0);await h.state().open();await h.state().inspect();await flush();
 assert.match(h.text(),/有其他来源的共享指纹保留 2/);assert(calls.every(c=>c.action==='preview'));
 await h.state().apply();assert.equal(calls[1].expectedDigest,'scoped-preview');assert.equal(calls[1].action,'apply');assert.equal(calls[1].domain,'bad.example.net');
 assert.match(h.text(),/原始资产与历史保留/);
});
test('editing a preview invalidates confirmation and restores also require an explicit preview',async t=>{
 const actions=[];const h=await mount(t,{get_discovery_domain_exclusions:async()=>({domains:['bad.example.net']}),manage_discovery_domain_exclusion:async({input})=>{actions.push(input.action);return preview();}});
 await h.state().open();await h.state().inspect();h.state().domain.value='another.example.net';await flush();await h.state().apply();assert.deepEqual(actions,['preview']);
 await h.state().inspect('restore','bad.example.net');assert.deepEqual(actions,['preview','preview']);await h.state().apply();assert.equal(actions.at(-1),'restore');
});
test('a stale project response cannot populate a new projects cleanup preview',async t=>{
 const pending=deferred();const h=await mount(t,{manage_discovery_domain_exclusion:()=>pending.promise});h.state().domain.value='bad.example.net';const request=h.state().inspect();assert.equal(h.calls.length,1);h.props.projectId=2;await flush();pending.resolve(preview());await request;
 assert.equal(h.state().preview.value,undefined);assert.equal(h.state().domains.value.length,0);
});
