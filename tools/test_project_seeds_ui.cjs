// Compiles the production seed page. Scripted IPC fixtures do not establish
// installed-App, historical-database or provider acceptance.
const test=require('node:test');
const fs=require('node:fs');
const path=require('node:path');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/ProjectSeedsWorkspace.vue';
const seed=(id=1,more={})=>({id,kind:'domain',value:`asset${id}.example.com`,trustStatus:'needs_evidence',trustReason:'No domain-bound ownership evidence',firstSeenRound:null,firstSeenAt:null,lastSeenRound:null,lastSeenAt:'2026-10-23T01:02:03Z',historicalBaseline:true,sourceTypes:['fofa','historical_baseline'],assetCount:3,observationCount:2,hunterEligible:false,...more});
const page=(projectId=7,more={})=>({projectId,ready:true,state:'ready',page:1,pageSize:50,total:1,items:[seed()],summary:{total:9,verified:1,needsEvidence:6,conflicts:1,excluded:1,historicalBaseline:8},rounds:[],...more});
const detail=(id=1,more={})=>({projectId:7,seed:seed(id),observations:[],observationsTotal:0,page:1,pageSize:20,...more});
async function mount(t,commands={},props={}){
 const h=await mountAssetAccuracy(file,{projectId:7,...props},{locale:'zh',events:['discover'],commands:{list_project_seeds:async()=>page(),read_project_seed:async({seedId})=>detail(seedId),...commands}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}
test('project seeds reads only its selected project and separates actual evidence from AI accuracy',async t=>{
 const h=await mount(t,{list_project_seeds:async()=>page(7,{items:[seed(1,{kind:'fingerprint',value:'a'.repeat(64),trustStatus:'verified',verificationCoverage:{verifiedEvidence:2,totalEvidence:5},manualConclusion:{accepted:1,rejected:1,reviewed:2,total:5,acceptanceRate:0.5},sourceTypes:['fofa','hunter','tls','ct','dns']})]}),read_project_seed:async()=>detail(1,{seed:seed(1,{verificationCoverage:{verifiedEvidence:2,totalEvidence:5},manualConclusion:{accepted:1,rejected:1,reviewed:2,total:5,acceptanceRate:0.5}})})});
 assert.deepEqual(h.calls,[['list_project_seeds',{query:{projectId:7,search:'',kind:'all',trust:'all',page:1,pageSize:50}}]]);
 assert.match(h.text(),/历史基线 · 轮次未知/);assert.match(h.text(),/时间未记录/);assert.match(h.text(),/FOFAHunterTLS/);
 assert.match(h.text(),/缺少核验证据 6/);assert.doesNotMatch(h.text(),/自动待补证/);
 await h.click('data-seed-detail',1);assert.match(h.text(),/缺少核验证据/);assert.doesNotMatch(h.text(),/自动待补证/);assert.match(h.text(),/独立核验依据 2 条 · 历史观察记录 5 条/);assert.match(h.text(),/不作为核验证据分母/);assert.doesNotMatch(h.text(),/核验覆盖率/);assert.match(h.text(),/人工已审样本认可率 50.0%/);assert.match(h.text(),/不是 AI 准确率/);
 assert.equal(h.calls.some(([name])=>/hunter|control_seed|start_job|confirm/.test(name)),false);
 await h.click('data-seeds-action','discover');assert.deepEqual(h.events,[['discover']]);assert.equal(h.calls.length,2);
});
test('history keeps exact queries and later pages including unresolved charge without replay',async t=>{
 const h=await mount(t,{read_project_seed:async({page:historyPage,pageSize})=>{assert.equal(pageSize,20);return detail(1,{page:historyPage,observationsTotal:21,observations:[historyPage===1?{id:31,sourceType:'hunter',round:4,seenAt:'2026-10-23T01:02:03Z',queryKind:'certificate_sha256',queryValue:'cert.sha256="'+'b'.repeat(64)+'"',queryId:'original-query',page:12,spentPoints:10,assetCount:5,evidence:{organization:'target'}}:{id:32,sourceType:'hunter',round:4,seenAt:'2026-10-23T01:02:03Z',queryId:'unknown-original-query',page:13,costUnknown:true,spentPoints:null}]});}});
 await h.click('data-seed-detail',1);assert.match(h.text(),/第 4 轮/);assert.match(h.text(),/第 12 页/);assert.match(h.text(),/10 积分/);assert.match(h.text(),/cert.sha256/);
 await h.click('data-seeds-action','next-history');assert.match(h.text(),/第 13 页/);assert.match(h.text(),/扣费待核对/);assert.match(h.text(),/unknown-original-query/);
 assert.deepEqual(h.calls.filter(([name])=>name==='read_project_seed').map(([,args])=>args),[{projectId:7,seedId:1,page:1,pageSize:20},{projectId:7,seedId:1,page:2,pageSize:20}]);
 assert.equal(h.calls.some(([name])=>/start|retry_hunter|expansion|resume/.test(name)),false);
});
test('historical observation timestamp is labelled as ledger time and round records do not invent a baseline round',async t=>{
 const h=await mount(t,{list_project_seeds:async()=>page(7,{rounds:[{number:3,state:'paused',startedAt:'2026-10-23T01:02:03Z',completedAt:null}]}),read_project_seed:async()=>detail(1,{observationsTotal:1,observations:[{id:1,sourceType:'historical_baseline',round:null,seenAt:'2026-10-23T01:02:03Z',historicalBaseline:true}]})});
 assert.match(h.text(),/第 3 轮已暂停/);assert.match(h.text(),/结束时间未记录/);
 await h.click('data-seed-detail',1);assert.match(h.text(),/台账记录时间/);assert.doesNotMatch(h.text(),/第 1 轮/);
});
test('first read prepares local baseline once and keeps uninitialized counts unknown across refreshes',async t=>{
 let ready=false;
 const h=await mount(t,{list_project_seeds:async()=>ready?page():page(7,{ready:false,state:'pending_initialization',total:null,summary:null,items:[]}),prepare_project_seeds:async()=>({state:'preparing'})});
 assert.match(h.text(),/正在整理历史种子/);assert.match(h.text(),/当前数量未知/);assert.doesNotMatch(h.text(),/匹配 0|没有匹配/);
 await h.click('data-seeds-action','refresh');assert.equal(h.calls.filter(([name])=>name==='prepare_project_seeds').length,1);
 ready=true;await h.click('data-seeds-action','refresh');assert.match(h.text(),/asset1.example.com/);assert.equal(h.calls.filter(([name])=>name==='prepare_project_seeds').length,1);
 assert.equal(h.calls.some(([name])=>/control_seed|hunter|start_job/.test(name)),false);
});
test('baseline preparation failure is visible and a refresh does not duplicate mutation or imply an empty index',async t=>{
 const h=await mount(t,{list_project_seeds:async()=>page(7,{ready:false,state:'pending_initialization',items:[],total:null,summary:null}),prepare_project_seeds:async()=>{throw Error('storage_busy');}});
 assert.match(h.text(),/storage_busy/);assert.doesNotMatch(h.text(),/匹配 0|没有匹配/);
 await h.click('data-seeds-action','refresh');assert.equal(h.calls.filter(([name])=>name==='prepare_project_seeds').length,1);
});
test('filter and page reads retain global summary and send the active project without starting discovery',async t=>{
 const h=await mount(t,{list_project_seeds:async({query})=>page(query.projectId,{page:query.page,total:101})});
 h.state().trust.value='verified';h.state().kind.value='domain';await flush();await h.click('data-seeds-action','next');
 const query=h.calls.filter(([name])=>name==='list_project_seeds').at(-1)[1].query;
 assert.deepEqual(query,{projectId:7,search:'',kind:'domain',trust:'verified',page:2,pageSize:50});
 assert.match(h.text(),/项目全部种子 9/);assert.match(h.text(),/匹配 101/);
 assert.equal(h.calls.some(([name])=>name!=='list_project_seeds'),false);
});
test('old project list cannot leak rows or trigger baseline preparation after project changes',async t=>{
 const old=deferred();
 const h=await mount(t,{list_project_seeds:async({query})=>query.projectId===7?old.promise:page(8,{items:[seed(8)]})});
 h.props.projectId=8;await flush();old.resolve(page(7,{ready:false,total:null,summary:null,items:[]}));await flush();
 assert.equal(h.state().data.value.projectId,8);assert.match(h.text(),/asset8.example.com/);assert.equal(h.calls.some(([name])=>name==='prepare_project_seeds'),false);
});
test('late detail from earlier selection cannot replace newer project-bound evidence',async t=>{
 const old=deferred();const h=await mount(t,{list_project_seeds:async()=>page(7,{items:[seed(1),seed(2)]}),read_project_seed:async({seedId})=>seedId===1?old.promise:detail(2,{observationsTotal:1,observations:[{id:22,sourceType:'dns',round:2,seenAt:null,sourceRef:'current-evidence'}]})});
 const first=h.state().selectSeed(seed(1));await flush();await h.click('data-seed-detail',2);old.resolve(detail(1,{observations:[{id:11,sourceType:'history',round:null,seenAt:null,sourceRef:'stale-evidence'}]}));await first;await flush();
 assert.match(h.text(),/current-evidence/);assert.doesNotMatch(h.text(),/stale-evidence/);assert.equal(h.state().detail.value.seed.id,2);
});
test('read errors retain prior snapshot and cross-project response is rejected before exposing evidence',async t=>{
 let fail=false;const h=await mount(t,{list_project_seeds:async()=>{if(fail)throw Error('read_failed');return page();},read_project_seed:async()=>detail(1,{projectId:8,observations:[{id:4,sourceType:'history',round:null,seenAt:null,sourceRef:'other-project-secret'}]})});
 fail=true;await h.click('data-seeds-action','refresh');assert.match(h.text(),/read_failed/);assert.match(h.text(),/显示上次快照/);assert.match(h.text(),/asset1.example.com/);
 await h.click('data-seed-detail',1);assert.match(h.text(),/project_seed_detail_scope_mismatch/);assert.doesNotMatch(h.text(),/other-project-secret/);
});
test('failed list and evidence pagination retain the committed page and retry that same next page',async t=>{
 let listFails=true,detailFails=true;
 const h=await mount(t,{list_project_seeds:async({query})=>{if(query.page===2&&listFails)throw Error('page_2_failed');return page(7,{page:query.page,total:51});},read_project_seed:async({page:historyPage})=>{if(historyPage===2&&detailFails)throw Error('evidence_page_2_failed');return detail(1,{page:historyPage,observationsTotal:21,observations:[{id:historyPage,sourceType:'history',round:null,seenAt:null,sourceRef:`evidence-page-${historyPage}`}]});}});
 await h.click('data-seeds-action','next');assert.equal(h.state().page.value,1);assert.equal(h.state().data.value.page,1);assert.match(h.text(),/匹配 51 · 1 \/ 2/);
 listFails=false;await h.click('data-seeds-action','next');assert.equal(h.state().page.value,2);assert.equal(h.state().data.value.page,2);
 await h.click('data-seed-detail',1);await h.click('data-seeds-action','next-history');assert.equal(h.state().detailPage.value,1);assert.equal(h.state().detail.value.page,1);assert.match(h.text(),/evidence-page-1/);assert.match(h.text(),/21 条历史 · 1 \/ 2/);
 detailFails=false;await h.click('data-seeds-action','next-history');assert.equal(h.state().detailPage.value,2);assert.equal(h.state().detail.value.page,2);assert.match(h.text(),/evidence-page-2/);
});
test('no selected project means no reads or implicit project guesses',async t=>{
 const h=await mount(t,{}, {projectId:undefined});assert.deepEqual(h.calls,[]);assert.match(h.text(),/先选择一个工作空间/);
});
test('native wildcard identity and nullable single asset history remain distinguishable',async t=>{
 const item=seed(1,{wildcard:true,value:'shared.example.com'});
 const h=await mount(t,{list_project_seeds:async()=>page(7,{items:[item]}),read_project_seed:async()=>detail(1,{seed:item,observationsTotal:1,observations:[{id:1,sourceType:'hunter_page',round:null,seenAt:null,historicalBaseline:true,assetId:42,assetCount:null,queryId:'q-1',queryKind:null,queryValue:null,page:3,spentPoints:null,costUnknown:false,evidence:null}]})});
 assert.match(h.text(),/\*\.shared.example.com/);await h.click('data-seed-detail',1);
 assert.match(h.text(),/Hunter 分页/);assert.match(h.text(),/资产编号 · 42/);assert.match(h.text(),/积分未记录/);assert.match(h.text(),/关联资产 未记录/);
});
test('round history uses the first embedded page and reads all later rounds without restarting a run',async t=>{
 const round=(number)=>({number,state:'completed',startedAt:null,completedAt:null});
 const h=await mount(t,{list_project_seeds:async()=>page(7,{rounds:[round(12)],roundsTotal:12}),list_project_seed_rounds:async({projectId,page:roundPage,pageSize})=>{assert.equal(pageSize,10);return {projectId,page:roundPage,pageSize,total:12,items:[round(1)],historyBeforeTracking:'unknown'};}});
 assert.equal(h.calls.some(([name])=>name==='list_project_seed_rounds'),false);assert.match(h.text(),/发现轮次 · 12 轮/);
 await h.click('data-seeds-action','next-rounds');assert.match(h.text(),/第 1 轮/);assert.deepEqual(h.calls.at(-1),['list_project_seed_rounds',{projectId:7,page:2,pageSize:10}]);
 assert.equal(h.calls.some(([name])=>/start|resume|hunter|control/.test(name)),false);
});
test('round pagination failure retains the old page and old-project late history is fenced',async t=>{
 const old=deferred();let fail=true;
 const h=await mount(t,{list_project_seeds:async({query})=>page(query.projectId,{rounds:[{number:12,state:'paused',startedAt:null,completedAt:null}],roundsTotal:12}),list_project_seed_rounds:async()=>{if(fail)throw Error('round_read_failed');return old.promise;}});
 await h.click('data-seeds-action','next-rounds');assert.match(h.text(),/round_read_failed/);assert.equal(h.state().roundData.value.page,1);assert.match(h.text(),/第 12 轮/);
 fail=false;const pending=h.state().loadRoundPage(2);h.props.projectId=8;await flush();old.resolve({projectId:7,page:2,pageSize:10,total:12,items:[{number:99,state:'completed',startedAt:null,completedAt:null}]});await pending;await flush();
 assert.equal(h.state().roundData.value.projectId,8);assert.doesNotMatch(h.text(),/第 99 轮/);
});
test('desktop exposes its connected seed page without advertising unsupported standalone actions',()=>{
 const source=relative=>fs.readFileSync(path.join(__dirname,'..',relative),'utf8');
 const catalog=JSON.parse(source('packages/agent-host/shell-apps.json')).apps.find(item=>item.shellModuleId==='asset');assert.ok(catalog.views.includes('seeds'));assert.ok(catalog.pages.some(item=>item.view==='seeds'));
 assert.match(source('src/features/assets/workspace/AssetsPluginWorkspace.vue'),/ProjectSeedsWorkspace v-else-if="activeView==='seeds'"/);
 assert.doesNotMatch(source('apps/assets/ui/Root.vue'),/ProjectSeedsWorkspace|data-query-action="seeds"|\['seeds',/);
 assert.match(source('src/features/assets/components/SeedReviewWorkbench.vue'),/hasLocalAssets\?'继续增量发现'/);
 const pageSource=source(file);assert.match(pageSource,/@media\(max-width:680px\)/);assert.match(pageSource,/overflow-wrap:anywhere/);
});

test('compact rows expose one status control and retain complete fingerprint and reason in accessible details',async t=>{
 const fingerprint='0123456789abcdef'.repeat(4),reason='Each source must independently bind this host to the legal owner';
 const item=seed(1,{kind:'fingerprint',value:fingerprint,trustReason:reason});
 const h=await mount(t,{list_project_seeds:async()=>page(7,{items:[item,seed(2,{trustReason:reason})]}),read_project_seed:async()=>detail(1,{seed:item})});
 const value=h.matching('data-seed-id',1)[0];
 assert.equal(value.props.title,fingerprint);assert.match(value.props['aria-label'],new RegExp(fingerprint));
 assert.match(h.text(),/0123456789ab…456789abcdef/);assert.equal(h.text().includes(reason),false,'Long repeated reason is not rendered in every row');
 const status=h.matching('data-seed-status',1)[0];assert.equal(status.props.title,reason);
 await h.click('data-seed-status',1);
 assert.match(h.text(),new RegExp(fingerprint));assert.match(h.text(),new RegExp(reason));
 assert.equal(h.matching('data-seed-detail',1)[0].props['aria-expanded'],true);
 assert.equal(h.calls.filter(([name])=>name==='read_project_seed').length,1);
});

test('copy uses the exact wildcard or fingerprint value and a late clipboard result cannot mark a new selection copied',async t=>{
 const writes=[],wait=deferred(),item=seed(1,{wildcard:true,value:'shared.example.com'});
 const h=await mountAssetAccuracy(file,{projectId:7},{locale:'zh',host:{navigator:{clipboard:{writeText:async value=>{writes.push(value);return wait.promise;}}}},commands:{list_project_seeds:async()=>page(7,{items:[item,seed(2)]}),read_project_seed:async({seedId})=>detail(seedId,{seed:seedId===1?item:seed(2)})}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});
 await h.click('data-seed-detail',1);const copying=h.state().copySeed();await flush();assert.deepEqual(writes,['*.shared.example.com']);
 await h.click('data-seed-detail',2);wait.resolve();await copying;await flush();
 assert.equal(h.state().selectedId.value,2);assert.equal(h.state().copiedSeedId.value,undefined);assert.doesNotMatch(h.text(),/已复制/);
 await h.click('data-seeds-action','copy-seed');assert.equal(h.state().copiedSeedId.value,2);assert.equal(writes.at(-1),'asset2.example.com');assert.match(h.text(),/已复制/);
 await h.click('data-seeds-action','close-detail');assert.equal(h.state().copiedSeedId.value,undefined);assert.equal(h.matching('aria-label','种子证据与历史').length,0);
});

test('clipboard rejection keeps the complete selectable value and reports failure without read or mutation retries',async t=>{
 const h=await mount(t);await h.click('data-seed-detail',1);const before=h.calls.length;
 await h.click('data-seeds-action','copy-seed');assert.match(h.text(),/无法访问剪贴板/);assert.match(h.text(),/asset1.example.com/);assert.equal(h.calls.length,before);
 await h.click('data-seed-detail',1);assert.match(h.text(),/无法访问剪贴板/);
});

test('detail chrome offset follows actual shell navigation height and selection does not launch a stale read after project changes',async t=>{
 const h=await mount(t);let sectionHeight=64;
 h.state().workspaceElement.value.closest=()=>({querySelector:selector=>({getBoundingClientRect:()=>({height:selector.includes('.topbar')?64:sectionHeight})})});
 h.state().updateLayout();assert.equal(h.state().stickyTop.value,140);
 sectionHeight=108;h.state().updateLayout();assert.equal(h.state().stickyTop.value,184,'Wrapped section tabs must not cover detail controls');
 const pending=h.state().selectSeed(seed(1));h.props.projectId=8;await pending;await flush();
 assert.equal(h.state().selectedId.value,undefined);assert.equal(h.calls.some(([name])=>name==='read_project_seed'),false,'Project switch during initial focus update fences the obsolete selection');
});

test('subject fields are visible with provenance, excluded rows suppress subject extraction and selection is project scoped',async t=>{
 const owner={name:'中移（上海）信息通信科技有限公司',field:'证书 Subject.Organization (O)',source:'leaf_subject'};
 const h=await mount(t,{list_project_seeds:async({query})=>page(query.projectId,{items:[seed(1,{subjects:[owner],trustStatus:'accepted'}),seed(2,{subjects:[{name:'Excluded operator',field:'证书 O'}],trustStatus:'excluded'})]})});
 assert.match(h.text(),/中移（上海）信息通信科技有限公司/);assert.match(h.text(),/Subject.Organization/);assert.doesNotMatch(h.text(),/Excluded operator/);
 await h.state().togglePage();await flush();assert.deepEqual([...h.state().checkedIds.value],[1]);
 h.props.projectId=8;await flush();assert.equal(h.state().checkedIds.value.size,0);assert.equal(h.state().allMatches.value,false);
});
test('bulk exclusion previews all matching pages, commits only after confirmation, then refreshes without restarting cleanup',async t=>{
 const h=await mountAssetAccuracy(file,{projectId:7},{locale:'zh',host:{window:{confirm:()=>true,addEventListener(){},removeEventListener(){}}},commands:{list_project_seeds:async()=>page(7,{total:100,items:[seed(1)]}),manage_project_seed_exclusion:async({input})=>input.action==='preview'?{count:100,digest:'frozen'}:{count:100,batchId:'saved'}}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});
 await h.state().togglePage();await flush();await h.click('data-seeds-action','check-all');await h.click('data-seeds-action','exclude-selected');
 const calls=h.calls.filter(([name])=>name==='manage_project_seed_exclusion');assert.equal(calls.length,2);assert.equal(calls[0][1].input.query.projectId,7);assert.equal(calls[1][1].input.expectedDigest,'frozen');assert.equal(calls[1][1].input.action,'apply');assert.match(h.text(),/已排除 100/);assert.equal(h.calls.some(([name])=>/resume|seed_cleanup_action|hunter/.test(name)),false);
});

test('selected rows expose the docked bulk region and all-match selection can be cleared without a mutation',async t=>{
 const items=[seed(1),seed(2),seed(3)];const h=await mount(t,{list_project_seeds:async()=>page(7,{total:100,items})});
 assert.equal(h.matching('role','region').length,0);
 await h.state().toggleSeed(1);await flush();
 assert.equal(h.matching('role','region').length,1);assert.equal(h.matching('data-seeds-action','exclude-selected')[0].props.disabled,false);
 await h.click('data-seeds-action','check-all');assert.equal(h.state().allMatches.value,true);assert.equal(h.state().allPageChecked.value,true);
 await h.state().togglePage();await flush();assert.equal(h.state().allMatches.value,false);assert.equal(h.state().checkedIds.value.size,0);assert.equal(h.matching('role','region').length,0);
 assert.equal(h.calls.some(([name])=>name==='manage_project_seed_exclusion'),false);
});
