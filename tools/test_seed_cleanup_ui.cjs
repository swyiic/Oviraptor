// Mounts the production cleanup view; fixtures validate IPC fencing, not live ownership.
const test=require('node:test');
const {assert,flush,deferred,textOf,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/SeedCleanupPanel.vue';
const state=(projectId=7,more={})=>({projectId,phase:'idle',enabled:false,organization:'测试集团',items:[],batches:[],counts:{},snapshot:{rawIndexSeeds:29977,projectLedgerSeeds:28687,groups:100},...more});
async function mount(t,command,props={}){
 const h=await mountAssetAccuracy(file,{projectId:7,...props},{locale:'zh',events:['changed'],commands:{seed_cleanup_action:command}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}
test('cleanup only reads on open and keeps raw and ledger counts separate',async t=>{
 const h=await mount(t,async()=>state());
 assert.deepEqual(h.calls.map(([,a])=>a.action),['status']);assert.match(h.text(),/29,977/);assert.match(h.text(),/28,687/);
 assert.match(h.text(),/不调用 Hunter/);assert.match(h.text(),/可撤销/);
});
test('one start freezes organization and starts one durable run; repeated click while pending cannot duplicate',async t=>{
 const pending=deferred();let starts=0,latest=state();
 const h=await mount(t,async args=>args.action==='status'?latest:(starts++,pending.promise.then(value=>(latest=value))));
 const first=h.state().control('start');await flush();await h.state().control('start');assert.equal(starts,1);
 const args=h.calls.at(-1)[1];assert.equal(args.projectId,7);assert.deepEqual(args.args,{organization:'测试集团',autoExcludeUnrelated:true});assert.ok(args.operationId.length>20);
 pending.resolve(state(7,{runId:'durable-run',phase:'researching',enabled:true}));await first;await flush();
 assert.match(h.text(),/自动分析与联网补证/);assert.equal(h.state().runId.value,'durable-run');
});
test('pause resume and batch restore bind the same project and original run without dropping source records',async t=>{
 let phase='paused';
 const h=await mount(t,async args=>{if(args.action==='resume')phase='researching';if(args.action==='pause')phase='paused';return state(7,{runId:'original-run',enabled:phase==='researching',phase,batches:[{id:'batch',restored:args.action==='restore'}],counts:{excluded:15},items:[{domain:'third-party.example',status:'excluded',reason:'domain-bound certificate and registration evidence',evidence:{sourceUrl:'https://example.test/record'}}]});});
 await h.click('data-cleanup-action','resume');await h.click('data-cleanup-action','pause');await h.click('data-cleanup-action','restore');
 assert.deepEqual(h.calls.filter(([,a])=>a.action!=='status').map(([,a])=>[a.projectId,a.action,a.args]),[[7,'resume',{runId:'original-run'}],[7,'pause',{runId:'original-run'}],[7,'restore',{runId:'original-run'}]]);
 assert.match(h.text(),/third-party.example/);assert.match(h.text(),/domain-bound/);
});
test('late old-project response cannot expose evidence or carry its run into a new project',async t=>{
 const old=deferred();const h=await mount(t,async a=>a.projectId===7?old.promise:state(8,{organization:'另一集团',runId:'new-run'}));
 h.props.projectId=8;await flush();old.resolve(state(7,{runId:'old-run',items:[{domain:'old-private.example'}]}));await flush();
 assert.equal(h.state().runId.value,'new-run');assert.equal(h.state().organization.value,'另一集团');assert.doesNotMatch(h.text(),/old-private/);
});
test('unconfirmed mutation cannot be replayed; a read error retains its prior snapshot',async t=>{
 let fail=false;const h=await mount(t,async a=>{if(a.action!=='status'||fail)throw Error('connection_lost');return state();});
 await h.click('data-cleanup-action','start');assert.equal(h.state().uncertain.value,true);await h.state().control('start');
 assert.equal(h.calls.filter(([,a])=>a.action==='start').length,1);fail=true;await h.click('data-cleanup-action','refresh');assert.match(h.text(),/29,977/);assert.match(h.text(),/connection_lost/);
});
test('cross-project payload is rejected before rendering a result',async t=>{
 const h=await mount(t,async()=>state(9,{items:[{domain:'other-project.example'}]}));assert.match(h.text(),/seed_cleanup_scope_mismatch/);assert.doesNotMatch(h.text(),/other-project/);
});

test('provider refusal remains visible with retry details and never claims an unrelated verdict',async t=>{
 const h=await mount(t,async()=>state(7,{runId:'run',phase:'waiting_evidence',enabled:false,research:{completed:0,pending:20,recentErrors:[{kind:'ct_search',target:'www.bank.example',error:'HTTP 403; retained, not proof of ownership',attempts:1,nextAt:1792890000}]}}));
 assert.match(h.text(),/HTTP 403/);assert.match(h.text(),/www.bank.example/);assert.match(h.text(),/自动重试/);assert.match(h.text(),/下次重试/);
 assert.equal(h.calls.some(([,a])=>a.action!=='status'),false);
});

test('public DNS and CT failures stay separate from applied AI decisions and real conflicts',async t=>{
 const h=await mount(t,async()=>state(7,{runId:'run',phase:'waiting_evidence',enabled:true,counts:{retained:115,excluded:31,evidence:900,held:50,conflict:2},model:{requests:81,inputTokens:20000,outputTokens:3000},hunterRequests:0,research:{completed:12,pending:40,running:1,retrying:39,recentErrors:[{kind:'dns_cname',target:'zx.cdtc.edu.cn',error:'DNS 返回截断',attempts:4,nextAt:1793070000},{kind:'ct_certificate',target:'29821641380',error:'HTTP 502',attempts:4,nextAt:1793070000}]}}));
 const decisions=h.matching('data-testid','cleanup-decisions')[0],processing=h.matching('data-testid','cleanup-processing')[0],errors=h.matching('data-testid','cleanup-public-errors')[0];
 const text=n=>n.text+n.children.map(text).join('');
 assert.match(text(decisions),/保留相关\s*115/);assert.match(text(decisions),/已排除 \/ 本轮新增\s*31 \/ 待统计/);assert.match(text(decisions),/冲突需审核\s*2/);
 assert.match(text(processing),/AI 分析模型请求 81/);assert.match(text(processing),/退避重试 39/);assert.match(text(processing),/Hunter 请求 0/);
 assert.equal(Boolean(errors.props.open),false);assert.match(text(errors),/不是 AI 判断全部失败，也不是归属冲突/);assert.match(text(errors),/不会作为排除依据/);
 assert.match(text(errors),/DNS \/ CNAME 记录/);assert.match(text(errors),/公开 CT 证书读取/);assert.match(text(errors),/下次自动重试/);
 assert.equal(h.state().filter.value,'conflict');assert.equal(h.calls.some(([,a])=>a.action!=='status'),false);
});

test('paused or exhausted public work does not claim it is automatically executing and never renders an invalid date',async t=>{
 const h=await mount(t,async()=>state(7,{runId:'run',phase:'paused',enabled:false,research:{recentErrors:[{kind:'dns_cname',target:'paused.example',error:'timeout',attempts:4,nextAt:1793070000},{kind:'ct_certificate',target:'broken-date',error:'502',attempts:4,nextAt:NaN,state:'failed'}]}}));
 assert.match(h.text(),/任务已暂停；继续后恢复补证/);assert.match(h.text(),/等待新的有效证据；不会据此排除/);
 assert.doesNotMatch(h.text(),/下次自动重试：|Invalid Date|1970/);assert.equal(h.calls.some(([,a])=>a.action!=='status'),false);
});

test('held decisions and dependent fingerprints remain visible without forcing another manual review',async t=>{
 const h=await mount(t,async a=>state(7,{runId:'run',phase:'paused',counts:{held:520},items:a.args?.filter==='held'?[{key:'fingerprint:abc',state:'held',reason:'保留已有人工决定及关联证据'}]:[]}));
 assert.match(h.text(),/保留原决定 \/ 关联待证\s*520/);
 h.state().filter.value='held';await flush();assert.equal(h.calls.at(-1)[1].args.filter,'held');assert.match(h.text(),/fingerprint:abc/);
 assert.match(h.text(),/保留已有人工决定及关联证据/);assert.equal(h.calls.some(([,a])=>a.action!=='status'),false);
});

test('one progress read failure retains the snapshot and automatically recovers without replaying a command',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});let reads=0;
 const h=await mount(t,async()=>{reads++;if(reads===2)throw Error('transient_read_error');return state(7,{runId:'run',phase:reads<3?'researching':'completed',enabled:reads<3,counts:{excluded:31}});});
 t.mock.timers.tick(2500);await flush();assert.equal(reads,2);assert.match(h.text(),/transient_read_error/);assert.match(h.text(),/29,977/);
 t.mock.timers.tick(2500);await flush();assert.equal(reads,3);assert.doesNotMatch(h.text(),/transient_read_error/);assert.match(h.text(),/本轮已完成/);
 assert.deepEqual(h.calls.map(([,a])=>a.action),['status','status','status']);
 t.mock.timers.tick(10000);await flush();assert.equal(reads,3);
});

test('provider backoff shows the effective automatic retry time and is hidden when the run pauses',async t=>{
 let enabled=true;const effective=1793000100,obsolete=1793086500;
 const h=await mount(t,async()=>state(7,{runId:'run',phase:enabled?'waiting_evidence':'paused',enabled,model:{requests:81,pendingGroups:3},research:{completed:12,pending:40,parked:500,providerWaits:[{provider:'ct',nextAt:effective,error:'502'}],recentErrors:[{kind:'ct_certificate',target:'29821641380',error:'HTTP 502',attempts:4,nextAt:effective,taskNextAt:obsolete}]}}));
 assert.match(h.text(),/可分析证据组 3/);assert.match(h.text(),/待完成步骤 40/);assert.doesNotMatch(h.text(),/待完成步骤 540/);
 assert.equal(h.matching('data-testid','cleanup-provider-waits').length,1);assert.match(h.text(),/CT · 自动恢复时间/);
 const time=value=>new Date(value*1000).toLocaleString('zh-CN',{hour12:false});
 assert.ok(h.text().includes(time(effective)));assert.ok(!h.text().includes(time(obsolete)));
 enabled=false;await h.click('data-cleanup-action','refresh');assert.equal(h.matching('data-testid','cleanup-provider-waits').length,0);assert.doesNotMatch(h.text(),/自动恢复时间/);
});

test('pausing reloads the selected conflict page instead of displaying the mutation receipt default all-results page',async t=>{
 let enabled=true;
 const h=await mount(t,async a=>{
  if(a.action==='pause'){enabled=false;return state(7,{runId:'run',phase:'paused',enabled,items:[{domain:'excluded-receipt.example',state:'excluded'}],total:10596,page:1,pageSize:1});}
  return state(7,{runId:'run',phase:enabled?'researching':'paused',enabled,items:[{domain:'actual-conflict.example',state:'conflict'}],total:1,page:1,pageSize:25});
 });
 await h.click('data-cleanup-action','pause');
 assert.deepEqual(h.calls.map(([,a])=>a.action),['status','pause','status']);
 assert.deepEqual(h.calls.at(-1)[1].args,{runId:'run',page:1,pageSize:25,filter:'conflict'});
 assert.match(h.text(),/actual-conflict.example/);assert.doesNotMatch(h.text(),/excluded-receipt.example|10,596/);
 assert.equal(h.state().data.value.total,1);assert.equal(h.state().data.value.pageSize,25);assert.equal(h.state().busy.value,false);assert.equal(h.state().reading.value,false);
});

test('a paused follow-up read failure cannot relabel excluded receipt items as conflicts and retry remains read-only',async t=>{
 let enabled=true,failed=false;
 const h=await mount(t,async a=>{
  if(a.action==='pause'){enabled=false;failed=true;return state(7,{runId:'run',phase:'paused',enabled,items:[{domain:'excluded-receipt.example',state:'excluded'}],total:10596,page:1,pageSize:1});}
  if(failed)throw Error('selected_filter_read_failed');
  return state(7,{runId:'run',phase:enabled?'researching':'paused',enabled,items:[],total:0,page:1,pageSize:25});
 });
 await h.click('data-cleanup-action','pause');
 assert.doesNotMatch(h.text(),/excluded-receipt.example|暂无需要人工决定/);assert.match(h.text(),/当前筛选结果尚未读取/);assert.match(h.text(),/selected_filter_read_failed/);
 assert.equal(h.state().data.value.items.length,0);assert.equal(h.state().resultsReady.value,false);assert.equal(h.state().busy.value,false);assert.equal(h.state().uncertain.value,false);
 failed=false;await h.click('data-cleanup-action','refresh');assert.match(h.text(),/暂无需要人工决定的证据冲突/);
 assert.deepEqual(h.calls.map(([,a])=>a.action),['status','pause','status','status']);
});

test('policy re-review shows pending local groups separately from network evidence and prepared snapshot completion',async t=>{
 const h=await mount(t,async()=>state(7,{runId:'run',phase:'researching',enabled:true,snapshot:{rawIndexSeeds:29977,projectLedgerSeeds:28687,groups:10596,processed:188016,total:188016},counts:{pending:9906,retained:115,excluded:31,evidence:490,needs_evidence:490,held:50,conflict:4},research:{completed:300,pending:40}}));
 assert.match(h.text(),/本地待复核\s*9,906/);assert.match(h.text(),/本地已复核\s*690 \/ 10,596/);
 assert.match(h.text(),/待完成步骤 40/);assert.match(h.text(),/系统继续补证\s*490/);
 assert.match(h.text(),/索引已整理不代表全部种子已完成判断/);assert.doesNotMatch(h.text(),/188,016 \/ 188,016|本轮已完成/);
 assert.equal(h.calls.some(([,a])=>a.action!=='status'),false);
});

test('cleanup reports this run exclusions separately from old rules and labels website evidence honestly',async t=>{
 const h=await mount(t,async()=>state(7,{runId:'run',phase:'researching',enabled:true,counts:{excluded:13},exclusions:{activeDomainFamilies:8,restoredDomainFamilies:2,batchCount:10},research:{website:{completed:20,pending:100,retrying:3}}}));
 assert.match(textOf(h.matching('data-testid','cleanup-decisions')[0]),/已排除 \/ 本轮新增.*13 \/ 8/);
 assert.match(textOf(h.matching('data-testid','cleanup-website-progress')[0]),/已读取 20.*待查证 100.*待重试 3/);
 assert.match(h.text(),/官网备案号不等于备案主体已核实/);
 assert.equal(h.calls.some(([,args])=>args.action!=='status'),false);
});


test('old excluded families cannot appear as newly applied exclusions and absent counts stay unknown',async t=>{
 let legacy=false;
 const h=await mount(t,async()=>state(7,{runId:'run',phase:'paused',counts:{excluded:5},...(legacy?{}:{exclusions:{activeDomainFamilies:0,restoredDomainFamilies:0,batchCount:0}})}));
 assert.match(textOf(h.matching('data-testid','cleanup-decisions')[0]),/已排除 \/ 本轮新增\s*5 \/ 0/);
 assert.equal(h.matching('data-cleanup-action','restore')[0].props.disabled,true,'Historical rules without this run batches must not enable undo');
 legacy=true;await h.click('data-cleanup-action','refresh');
 assert.match(textOf(h.matching('data-testid','cleanup-decisions')[0]),/已排除 \/ 本轮新增\s*5 \/ 待统计/);
 assert.doesNotMatch(textOf(h.matching('data-testid','cleanup-decisions')[0]),/5 \/ 0/,'An older response without receipts does not prove zero new exclusions');
 assert.equal(h.calls.some(([,args])=>args.action!=='status'),false);
});


test('automatic exclusion and restore receipts refresh the parent once while research-only polls stay local',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});
 let latest=state(7,{runId:'run',phase:'researching',enabled:true,counts:{excluded:5},exclusions:{activeDomainFamilies:0,restoredDomainFamilies:0,batchCount:0}});
 const h=await mount(t,async()=>latest);assert.deepEqual(h.events,[],'Initial receipt establishes a baseline without an extra list read');
 latest={...latest,counts:{excluded:6},exclusions:{activeDomainFamilies:1,restoredDomainFamilies:0,batchCount:1}};
 t.mock.timers.tick(2500);await flush();assert.deepEqual(h.events,[['changed']]);
 latest={...latest,model:{requests:81,pendingGroups:3},research:{completed:50,pending:100,retrying:9}};
 t.mock.timers.tick(2500);await flush();t.mock.timers.tick(2500);await flush();assert.deepEqual(h.events,[['changed']],'Repeated receipts and research progress do not re-read the seed list');
 latest={...latest,counts:{excluded:5},exclusions:{activeDomainFamilies:0,restoredDomainFamilies:1,batchCount:1}};
 t.mock.timers.tick(2500);await flush();assert.deepEqual(h.events,[['changed'],['changed']]);
 assert.equal(h.calls.some(([,args])=>args.action!=='status'),false,'Automatic parent refresh never submits a cleanup mutation');
});

test('a control receipt and its matching follow-up status emit only one parent refresh',async t=>{
 let restored=false;
 const h=await mount(t,async args=>{if(args.action==='restore')restored=true;return state(7,{runId:'run',phase:'paused',enabled:false,batches:[{id:'batch',restored}],exclusions:{activeDomainFamilies:restored?0:1,restoredDomainFamilies:restored?1:0,batchCount:1}});});
 assert.deepEqual(h.events,[]);await h.click('data-cleanup-action','restore');
 assert.deepEqual(h.events,[['changed']]);assert.deepEqual(h.calls.map(([,args])=>args.action),['status','restore','status']);
 await h.click('data-cleanup-action','refresh');assert.deepEqual(h.events,[['changed']],'Repeating the settled receipt cannot refresh again');
});

test('old-project late receipts and a new project initial receipt cannot trigger a parent refresh',async t=>{
 const pending=deferred();let delayed=false;
 const h=await mount(t,async args=>args.projectId===8?state(8,{runId:'new-run',exclusions:{activeDomainFamilies:9,restoredDomainFamilies:0,batchCount:9}}):delayed?pending.promise:state(7,{runId:'old-run',exclusions:{activeDomainFamilies:0,restoredDomainFamilies:0,batchCount:0}}));
 delayed=true;const read=h.state().read();await flush();h.props.projectId=8;await flush();
 pending.resolve(state(7,{runId:'old-run',exclusions:{activeDomainFamilies:1,restoredDomainFamilies:0,batchCount:1}}));await read;await flush();
 assert.deepEqual(h.events,[]);assert.equal(h.state().data.value.projectId,8);assert.equal(h.state().data.value.exclusions.activeDomainFamilies,9);
});

test('a background exclusion refreshes the production parent seed summary and selected details without user clicks',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});let excluded=false,listReads=0,detailReads=0;
 const item=()=>({id:1,kind:'domain',value:'third-party.example',trustStatus:excluded?'excluded':'needs_evidence',trustReason:excluded?'corroborated unrelated operator':'awaiting evidence',firstSeenRound:null,firstSeenAt:null,lastSeenAt:null,historicalBaseline:true,sourceTypes:['history'],assetCount:1,observationCount:1});
 const parent='src/features/assets/components/ProjectSeedsWorkspace.vue';
 const h=await mountAssetAccuracy(parent,{projectId:7},{locale:'zh',events:['discover'],commands:{
  list_project_seeds:async()=>{listReads++;return {projectId:7,ready:true,page:1,pageSize:50,total:1,items:[item()],summary:{total:1,verified:0,needsEvidence:excluded?0:1,conflicts:0,excluded:excluded?1:0,historicalBaseline:1},rounds:[]};},
  read_project_seed:async()=>{detailReads++;return {projectId:7,seed:item(),observations:[],observationsTotal:0,page:1,pageSize:20};},
  seed_cleanup_action:async args=>{assert.equal(args.action,'status');return state(7,{runId:'run',phase:'researching',enabled:true,counts:{excluded:excluded?1:0},exclusions:{activeDomainFamilies:excluded?1:0,restoredDomainFamilies:0,batchCount:excluded?1:0}});},
 }});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});
 await h.click('data-seed-detail',1);await h.click('data-seeds-action','cleanup');
 assert.equal(listReads,1);assert.equal(detailReads,1);assert.equal(h.state().detail.value.seed.trustStatus,'needs_evidence');
 excluded=true;t.mock.timers.tick(2500);await flush();
 assert.equal(listReads,2);assert.equal(detailReads,2);assert.equal(h.state().data.value.summary.excluded,1);assert.equal(h.state().selectedId.value,1);assert.equal(h.state().detail.value.seed.trustStatus,'excluded');
 t.mock.timers.tick(2500);await flush();assert.equal(listReads,2);assert.equal(detailReads,2,'An unchanged receipt cannot cause repeated parent reads');
});
