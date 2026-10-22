const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/SeedReviewWorkbench.vue';
const verification=(host,kind='tls_subject')=>({status:'bound_local_evidence',sources:[{kind,host,organization:'目标集团独立单位',assetId:31}]});
const item=()=>({key:'certificate:conflict',kind:'certificate',target:'a'.repeat(64),assetCount:20,state:'done',advice:'conflict',confidence:.6,reason:'证书单位与主机的备案证据不一致',decision:'pending',revision:3,evidence:{domains:['one.example.com'],companies:[],subjectOrganizations:['外部单位'],organizationHints:[],icpOrganizations:['目标单位'],sources:['same_certificate_identity'],exampleAssetIds:[1,2],quarantinedAssets:0,ambiguousIdentity:true,evidenceTruncated:false}});
const review=(p=1)=>({run:{id:'run-'+p,organization:'中国移动集团及其下属单位',status:'completed',totalAssets:98989,processedAssets:98989,cursor:433397,maxId:433397,groups:12478,counts:{retain:200,evidence:11000,conflict:1},queueCounts:{retain:200,evidence:11000,conflict:1,pending:0},manualCounts:{accepted:115,rejected:22},model:'fixture-model',requests:230,inputTokens:500000,outputTokens:160000,unknownRequests:0,reason:'旧证据处理完成'},items:[item()],total:1,roots:['known.example']});
const proposal=(token='revision-1')=>({token,roots:['known.example'],totalEvidenceGroups:12478,networkVerified:false,groups:[{root:'example.com',shared:false,scopes:['example.com'],observedDomains:['one.example.com'],observedCount:1,evidenceGroups:200,certificateGroups:40,pendingEvidenceGroups:100,conflictGroups:1,assetCount:3000,reasons:['证书及备案主体相符'],verification:verification('one.example.com')},{root:'shared.example',shared:true,scopes:Array.from({length:111},(_,i)=>`tenant${i}.shared.example`),observedDomains:['tenant0.shared.example'],observedCount:111,evidenceGroups:292,certificateGroups:2,pendingEvidenceGroups:181,conflictGroups:0,assetCount:9551,reasons:['仅限实际观察的具体租户主机'],verification:verification('tenant0.shared.example','icp')}]});
const assist=(p=1,phase='idle')=>({phase,enabled:false,reason:'',runId:'run-'+p,hasLocalAssets:true,review:review(p),proposal:null});
const enrichment=()=>({roots:['known.example'],run:null,conflicts:[],hunterRequests:0});
async function mount(t,commands={},props={projectId:1}){const h=await mountAssetAccuracy(file,props,{events:['confirmed','collect','results'],commands:{seed_review_assist_status:async()=>assist(props.projectId),enrichment_action:async()=>enrichment(),...commands}});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[])});return h;}
const actions=h=>h.matching('data-ai-action').map(n=>n.props['data-ai-action']);
test('one main action replaces manual prepare, model trial and scope-summary controls',async t=>{
 const h=await mount(t);assert(actions(h).includes('start'));assert(!actions(h).includes('confirm'));assert.match(h.text(),/AI 发现资产/);assert.match(h.text(),/要查找的单位或业务/);assert(!h.text().includes('11000'));assert(!h.text().includes('先试审一批'));assert(!h.text().includes('汇总整个项目'));assert(!h.text().includes('重新整理全量证据'));assert.equal(h.calls.some(c=>/control_seed|scope_proposal|hunter/.test(c[0])),false);assert.equal(h.matching('class','evidence-records').length,0);
});
test('one click authorizes durable orchestration; polling never clicks through old stages',async t=>{
 let value=assist();const h=await mount(t,{seed_review_assist_status:async()=>value,control_seed_assist:async args=>{assert.equal(args.input.action,'start');value={...assist(),phase:'preparing',enabled:true};return value;}});
 await h.click('data-ai-action','start');assert.equal(h.calls.filter(c=>c[0]==='control_seed_assist').length,1);assert.deepEqual(h.calls.find(c=>c[0]==='control_seed_assist')[1].input,{projectId:1,action:'start',organization:'中国移动集团及其下属单位',includeIsolated:true});
 assert(actions(h).includes('pause'));assert(!actions(h).includes('start'));
 value={...value,phase:'analyzing'};await h.state().load();value={...value,phase:'awaiting_scope',proposal:proposal()};await h.state().load();assert(actions(h).includes('confirm'));assert.equal(h.calls.filter(c=>c[0]==='control_seed_assist').length,1);assert.equal(h.calls.some(c=>['control_seed_review','seed_review_scope_proposal','start_hunter_expansion'].includes(c[0])),false);
});
test('a new empty project offers collection before requesting AI or creating authorization',async t=>{
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'empty',hasLocalAssets:false,review:{run:null,items:[],total:0,roots:[]}})});
 assert.deepEqual(actions(h),['collect']);await h.click('data-ai-action','collect');assert.equal(h.events.filter(e=>e[0]==='collect').length,1);assert.equal(h.calls.some(c=>/control_seed|confirm_seed|hunter/.test(c[0])),false);
});
test('scope selection uses domain families instead of one checkbox per certificate or tenant',async t=>{
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'awaiting_scope',proposal:proposal()})});
 assert.deepEqual(h.state().selectedFamilies.value,['example.com','shared.example']);assert.equal(h.state().selectedHosts.value.length,112);assert.match(h.text(),/已选 2 \/ 2 组/);assert.match(h.text(),/共享平台，仅 111 个具体主机/);assert.equal(h.nodes().filter(n=>n.type==='input'&&n.props.type==='checkbox'&&['example.com','shared.example'].includes(n.props.value)).length,2);assert.equal(h.nodes().filter(n=>n.type==='input'&&n.props.type==='checkbox'&&String(n.props.value).startsWith('tenant')).length,0);
 await h.state().confirmScope();assert.equal(h.calls.some(c=>c[0]==='confirm_seed_review_proposal'),false);
});
test('scope suggestions expose actual local binding sources while keeping AI inference distinct',async t=>{
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'awaiting_scope',proposal:proposal()})});
 assert.match(h.text(),/已核验的范围建议/);assert.match(h.text(),/本次仅核验本地证据，未联网查证企业归属/);
 assert.match(h.text(),/TLS 证书 Subject · one.example.com · 目标集团独立单位/);assert.match(h.text(),/备案单位 · tenant0.shared.example/);assert.match(h.text(),/资产 #31/);assert.match(h.text(),/AI 推断（不作核验依据）/);
});
test('legacy AI advice without domain-bound evidence is neither selected nor confirmable',async t=>{
 const p=proposal();delete p.groups[0].verification;delete p.groups[1].verification;
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'awaiting_scope',proposal:p})});
 assert.deepEqual(h.state().selectedFamilies.value,[]);assert.deepEqual(h.state().selectedHosts.value,[]);assert(!actions(h).includes('confirm'));assert.match(h.text(),/2 个域名家族缺少逐域绑定依据/);assert.match(h.text(),/无需逐项确认/);
 h.state().selectedFamilies.value=['example.com','shared.example'];h.state().scopeConsent.value=true;await h.state().confirmScope();assert.equal(h.calls.some(c=>c[0]==='confirm_seed_review_proposal'),false);
});
test('blocked third-party suggestions stay out of one consent without adding individual review work',async t=>{
 const p=proposal();p.groups.push({...p.groups[0],root:'grcbank.com',scopes:['grcbank.com'],verification:undefined,reasons:['含10086.cn']});p.heldForVerification=105;p.removedUnverified=157;
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'awaiting_scope',proposal:p})});
 assert.deepEqual(h.state().selectedFamilies.value,['example.com','shared.example']);assert.equal(h.text().includes('grcbank.com'),false);assert.equal(h.text().includes('含10086.cn'),false);assert.match(h.text(),/105 个域名家族/);assert.match(h.text(),/拦截 157 项/);
 h.state().scopeConsent.value=true;await h.state().confirmScope();const call=h.calls.find(c=>c[0]==='confirm_seed_review_proposal');assert.equal(call[1].input.hosts.includes('grcbank.com'),false);
});
test('paused owners keep empty-proposal verification diagnostics visible without enabling scope confirmation',async t=>{
 const p=proposal();p.groups=[];p.heldForVerification=109;p.removedUnverified=191;
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'paused',proposal:p})});
 assert.match(h.text(),/已暂停/);assert.match(h.text(),/109 个域名家族缺少逐域绑定依据/);assert.match(h.text(),/拦截 191 项/);assert.match(h.text(),/本次仅核验本地证据，未联网查证企业归属/);
 assert.equal(h.state().visibleProposal.value,undefined);assert.equal(h.state().selectedHosts.value.length,0);assert(!actions(h).includes('confirm'));
 h.state().selectedFamilies.value=['example.com'];h.state().scopeConsent.value=true;await h.state().confirmScope();assert.equal(h.calls.some(c=>c[0]==='confirm_seed_review_proposal'),false);
});
test('one consent confirms all selected families and resumes the owner without a paid Hunter request',async t=>{
 let value={...assist(),phase:'awaiting_scope',proposal:proposal()};const h=await mount(t,{seed_review_assist_status:async()=>value,confirm_seed_review_proposal:async()=>{value={...assist(),phase:'ready'};return{enrichmentStarted:true};}});
 h.state().scopeConsent.value=true;await flush();await h.click('data-ai-action','confirm');const call=h.calls.find(c=>c[0]==='confirm_seed_review_proposal');assert.equal(call[1].input.hosts.length,112);assert.equal(call[1].input.userConfirmed,true);assert.equal(call[1].input.itemKey,null);assert.equal(call[1].input.proposalToken,'revision-1');assert.deepEqual(call[1].input.expectedRoots,['known.example']);assert.equal(h.events.filter(e=>e[0]==='confirmed').length,1);assert.match(h.text(),/范围已确认，AI 正在自动补证/);assert.equal(h.calls.some(c=>c[0]==='start_hunter_expansion'),false);
});
test('saved scope and failed launch are distinguished without demanding repeated scope consent',async t=>{
 let value={...assist(),phase:'awaiting_scope',proposal:proposal()};const h=await mount(t,{seed_review_assist_status:async()=>value,confirm_seed_review_proposal:async()=>{value={...assist(),phase:'ready'};return{enrichmentStarted:false,enrichmentError:'queue unavailable'};}});h.state().scopeConsent.value=true;await h.state().confirmScope();assert.match(h.text(),/范围已保存/);assert.match(h.text(),/无需重复确认范围/);assert.match(h.text(),/queue unavailable/);assert.equal(h.state().scopeConsent.value,false);
});
test('new evidence retains excluded families, invalidates old consent and does not silently reselect them',async t=>{
 let value={...assist(),phase:'awaiting_scope',proposal:proposal()};const h=await mount(t,{seed_review_assist_status:async()=>value});h.state().selectedFamilies.value=['example.com'];await flush();h.state().scopeConsent.value=true;
 value={...value,proposal:proposal('revision-2')};await h.state().load();assert.deepEqual(h.state().selectedFamilies.value,['example.com']);assert.equal(h.state().scopeConsent.value,false);assert.equal(h.state().selectedHosts.value.includes('tenant0.shared.example'),false);assert.match(h.text(),/已保留你取消的范围/);
});
test('an active supplement task or pausing owner cannot expose a duplicate Start button',async t=>{
 const e=enrichment();e.run={id:'free-1',status:'running',enabled:true,counts:{done:38,retry:8},reason:'后台处理'};
 const h=await mount(t,{enrichment_action:async()=>structuredClone(e)});assert(actions(h).includes('pause'));assert(!actions(h).includes('start'));assert.match(h.text(),/已完成 38/);assert.match(h.text(),/8 项未成功/);
 e.run.status='pausing';e.run.enabled=false;await h.state().load();assert(!actions(h).includes('start'));assert.match(h.text(),/正在保存进度/);
});
test('only genuine review and TLS conflicts appear in the human queue; failed requests stay automatic',async t=>{
 const e=enrichment();e.conflicts=[{kind:'certificate_hostname_mismatch',identity:'leaf-sha',hosts:['one.example.com'],detail:'叶证书不覆盖请求主机',decision:'pending',revision:5}];const h=await mount(t,{enrichment_action:async args=>{if(args.action==='review_conflict'){assert.deepEqual(args.args,{kind:'certificate_hostname_mismatch',identity:'leaf-sha',expectedRevision:5,decision:'acknowledged'});e.conflicts[0].decision='acknowledged';}return structuredClone(e);}});
 assert.equal(h.state().conflictCount.value,2);assert.equal(h.text().includes('外部单位'),false);h.state().showConflicts.value=true;await flush();assert.match(h.text(),/证书与主机身份不一致/);assert.match(h.text(),/不需要逐条点击/);
 await h.state().acknowledgeCertificate(e.conflicts[0]);assert.equal(h.state().conflictCount.value,1);assert.match(h.text(),/不会据此扩大范围/);
});
test('recovered reads clear obsolete failures while old results remain available',async t=>{
 let fail=true;const h=await mount(t,{seed_review_assist_status:async()=>{if(fail)throw Error('temporary offline');return assist();}});assert.match(h.text(),/temporary offline/);fail=false;await h.state().load();assert.equal(h.state().readError.value,'');assert.equal(h.text().includes('temporary offline'),false);
});
test('late task and proposal receipts cannot authorize a different project',async t=>{
 const hold=deferred();const h=await mount(t,{seed_review_assist_status:args=>args.projectId===1?hold.promise:Promise.resolve({...assist(2),phase:'awaiting_scope',proposal:proposal('project-2')})});h.props.projectId=2;await flush();hold.resolve({...assist(1),phase:'awaiting_scope',proposal:proposal('project-1')});await flush();assert.equal(h.state().run.value.id,'run-2');assert.equal(h.state().proposal.value.token,'project-2');assert.equal(h.state().scopeConsent.value,false);
});
test('double clicks cannot dispatch a second AI owner and pause keeps progress',async t=>{
 const hold=deferred();const h=await mount(t,{control_seed_assist:()=>hold.promise});const first=h.state().control('start');await h.state().control('start');assert.equal(h.calls.filter(c=>c[0]==='control_seed_assist').length,1);hold.resolve({...assist(),phase:'preparing',enabled:true});await first;assert.equal(h.state().busy.value,false);
});
test('resume keeps original isolation option and AI progress is visible during analysis',async t=>{
 const value={...assist(),phase:'analyzing',enabled:true};value.review.run.includeIsolated=false;value.review.run.queueCounts.pending=20;value.review.run.queueCounts.analyzing=12;
 const h=await mount(t,{seed_review_assist_status:async()=>value});assert.equal(h.state().includeIsolated.value,false);assert.match(h.text(),/已自动处理 12,446 \/ 12,478 组证据/);assert.match(h.text(),/待分析 32 组/);
 value.phase='paused';value.enabled=false;await h.state().load();
 h.state().includeIsolated.value=false;assert.equal(h.state().includeIsolated.value,false);
});
test('a blocked model can retry while free certificate discovery continues',async t=>{
 const e=enrichment();e.run={id:'free',status:'running',enabled:true,counts:{done:8},reason:''};let received;
 const h=await mount(t,{seed_review_assist_status:async()=>({...assist(),phase:'blocked',reason:'模型暂时离线'}),enrichment_action:async()=>structuredClone(e),control_seed_assist:async a=>{received=a;return assist();}});
 assert(actions(h).includes('start'));assert(actions(h).includes('pause'));assert.match(h.text(),/重试 AI 分析/);await h.click('data-ai-action','start');assert.equal(received.input.action,'start');
});
test('complete evidence records stay bound to the active assistant review',async t=>{
 let received;const h=await mount(t,{seed_review_status:async a=>{received=a;return review();}});h.state().showEvidence.value=true;await flush();assert.equal(received.runId,'run-1');
});
test('free enrichment exposes exhausted errors separately from retries and successful proofs',async t=>{const e=enrichment();e.run={id:'free',status:'completed',enabled:false,counts:{done:7,retry:0,failed:8},reason:'有失败项'};const h=await mount(t,{enrichment_action:async()=>e});assert.match(h.text(),/已完成 7 项/);assert.match(h.text(),/8 项永久错误或已达到重试上限/);assert(!h.text().includes('系统保留并重试'));});
