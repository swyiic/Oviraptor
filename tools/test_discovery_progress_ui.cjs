const test=require('node:test');
const {assert,flush,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/DiscoveryPipelineProgress.vue';
const pipeline=extra=>({phase:'steady',round:3,collectionRunId:17,allowHunter:false,counts:{collectedAssets:98989,uniqueAssets:98989,roundAdded:4,freeCompleted:38,pendingEvidenceGroups:12000,conflicts:2},hunter:{queries:0,pages:0,spentPoints:0,costUnknown:false},reason:'完整处理日志',...extra});
async function mount(t,props){const h=await mountAssetAccuracy(file,props);t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[])});return h;}
test('compact process shows one status and bounded summary without duplicate totals or diagnostic inventory',async t=>{
 const h=await mount(t,{pipeline:pipeline({organizationEvidence:{supported:true,names:['内部单位细节'],networkVerified:true}}),compact:true});
 assert.match(h.text(),/第 3 轮/);assert.equal(h.text().split('98,989').length-1,1);
 assert.doesNotMatch(h.text(),/完整处理日志|12,000|内部单位细节|目标单位关系来源|累计唯一/);
 assert.match(h.text(),/Hunter 本次关闭/);assert.doesNotMatch(h.text(),/不消耗测绘积分/);
});
test('closing paid discovery never hides its unresolved or previously measured receipts',async t=>{
 const h=await mount(t,{pipeline:pipeline({hunter:{queries:2,pages:7,spentPoints:35,costUnknown:true}}),compact:true});
 assert.match(h.text(),/已知消耗 35 积分/);assert.match(h.text(),/2 个精确查询 · 7 页/);assert.match(h.text(),/部分扣费或请求结果未知/);assert.match(h.text(),/原收费记录保留/);
 assert.doesNotMatch(h.text(),/Hunter 本次关闭|Hunter 0 积分/);
});
test('disabled Hunter is absent from completion steps while detailed diagnostics remain reachable',async t=>{
 const h=await mount(t,{pipeline:pipeline(),compact:false});
 assert.equal(h.state().steps.value.length,4);assert.equal(h.state().steps.value.some(x=>x.key==='hunter'),false);
 assert.equal(h.state().phaseIndex.value,3);assert.match(h.text(),/完整处理日志/);assert.match(h.text(),/12,000/);
 h.props.pipeline=pipeline({allowHunter:true});await flush();assert.equal(h.state().steps.value.length,5);
});
