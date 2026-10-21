const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/QuarantineCertificateReview.vue';
async function mount(t,commands){const h=await mountAssetAccuracy(file,{projectId:91},{commands});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[])});return h;}
const item=id=>({assetId:id,host:'https://app.example.test',link:'https://app.example.test',isDeleted:true,decision:'rejected',note:'原规则命中',contentCategory:'custom_rule',certificates:[{fingerprint:'a'.repeat(64),anchor:'app.example.test'}]});
test('local review automatically follows batches and retains classification without restoration requests',async t=>{
 const cursors=[];const h=await mount(t,{review_quarantined_certificates:async({cursor})=>{cursors.push(cursor);return {items:[item(cursor+1)],scanned:cursor?1:500,nextCursor:cursor?501:500,hasMore:!cursor,roots:['example.test']};}});
 assert.equal(h.calls.length,0);await h.state().scan();await flush();
 assert.deepEqual(cursors,[0,500]);assert.equal(h.state().items.value.length,2);assert.equal(h.state().scanned.value,501);assert.equal(h.state().complete.value,true);
 assert.match(h.text(),/不修改资产或隔离规则/);assert.match(h.text(),/原规则命中/);
 assert(h.calls.every(([name])=>name==='review_quarantined_certificates'));
});
test('a late review cannot leak another projects isolated assets or keep scanning',async t=>{
 const pending=deferred();const h=await mount(t,{review_quarantined_certificates:()=>pending.promise});
 const request=h.state().scan();h.props.projectId=92;await flush();pending.resolve({items:[item(1)],scanned:500,nextCursor:500,hasMore:true,roots:['old.example.test']});await request;
 assert.equal(h.state().items.value.length,0);assert.equal(h.state().cursor.value,0);assert.equal(h.calls.length,1);
});
