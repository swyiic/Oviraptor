const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/SeedReviewWorkbench.vue';
const item=()=>({key:'certificate:known',kind:'certificate',target:'a'.repeat(64),assetCount:2000,state:'done',advice:'review',confidence:.6,reason:'共享证书，不能只凭采集标签确认',decision:'pending',revision:1,evidence:{domains:['one.example.com'],companies:['中国移动'],subjectOrganizations:[],organizationHints:[],sources:['same_certificate_identity'],exampleAssetIds:[1,2],quarantinedAssets:5,ambiguousIdentity:true,evidenceTruncated:false}});
const view=(p=1,status='ready')=>({run:{id:'run-'+p,organization:'中国移动集团',status,totalAssets:30000,processedAssets:30000,cursor:99,maxId:99,groups:200,counts:{conflict:1,pending:199},queueCounts:{conflict:1,pending:199},manualCounts:{accepted:4,rejected:10},model:'fixture-model',requests:1,inputTokens:100,outputTokens:20,unknownRequests:0,estimatedUsage:false,estimatedBatches:17,reason:'本地已整理'},items:[item()],total:1,roots:[]});
const proposal=()=>({token:'revision-1',roots:[],totalEvidenceGroups:12477,proposedPendingFamilyReferences:2000,groups:[{root:'example.com',shared:false,scopes:['example.com'],observedDomains:['one.example.com','later.example.com'],observedCount:2,evidenceGroups:220,certificateGroups:90,pendingEvidenceGroups:200,conflictGroups:1,assetCount:3000,reasons:['单位关联证据']}]});
async function mount(t,commands={},props={projectId:1}){const h=await mountAssetAccuracy(file,props,{events:['confirmed'],commands:{seed_review_status:async()=>view(),control_seed_review:async()=>({id:'run-1'}),seed_review_scope_proposal:async()=>proposal(),confirm_seed_review_proposal:async()=>({enrichmentStarted:true}),...commands}});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;}
test('bulk AI control is distinct from scope, sends declared organization and permits a one-batch trial',async t=>{
 const h=await mount(t);await h.state().control('start',1);await flush();const call=h.calls.find(c=>c[0]==='control_seed_review');assert.equal(call[1].input.projectId,1);assert.equal(call[1].input.maxBatches,1);assert.equal(call[1].input.organization,'中国移动集团');assert.equal(h.calls.some(c=>/hunter|probe|confirm_seed/.test(c[0])),false);assert.match(h.text(),/不消耗 Hunter 积分/);assert.match(h.text(),/不是授权/);
});
test('whole-project proposal is visible above the list and explicit confirmation starts free enrichment',async t=>{
 const h=await mount(t);await h.state().selectProjectCandidates();await h.state().confirmScope();assert.equal(h.calls.some(c=>c[0]==='confirm_seed_review_proposal'),false);
 assert.match(h.text(),/12,477/);assert.match(h.text(),/90 张指纹证书/);assert.deepEqual(h.state().selectedHosts.value,['example.com']);assert(h.text().indexOf('新增范围建议')<h.text().indexOf('未绑定指纹的证书名称')||h.text().indexOf('新增范围建议')<h.text().indexOf('证书 SHA-256'));
 h.state().scopeConsent.value=true;await h.state().confirmScope();const call=h.calls.find(c=>c[0]==='confirm_seed_review_proposal');assert.deepEqual(call[1].input,{projectId:1,runId:'run-1',hosts:['example.com'],expectedRoots:[],proposalToken:'revision-1',itemKey:null,userConfirmed:true});assert.equal(h.events.filter(e=>e[0]==='confirmed').length,1);assert.match(h.state().notice.value,/已自动启动/);assert.equal(h.state().selectedHosts.value.length,0);
});
test('manual association approval does not grant scope or invoke Hunter and includes a revision witness',async t=>{
 const h=await mount(t,{decide_seed_review:async()=>{}});await h.state().decide(item(),'accepted');const call=h.calls.find(c=>c[0]==='decide_seed_review');assert.equal(call[1].input.expectedRevision,1);assert.equal(call[1].input.decision,'accepted');assert.equal(h.calls.some(c=>/confirm_seed|hunter|probe/.test(c[0])),false);
});
test('late page and control results cannot replace another project or move its scope',async t=>{
 const hold=deferred();let count=0;const h=await mount(t,{seed_review_status:async()=>++count===1?view():count===2?hold.promise:view(2)});const pending=h.state().load();h.props.projectId=2;await flush();hold.resolve(view());await pending;assert.equal(h.state().run.value.id,'run-2');assert.equal(h.state().organization.value,'中国移动集团');assert.equal(h.state().scopeConsent.value,false);
});
test('double clicks do not start a second bulk model worker and failed writes stay reviewable',async t=>{
 const hold=deferred();const h=await mount(t,{control_seed_review:()=>hold.promise});const first=h.state().control('start');await h.state().control('start');assert.equal(h.calls.filter(c=>c[0]==='control_seed_review').length,1);hold.reject(Error('synthetic provider unavailable'));await first;assert.match(h.state().error.value,/synthetic/);assert.equal(h.state().busy.value,false);
});

test('unbound certificate metadata never displays its internal grouping digest as a real SHA-256 fingerprint',async t=>{
 const v=view();v.items[0].kind='certificate_metadata';v.items[0].key='certificate_metadata:internal';v.items[0].target='f'.repeat(64);
 const h=await mount(t,{seed_review_status:async()=>v});
 assert.match(h.text(),/未绑定指纹的证书名称/);assert.match(h.text(),/one.example.com/);assert.equal(h.text().includes('f'.repeat(64)),false);
});

test('resuming a partial local scan does not claim that AI or network billing has started',async t=>{
 const v=view();v.run.cursor=50;v.run.maxId=100;v.run.status='paused';
 const h=await mount(t,{seed_review_status:async()=>v});await h.state().control('start');
 assert.match(h.state().notice.value,/继续本地整理/);assert.match(h.state().notice.value,/不消耗模型 Token/);
});

test('local scan completion is visibly distinct from completion of AI review',async t=>{
 const h=await mount(t);assert.equal(h.state().progress.value,100);assert.equal(h.state().reviewed.value,1);assert.equal(h.state().reviewProgress.value,.5);assert.match(h.text(),/本地证据整理/);assert.match(h.text(),/AI 有效审阅 1 \/ 200 组/);
});

test('only related candidates and conflicts are actionable; evidence gaps and technical failures are separate',async t=>{
 const v=view();v.run.queueCounts={retain:1,conflict:0,evidence:1975,unknown:2,pending:10485};v.run.manualCounts={accepted:4,rejected:10};
 const h=await mount(t,{seed_review_status:async()=>v});
 assert.equal(h.calls.find(c=>c[0]==='seed_review_status')[1].filter,'actionable');
 assert.match(h.text(),/证据冲突 0/);assert.match(h.text(),/待补证 1975/);assert.match(h.text(),/技术错误 2/);assert.match(h.text(),/已排除 10/);
 await h.state().control('triage');assert.equal(h.calls.find(c=>c[0]==='control_seed_review')[1].input.action,'triage');assert.match(h.state().notice.value,/没有调用模型或 Hunter/);
});

test('automatic routing is visible without inventing model analysis or manual approvals',async t=>{
 const v=view();v.run.counts={resolved:120,deferred:60,pending:20};v.run.queueCounts={resolved:120,evidence:60,pending:20};v.run.automationCounts={existing_scope:100,excluded:15,reused_rejected:5,missing_evidence:60};
 const h=await mount(t,{seed_review_status:async()=>v});
 assert.match(h.text(),/已在范围 100/);assert.match(h.text(),/按排除规则处理 15/);assert.match(h.text(),/复用人工判断 5/);assert.match(h.text(),/自动暂存缺证 60/);
 assert.equal(h.state().reviewed.value,0);assert.equal(h.state().automaticallyHandled.value,180);assert.equal(h.state().reviewProgress.value,90);
 assert.match(h.text(),/自动处理并继续 AI 分析/);assert.equal(h.calls.some(c=>/hunter|probe|confirm_seed/.test(c[0])),false);
});

test('selecting one certificate requests its family across every project page without authorizing it',async t=>{
 const h=await mount(t);await h.state().selectProjectCandidates('certificate:on-page');const call=h.calls.find(c=>c[0]==='seed_review_scope_proposal');assert.deepEqual(call[1].input,{projectId:1,runId:'run-1',itemKey:'certificate:on-page'});assert.equal(h.state().proposalKey.value,'certificate:on-page');assert.match(h.text(),/220 组证据/);assert.equal(h.calls.some(c=>/confirm_seed|enrichment_action|hunter/.test(c[0])),false);
 h.state().scopeConsent.value=true;h.state().selectedHosts.value=[];await flush();assert.equal(h.state().scopeConsent.value,false);
});
test('no candidates gives immediate visible feedback and partial start failure does not ask for scope again',async t=>{
 const h=await mount(t,{seed_review_scope_proposal:async()=>({...proposal(),groups:[]})});await h.state().selectProjectCandidates();assert.match(h.text(),/整个项目暂无/);assert.match(h.text(),/没有新增候选/);
});
test('scope saved but failed task launch is reported honestly with existing records retained',async t=>{
 const h=await mount(t,{confirm_seed_review_proposal:async()=>({enrichmentStarted:false,enrichmentError:'queue locked'})});await h.state().selectProjectCandidates();h.state().scopeConsent.value=true;await h.state().confirmScope();assert.match(h.state().notice.value,/范围已保存/);assert.match(h.state().notice.value,/queue locked/);assert.match(h.state().notice.value,/无需再次确认范围/);
});
test('late full-project proposal cannot cross workspace boundaries',async t=>{
 const hold=deferred();const h=await mount(t,{seed_review_scope_proposal:()=>hold.promise});const pending=h.state().selectProjectCandidates();h.props.projectId=2;await flush();hold.resolve(proposal());await pending;assert.equal(h.state().proposal.value,undefined);assert.deepEqual(h.state().selectedHosts.value,[]);
});
