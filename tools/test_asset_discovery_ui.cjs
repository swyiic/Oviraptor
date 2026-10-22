// Actual compiled Vue components with synthetic IPC; no live provider or DB.
const test = require('node:test');
const { assert, flush, deferred, textOf, mountAssetAccuracy } = require('./asset_accuracy_ui_harness.cjs');
const WORKBENCH='src/features/assets/components/DiscoveryWorkbench.vue';
const SETTINGS='src/features/configuration/components/HunterAccountSettings.vue';
test('failed certificate admission cannot dispatch a paid Hunter request',async t=>{
 const v={roots:['example.com'],seeds:[],settings:{keyConfigured:true,contractVerified:true,maxRequests:3,maxResults:10}};
 const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>v,preview_hunter_evidence:async()=>({eligible:false,reason:'证书属于其他域名，保持待核验'})});
 h.state().hunterKind.value='fingerprint';h.state().target.value='b'.repeat(64);await h.state().act('hunter');await flush();
 assert.equal(h.calls.some(([name])=>name==='start_hunter_expansion'),false);assert.match(h.text(),/证书属于其他域名/);
});
const view=(root='example.com')=>({roots:[root],seeds:[],settings:{keyConfigured:true,contractVerified:false,maxRequests:3,maxResults:10}});
async function mount(t,file,props,commands){
  const h=await mountAssetAccuracy(file,props,{commands:{seed_review_status:async()=>({run:null,items:[],total:0}),seed_review_assist_status:async()=>({phase:'idle',enabled:false,reason:'',hasLocalAssets:true,review:{run:null,items:[],total:0},proposal:null}),enrichment_action:async()=>({roots:[],run:null,events:[],conflicts:[]}),preview_hunter_evidence:async()=>({eligible:true,reason:'受控测试中的已核实证书域名'}),...commands}});
  if(file===WORKBENCH){void h.state().openAdvanced();await flush();const rawCalls=h.calls;Object.defineProperty(h,'calls',{get:()=>rawCalls.filter(([name])=>!['seed_review_status','seed_review_assist_status','enrichment_action'].includes(name))});}
  t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}
test('discovery load never sends external queries and unverified Hunter stays disabled',async t=>{
  const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>view()});
  assert.deepEqual(h.calls.map(c=>c[0]),['get_asset_discovery_page']);
  const hunter=h.nodes().find(n=>n.type==='button'&&textOf(n).includes('Hunter 补查'));
  assert.equal(hunter.props.disabled,true);
  await h.state().act('hunter');await flush();
  assert.equal(h.calls.length,1);assert.match(h.text(),/实际查询暂未启用/);
});
test('late project responses cannot overwrite current roots or seed selection',async t=>{
  const old=deferred();const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async({projectId})=>projectId===1?old.promise:view('current.example.com')});
  h.props.projectId=2;await flush();await h.state().openAdvanced();
  old.resolve(view('old.example.com'));await flush();
  assert.equal(h.state().target.value,'current.example.com');
  assert.equal(h.state().data.value.roots[0],'current.example.com');
  assert.equal(h.state().busy.value,false);
});
test('external CNAME and certificate wildcard remain visible leads without automatic scope expansion',async t=>{
  const v=view();v.seeds=[{id:1,host:'shared.cloud.test',source:'dns_cname',reference:'asset:3',wildcard:false,eligible:false},{id:2,host:'example.com',source:'certificate_cn',reference:'asset:4',wildcard:true,eligible:false}];
  const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>v});
  assert.match(h.text(),/外部线索，待确认/);assert.match(h.text(),/通配符范围，非已发现主机/);
  assert.equal(h.nodes().filter(n=>n.type==='button'&&textOf(n)==='选择').every(n=>n.props.disabled),true);
  assert.equal(h.calls.length,1);
});
test('Hunter settings preserve saved key on blank input and clear typed secrets after saving',async t=>{
  let saved;
  const h=await mount(t,SETTINGS,{}, {get_hunter_discovery_settings:async()=>({revision:2,...view().settings}),save_hunter_discovery_settings:async({input})=>{saved=input;return{revision:3,...view().settings};}});
  await h.state().save();assert.equal(saved.key,null);assert.equal(saved.clearKey,false);
  h.state().key.value='synthetic-key';h.state().visible.value=true;
  await h.state().save();await flush();assert.equal(saved.key,'synthetic-key');
  assert.equal(h.state().key.value,'');assert.equal(h.state().visible.value,false);
  assert.equal(h.calls.some(c=>c[0]==='act_asset_discovery'),false);
});
test('documented exact Hunter mode starts durable automatic pagination with the selected real fingerprint',async t=>{
  const v=view();v.settings.contractVerified=true;let input;
  const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>v,start_hunter_expansion:async args=>{input=args.input;return 'synthetic-query';}});
  assert.equal(h.calls.length,1);
  h.state().hunterKind.value='fingerprint';h.state().target.value='a'.repeat(64);await h.state().act('hunter');await flush();
  assert.equal(input.kind,'fingerprint');assert.equal(input.target,'a'.repeat(64));assert.equal(input.projectId,1);
  assert.equal(h.calls.some(c=>c[0]==='act_asset_discovery'),false);assert.match(h.text(),/已启动精确关联自动分页/);
});
test('Hunter connection test requests only account information',async t=>{
  const h=await mount(t,SETTINGS,{}, {get_hunter_discovery_settings:async()=>({revision:1,...view().settings}),test_hunter_discovery_api:async()=>({type:'个人账号',rest_free_point:500})});
  await h.state().testConnection();await flush();
  assert.deepEqual(h.calls.map(c=>c[0]),['get_hunter_discovery_settings','test_hunter_discovery_api']);
  assert.match(h.text(),/未查询资产/);
});
const PROGRESS='src/features/assets/components/HunterExpansionProgress.vue';
const progress=(status='paused')=>({id:'q1',kind:'fingerprint',target:'a'.repeat(64),query:'cert.sha-256="'+ 'a'.repeat(64)+'"',status,pageSize:100,nextPage:4,total:1000,received:300,uniqueRows:280,rejected:0,spentPoints:300,costUnknown:false,account:{rest_free_point:0,rest_equity_point:9000},reason:'合成分页回执',pages:[{page:3,state:'imported',points:100,consumeQuota:'消耗积分：100',restQuota:'今日剩余积分：9000',returned:100,accepted:90,rejected:0}]});
test('pagination progress displays actual page cost, aggregate cost, duplicate count, equity and next cursor',async t=>{
 const h=await mount(t,PROGRESS,{projectId:1,items:[progress()]},{});assert.match(h.text(),/已核对消耗 300 积分/);assert.match(h.text(),/去重后 280 条/);assert.match(h.text(),/下一页 4/);assert.match(h.text(),/权益余额 9000/);assert.match(h.text(),/消耗积分：100/);
});
test('unknown remote outcome never offers automatic replay and is not rendered as free',async t=>{
 const v={...progress('unknown'),costUnknown:true};const h=await mount(t,PROGRESS,{projectId:1,items:[v]},{});assert.match(h.text(),/消耗未确认，不能当作 0/);assert.equal(h.nodes().some(n=>n.type==='button'&&textOf(n).includes('继续')),false);
});
test('pause/resume forwards the exact project and query identity with no new search or prompt',async t=>{
 let args;const h=await mount(t,PROGRESS,{projectId:1,items:[progress()]},{resume_hunter_expansion:async a=>{args=a;return'q1'}});await h.state().act('resume','q1');assert.deepEqual(args,{projectId:1,queryId:'q1'});assert.equal(h.calls.length,1);
});
test('late pagination resume response cannot change another project state',async t=>{
 const waiting=deferred();const h=await mount(t,PROGRESS,{projectId:1,items:[progress()]},{resume_hunter_expansion:()=>waiting.promise});const pending=h.state().act('resume','q1');h.props.projectId=2;await flush();waiting.resolve('q1');await pending;assert.equal(h.state().busy.value,'');assert.equal(h.state().error.value,'');
});

// Repro: a pasted key must be saved before the primary action validates it.
test('primary Hunter action persists typed key before verification and keeps saved state after 403',async t=>{
 let saved;
 const h=await mount(t,SETTINGS,{}, {get_hunter_discovery_settings:async()=>({revision:1,...view().settings}),save_hunter_discovery_settings:async({input})=>{saved=input;return{revision:2,...view().settings,keyConfigured:true};},test_hunter_discovery_api:async()=>{throw new Error('HTTP 403');}});
 h.state().key.value='synthetic-new-key';await flush();assert.match(h.text(),/保存并验证 API Key/);
 await h.state().testConnection();await flush();
 assert.equal(saved.key,'synthetic-new-key');assert.equal(saved.networkMode,'direct');
 assert.deepEqual(h.calls.map(c=>c[0]),['get_hunter_discovery_settings','save_hunter_discovery_settings','test_hunter_discovery_api']);
 assert.equal(h.state().settings.value.keyConfigured,true);assert.equal(h.state().key.value,'');assert.match(h.text(),/API Key 已保存；账户验证失败/);
});
test('failed save retains typed key and never verifies a different saved key',async t=>{
 const h=await mount(t,SETTINGS,{}, {get_hunter_discovery_settings:async()=>({revision:1,...view().settings}),save_hunter_discovery_settings:async()=>{throw new Error('revision conflict');}});
 h.state().key.value='synthetic-new-key';await h.state().testConnection();await flush();
 assert.equal(h.state().key.value,'synthetic-new-key');assert.equal(h.calls.some(c=>c[0]==='test_hunter_discovery_api'),false);
});
test('repeated primary clicks while save is pending cannot duplicate persistence or test calls',async t=>{
 const pending=deferred();const h=await mount(t,SETTINGS,{}, {get_hunter_discovery_settings:async()=>({revision:1,...view().settings}),save_hunter_discovery_settings:()=>pending.promise,test_hunter_discovery_api:async()=>({rest_free_point:500})});
 h.state().key.value='synthetic-key';const first=h.state().testConnection();await h.state().testConnection();pending.resolve({revision:2,...view().settings,keyConfigured:true});await first;
 assert.equal(h.calls.filter(c=>c[0]==='save_hunter_discovery_settings').length,1);assert.equal(h.calls.filter(c=>c[0]==='test_hunter_discovery_api').length,1);
});

const harvest=(status='running')=>({id:'local-run-1',status,total:70013,processed:1200,skipped:0,added:1150,certificates:19,reason:'后台本地提取',updatedAt:'2026-10-20 10:00:00'});
const local=(overrides={})=>({target:'example.com',kind:'domain',matchCount:1,endpointMatches:1,certificateMatches:1,rawCertificateMatches:1,certificateCount:1,certificates:[{fingerprint:'a'.repeat(64),domains:['example.com'],assetIds:[7],eligible:true}],assets:[{id:7,host:'https://example.com',matchType:'endpoint',certificatePresent:true}],inspectionTruncated:false,certificatePreviewTruncated:false,harvest:harvest('completed'),...overrides});
test('one click starts a durable full local extraction without issuing manual batch actions or paid requests',async t=>{
 const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>view(),start_asset_discovery_harvest:async()=>harvest()});
 await h.state().startHarvest();await flush();assert.equal(h.state().data.value.harvest.id,'local-run-1');assert.equal(h.state().harvesting.value,true);assert.match(h.text(),/70,013/);
 assert.deepEqual(h.calls.map(c=>c[0]),['get_asset_discovery_page','start_asset_discovery_harvest']);assert.equal(h.calls.some(c=>c[0]==='act_asset_discovery'),false);
});
test('local pause and resume preserve project/run identity and never reset the cursor in the UI',async t=>{
 let paused;const v={...view(),harvest:harvest()};const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>v,pause_asset_discovery_harvest:async a=>{paused=a;return harvest('paused')},start_asset_discovery_harvest:async()=>harvest()});
 await h.state().pauseHarvest();assert.deepEqual(paused,{projectId:1,runId:'local-run-1'});assert.match(h.text(),/继续本地提取/);await h.state().startHarvest();assert.equal(h.state().data.value.harvest.processed,1200);
});
test('local certificate match is reused without network or paid Hunter action',async t=>{
 const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>view(),lookup_local_discovery:async()=>local()});
 await h.state().checkLocal();await flush();assert.match(h.text(),/本地已有证书信息/);assert.match(h.text(),/未发出外部请求/);assert.equal(h.calls.filter(c=>c[0]==='lookup_local_discovery').length,1);
 h.state().useFingerprint('a'.repeat(64));assert.equal(h.state().target.value,'a'.repeat(64));assert.equal(h.state().hunterKind.value,'fingerprint');assert.equal(h.calls.some(c=>c[0]==='start_hunter_expansion'),false);
});
test('absence before full extraction is disclosed and raw certificates without fingerprints are not invented',async t=>{
 let result=local({matchCount:0,endpointMatches:0,certificateMatches:0,certificateCount:0,certificates:[],assets:[],harvest:null});const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>view(),lookup_local_discovery:async()=>result});await h.state().checkLocal();await flush();assert.match(h.text(),/暂未发现/);assert.match(h.text(),/全量提取尚未完成/);
 result=local({certificateCount:0,certificates:[]});await h.state().checkLocal();await flush();assert.match(h.text(),/不会用域名冒充指纹/);
});
test('late local match cannot leak certificate or selection into the next project',async t=>{
 const pending=deferred();const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async()=>view(),lookup_local_discovery:()=>pending.promise});const request=h.state().checkLocal();h.props.projectId=2;await flush();pending.resolve(local());await request;assert.equal(h.state().localMatch.value,undefined);assert.equal(h.state().localError.value,'');
});
test('search and pagination query the complete server index and preserve unsaved scope/target drafts',async t=>{
 const v={...view(),seedTotal:503,seedPageSize:50,seedPage:1,allSeeds:503};let received;
 const h=await mount(t,WORKBENCH,{projectId:1},{get_asset_discovery_page:async args=>{received=args;return {...v,seedPage:args.page}}});h.state().rootsText.value='draft.example.com';h.state().target.value='chosen.example.com';h.state().seedSearch.value='tail';h.state().seedSource.value='certificate';await h.state().changePage(11);
 assert.deepEqual(received,{projectId:1,search:'tail',source:'certificate',page:11});assert.equal(h.state().rootsText.value,'draft.example.com');assert.equal(h.state().target.value,'chosen.example.com');
});
