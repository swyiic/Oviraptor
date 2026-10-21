// Actual SFC setup/template with synthetic IPC; no live model/provider/database claims.
const test=require('node:test');
const {assert,flush,deferred,textOf,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const ASSETS='src/features/assets/components/AssetWorkspace.vue';
const AGENT='src/features/assets/components/AutonomousProbeWorkbench.vue';
const projects=[{id:91,name:'Synthetic project',status:'active'},{id:92,name:'Other project',status:'active'}];
const asset=(id,projectId=91)=>({id,projectId,host:`host${id}.example.com`,extra:{},isDeleted:false});
const page=query=>({items:[asset(1),asset(2)],total:2,memberTotal:17,page:query.page,pageSize:query.pageSize,summary:{all:17,pending:17,uncertain:0,confirmed:0,rejected:0,notApplicable:0,sentToAgent:0}});
const discovery=(roots=['example.com'])=>({roots,settings:{keyConfigured:true,contractVerified:true,maxResults:100}});
const run=(status='running')=>({id:'synthetic-run',status,roots:['example.com'],model:'synthetic-model',round:1,probed:0,hunterRequests:0,summary:'planning',limits:{maxRounds:8,maxAssets:100,maxMinutes:15,hunterRequests:0},steps:[]});
async function mount(t,file,props,commands,events=[]){const h=await mountAssetAccuracy(file,props,{commands,events});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[])});return h;}
test('filtered reprobe forwards a frozen copy of all active filters with raw member count',async t=>{
 const h=await mount(t,ASSETS,{projects,selectedProjectId:91},{list_assets:async({query})=>page(query)},['reprobe']);
 h.state().query.value.search='specific.example.com';h.state().query.value.hasLoginView='yes';await h.state().refresh();await flush();
 assert.equal(h.state().reprobeCount.value,17);await h.click('data-asset-action','reprobe');const request=h.events[0][1];
 assert.equal(request.projectId,91);assert.equal(request.scope.kind,'query');assert.equal(request.scope.expectedCount,17);assert.equal(request.scope.query.search,'specific.example.com');assert.equal(request.scope.query.hasLoginView,'yes');
 h.state().query.value.search='changed';assert.equal(request.scope.query.search,'specific.example.com');
});
test('checked rows take precedence over filtered matches including off-page selections',async t=>{
 const h=await mount(t,ASSETS,{projects,selectedProjectId:91},{list_assets:async({query})=>page(query)},['reprobe']);
 h.state().toggleAsset(asset(77),true);await flush();await h.click('data-asset-action','reprobe');assert.deepEqual(h.events[0][1].scope,{kind:'selection',selections:[{projectId:91,assetId:77}],expectedCount:1});
 assert.match(h.text(),/Re-probe selected/);
});
test('reprobe rejects cross-project selections and stale counts and trash at the UI boundary',async t=>{
 const h=await mount(t,ASSETS,{projects,selectedProjectId:91},{list_assets:async({query})=>page(query)},['reprobe']);
 h.state().toggleAsset(asset(77,92),true);await flush();assert.equal(h.state().reprobeReady.value,false);h.state().requestReprobe();assert.equal(h.events.length,0);
 h.state().clearSelection();h.state().query.value.search='stale';await flush();assert.equal(h.state().reprobeReady.value,false);
 h.state().query.value.deletedView='trash';await h.state().refresh();assert.equal(h.state().reprobeReady.value,false);
});
test('autonomous mode performs read-only mounting and discloses optional all-points pagination',async t=>{
 const h=await mount(t,AGENT,{projectId:91},{get_asset_discovery:async()=>discovery(),autonomous_probe_status:async()=>null});
 assert.deepEqual(h.calls.map(c=>c[0]).sort(),['autonomous_probe_status','get_asset_discovery']);assert.equal(h.state().limits.value.hunterRequests,0);assert.equal(h.state().limits.value.hunterEnabled,true);assert.match(h.text(),/不设积分预算/);assert.match(h.text(),/自动翻页/);
});
test('start sends one project-bound Agent goal and limits; stop addresses the same run',async t=>{
 let started=false,input;const h=await mount(t,AGENT,{projectId:91},{get_asset_discovery:async()=>discovery(),autonomous_probe_status:async()=>started?run():null,start_autonomous_probe:async({input:i})=>{input=i;started=true;return'synthetic-run'},stop_autonomous_probe:async()=>{}});
 h.state().goal.value='Continue from concrete certificate leads';await h.click('data-agent-action','start');assert.equal(input.projectId,91);assert.equal(input.goal,'Continue from concrete certificate leads');assert.equal(input.limits.hunterRequests,0);
 await h.click('data-agent-action','stop');assert.deepEqual(h.calls.find(c=>c[0]==='stop_autonomous_probe')[1],{projectId:91,runId:'synthetic-run'});assert.equal(h.calls.some(c=>/messages|send_chat/.test(c[0])),false);
});
test('late Agent start response cannot overwrite another project or issue a stop against it',async t=>{
 const start=deferred();const h=await mount(t,AGENT,{projectId:91},{get_asset_discovery:async({projectId})=>discovery([projectId===91?'one.example.com':'two.example.com']),autonomous_probe_status:async()=>null,start_autonomous_probe:()=>start.promise});
 const pending=h.state().start();h.props.projectId=92;await flush();start.resolve('old-run');await pending;await flush();assert.equal(h.state().run.value,null);assert.deepEqual(h.state().discovery.value.roots,['two.example.com']);assert.equal(h.state().busy.value,false);
});
test('missing domain scope or archived project prevents autonomous launch',async t=>{
 const h=await mount(t,AGENT,{projectId:91},{get_asset_discovery:async()=>discovery([]),autonomous_probe_status:async()=>null});assert.equal(h.state().canStart.value,false);await h.state().start();assert.equal(h.calls.length,2);
 h.state().discovery.value=discovery();h.props.archived=true;await flush();assert.equal(h.state().canStart.value,false);
});
test('Hunter permission is enabled only with a saved key and documented contract',async t=>{
 let input;const h=await mount(t,AGENT,{projectId:91},{get_asset_discovery:async()=>discovery(),autonomous_probe_status:async()=>null,start_autonomous_probe:async({input:i})=>{input=i;return'new-run'}});
 h.state().limits.value.hunterEnabled=true;await h.state().start();assert.equal(input.limits.hunterEnabled,true);assert.equal(input.limits.hunterRequests,0);
 h.state().discovery.value.settings.keyConfigured=false;await h.state().start();assert.equal(input.limits.hunterEnabled,false);
});

test('unconfigured Hunter checkbox renders unchecked even when remembered permission was enabled',async t=>{
 const v=discovery();v.settings.keyConfigured=false;
 const h=await mount(t,AGENT,{projectId:91},{get_asset_discovery:async()=>v,autonomous_probe_status:async()=>null});
 const toggle=h.nodes().find(n=>n.type==='input'&&n.props.type==='checkbox');assert.equal(toggle.props.checked,false);assert.equal(toggle.props.disabled,true);assert.match(h.text(),/配置 Hunter API Key/);
});
