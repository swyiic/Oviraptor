// Compiled production outer UI, synthetic IPC only. No real DB, model or network.
const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/DiscoveryWorkbench.vue';
const child='src/features/assets/components/SeedReviewWorkbench.vue';
const background='src/features/assets/components/ContinuousDiscovery.vue';
const view=root=>({roots:[root],seeds:[],settings:{keyConfigured:true,contractVerified:true}});
async function mount(t,commands={}){
 const h=await mountAssetAccuracy(file,{projectId:1,batchReview:true},{events:['collect','results','confirmed','changed','seeds','configure'],commands:{seed_review_status:async()=>({run:null,items:[],total:0,roots:[]}),seed_review_assist_status:async()=>({phase:'idle',enabled:false,reason:'',hasLocalAssets:false,review:{run:null,items:[],total:0,roots:[]},proposal:null}),enrichment_action:async()=>({roots:[],run:null,events:[],conflicts:[]}),get_asset_discovery_page:async()=>view('saved.example.com'),...commands}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}
test('default discovery mounts one AI workbench without manual-tool inventory or background panels',async t=>{
 const h=await mount(t);
 assert.ok(h.state(child));assert.equal(h.state(background),undefined);assert.equal(h.state().advanced.value,false);
 assert.equal(h.calls.some(([name])=>['get_asset_discovery_page','lookup_local_discovery','preview_hunter_evidence','analyze_local_discovery'].includes(name)),false);
 assert.doesNotMatch(h.text(),/保存确认范围|自动整理本地线索|先查本地，再精确补查|可用线索库/);
 await h.click('data-discovery-action','more');assert.equal(h.calls.some(([name])=>name==='get_asset_discovery_page'),false);await h.click('data-discovery-action','collect');assert.ok(h.events.some(([kind])=>kind==='collect'));

});
test('manual tools mount only on request and return leaves background work intact',async t=>{
 const h=await mount(t),primary=h.state(child);primary.organization.value='未提交的目标草稿';
 await h.click('data-discovery-action','more');assert.equal(h.state(background),undefined);
 await h.click('data-discovery-tool','enrichment');
 assert.equal(h.state().advanced.value,true);assert.ok(h.state(background));assert.equal(h.state(child),primary);
 assert.equal(h.calls.filter(([name])=>name==='get_asset_discovery_page').length,0);
 await h.click('data-discovery-tool','scope');assert.equal(h.state(background),undefined);assert.match(h.text(),/保存确认范围/);
 assert.equal(h.calls.filter(([name])=>name==='get_asset_discovery_page').length,1);
 await h.click('data-discovery-action','return');assert.equal(h.state().advanced.value,false);assert.equal(h.state(child),primary);assert.equal(h.state(child).organization.value,'未提交的目标草稿');assert.equal(h.state(background),undefined);
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
 const discovery=fs.readFileSync(path.join(__dirname,'..',file),'utf8');assert.match(discovery,/<AutonomousProbeWorkbench v-else-if="manualProbe&&!archived"/);
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

test('records and configuration leave through explicit scoped events without starting work',async t=>{
 const h=await mount(t);await h.click('data-discovery-action','seeds');assert.equal(h.events.filter(([kind])=>kind==='seeds').length,1);
 h.state(child).emit('configure');await flush();assert.equal(h.events.filter(([kind])=>kind==='configure').length,1);
 assert.equal(h.calls.some(([name])=>/control_seed|start_hunter|start_job|confirm_seed/.test(name)),false);
});
test('archived projects expose local inspection but every manual mutation is refused',async t=>{
 const h=await mount(t);h.props.archived=true;await flush();assert.equal(h.state(child).props.archived,true);
 await h.state().openAdvanced('scope');assert.match(h.text(),/仅可查看历史记录/);const before=h.calls.length;
 h.state().rootsText.value='outside.example.com';h.state().target.value='outside.example.com';
 await h.state().act('scopes');await h.state().startHarvest();await h.state().startHunter();await h.state().analyzeSeeds();h.state().collectManually();
 assert.equal(h.calls.length,before);assert.equal(h.events.some(([kind])=>kind==='collect'),false);
 assert.equal(h.matching('data-discovery-tool','enrichment')[0].props.disabled,true);
});
test('manual group selection mounts only its chosen controls and project switch resets expert UI',async t=>{
 const h=await mount(t);await h.state().openAdvanced('local');await flush();assert.match(h.text(),/自动整理本地线索/);assert.doesNotMatch(h.text(),/先查本地，再精确补查/);assert.equal(h.state(background),undefined);
 await h.click('data-discovery-tool','query');assert.match(h.text(),/先查本地，再精确补查/);assert.doesNotMatch(h.text(),/可用线索库/);
 h.props.projectId=2;await flush();assert.equal(h.state().moreOpen.value,false);assert.equal(h.state().advanced.value,false);assert.equal(h.state(background),undefined);
});

test('switching expert groups fences a late local certificate preview without stopping an owner',async t=>{
 const late=deferred();const h=await mount(t,{lookup_local_discovery:()=>late.promise,preview_hunter_evidence:async()=>({eligible:true,reason:'fixture'})});
 await h.state().openAdvanced('query');const preview=h.state().checkLocal();await flush();await h.state().openAdvanced('scope');
 late.resolve({target:'old.example.com',certificateCount:99});await preview;await flush();
 assert.equal(h.state().localMatch.value,undefined);assert.equal(h.state().localLoading.value,false);assert.equal(h.state().activeTool.value,'scope');
 assert.equal(h.calls.some(([name])=>/pause|cancel|start_hunter/.test(name)),false);
});
test('failed local evidence pagination keeps the displayed page and retry requests the same next page',async t=>{
 let fail=false;const requested=[];const h=await mount(t,{get_asset_discovery_page:async args=>{requested.push(args.page);if(fail)throw Error('read failed');return{...view('one.example.com'),seedPage:args.page,seedTotal:120,seedPageSize:50};}});
 await h.state().openAdvanced('local');fail=true;await h.state().changePage(2);assert.equal(h.state().seedPage.value,1);assert.equal(h.state().data.value.seedPage,1);
 assert.match(h.state().readError.value,/read failed/);fail=false;h.state().error.value='保留原业务失败';await h.state().changePage(h.state().seedPage.value+1);assert.deepEqual(requested,[1,2,2]);assert.equal(h.state().data.value.seedPage,2);assert.equal(h.state().readError.value,'');
 h.state().error.value='保留原业务失败';await h.state().refreshProgress();assert.equal(h.state().error.value,'保留原业务失败');
});
