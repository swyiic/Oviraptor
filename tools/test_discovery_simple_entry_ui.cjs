// Compiled production outer UI, synthetic IPC only. No real DB, model or network.
const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/DiscoveryWorkbench.vue';
const child='src/features/assets/components/SeedReviewWorkbench.vue';
const background='src/features/assets/components/ContinuousDiscovery.vue';
const view=root=>({roots:[root],seeds:[],settings:{keyConfigured:true,contractVerified:true}});
async function mount(t,commands={}){
 const h=await mountAssetAccuracy(file,{projectId:1,batchReview:true},{events:['collect','results','confirmed','changed'],commands:{seed_review_status:async()=>({run:null,items:[],total:0,roots:[]}),seed_review_assist_status:async()=>({phase:'idle',enabled:false,reason:'',hasLocalAssets:false,review:{run:null,items:[],total:0,roots:[]},proposal:null}),enrichment_action:async()=>({roots:[],run:null,events:[],conflicts:[]}),get_asset_discovery_page:async()=>view('saved.example.com'),...commands}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}
test('default discovery mounts one AI workbench without manual-tool inventory or background panels',async t=>{
 const h=await mount(t);
 assert.ok(h.state(child));assert.equal(h.state(background),undefined);assert.equal(h.state().advanced.value,false);
 assert.equal(h.calls.some(([name])=>['get_asset_discovery_page','lookup_local_discovery','preview_hunter_evidence','analyze_local_discovery'].includes(name)),false);
 assert.doesNotMatch(h.text(),/保存确认范围|自动整理本地线索|先查本地，再精确补查|可用线索库/);
 await h.click('data-discovery-action','collect');assert.ok(h.events.some(([kind])=>kind==='collect'));

});
test('manual tools mount only on request and return leaves background work intact',async t=>{
 const h=await mount(t);await h.click('data-discovery-action','advanced');
 assert.equal(h.state().advanced.value,true);assert.ok(h.state(background));assert.equal(h.state(child),undefined);
 assert.equal(h.calls.filter(([name])=>name==='get_asset_discovery_page').length,1);assert.match(h.text(),/高级发现工具|保存确认范围/);
 await h.click('data-discovery-action','return');assert.equal(h.state().advanced.value,false);assert.ok(h.state(child));assert.equal(h.state(background),undefined);
 assert.equal(h.calls.some(([name])=>/pause|cancel|delete|start_hunter/.test(name)),false);
});
test('scope saved in AI mode is re-read when manual tools reopen, replacing only their old draft',async t=>{
 let saved='old.example.com';const h=await mount(t,{get_asset_discovery_page:async()=>view(saved)});
 await h.state().openAdvanced();h.state().rootsText.value='stale-draft.example.com';h.state().closeAdvanced();await flush();saved='new.example.com';
 await h.state().scopeConfirmed();await h.state().openAdvanced();await flush();
 assert.equal(h.state().rootsText.value,'new.example.com');assert.ok(h.events.some(([kind])=>kind==='confirmed'));
 assert.equal(h.calls.some(([name])=>name==='act_asset_discovery'),false);
});
test('closing and reopening advanced tools fences late old reads from the current saved roots',async t=>{
 const old=deferred();let reads=0;const h=await mount(t,{get_asset_discovery_page:async()=>++reads===1?old.promise:view('current.example.com')});
 const first=h.state().openAdvanced();await flush();h.state().closeAdvanced();await h.state().openAdvanced();old.resolve(view('stale.example.com'));await first;await flush();
 assert.equal(h.state().rootsText.value,'current.example.com');assert.equal(h.state().data.value.roots[0],'current.example.com');
});
test('asset result page does not mount the full autonomous probe form above the inventory',()=>{
 const fs=require('node:fs'),path=require('node:path');
 const source=fs.readFileSync(path.join(__dirname,'../src/features/assets/workspace/AssetsPluginWorkspace.vue'),'utf8');
 const inventory=source.slice(source.indexOf('<template v-else-if="activeView===\'assets\'">'),source.indexOf('<AssetOwnershipGate'));
 assert.match(inventory,/<AssetWorkspace/);assert.doesNotMatch(inventory,/AutonomousProbeWorkbench/);
 const discovery=fs.readFileSync(path.join(__dirname,'..',file),'utf8');assert.match(discovery,/<AutonomousProbeWorkbench v-if="manualProbe"/);
});
test('empty-project primary action uses its durable owner while manual collection remains an explicit secondary exit',async t=>{
 const h=await mount(t,{control_seed_assist:async({input})=>({phase:'collecting',enabled:true,reason:'首采',organization:input.organization,hasLocalAssets:false,review:{run:null,items:[],total:0,roots:[]},proposal:null,pipeline:{phase:'collecting',round:1,collectionRunId:44,allowHunter:false,counts:{},hunter:{queries:0,pages:0,spentPoints:0,costUnknown:false},reason:'首采'}})});
 h.state(child).organization.value='测试单位';await flush();await h.click('data-ai-action','start');
 assert.equal(h.calls.filter(([name])=>name==='control_seed_assist').length,1);assert.equal(h.events.some(([kind])=>kind==='collect'),false);
 assert.equal(h.calls.some(([name])=>['start_job','import_targets','start_hunter_expansion','confirm_seed_review_proposal'].includes(name)),false);
});
test('real child result and confirmation events cross the outer workbench without manual-tool reads',async t=>{
 const run={id:'review-1',organization:'测试单位',status:'completed',totalAssets:12,processedAssets:12,cursor:12,maxId:12,groups:4,counts:{},queueCounts:{},manualCounts:{},model:'test-model',requests:1,inputTokens:10,outputTokens:5,unknownRequests:0,reason:''};
 const h=await mount(t,{seed_review_assist_status:async()=>({phase:'ready',enabled:false,reason:'',hasLocalAssets:true,review:{run,items:[],total:0,roots:['saved.example.com']},proposal:null})});
 await h.click('data-ai-action','results');assert.equal(h.events.filter(([kind])=>kind==='results').length,1);
 assert.equal(typeof h.state(child).emit,'function');h.state(child).emit('confirmed');await flush();
 assert.equal(h.events.filter(([kind])=>kind==='confirmed').length,1);assert.equal(h.events.filter(([kind])=>kind==='changed').length,1);
 assert.equal(h.calls.some(([name])=>name==='get_asset_discovery_page'),false);
 assert.equal(h.state(background),undefined);
});
