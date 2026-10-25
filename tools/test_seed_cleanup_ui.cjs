// Mounts the production cleanup view; fixtures validate IPC fencing, not live ownership.
const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/SeedCleanupPanel.vue';
const state=(projectId=7,more={})=>({projectId,phase:'idle',enabled:false,organization:'测试集团',items:[],batches:[],counts:{},snapshot:{rawIndexSeeds:29977,projectLedgerSeeds:28687,groups:100},...more});
async function mount(t,command,props={}){
 const h=await mountAssetAccuracy(file,{projectId:7,...props},{locale:'zh',commands:{seed_cleanup_action:command}});
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}
test('cleanup only reads on open and keeps raw and ledger counts separate',async t=>{
 const h=await mount(t,async()=>state());
 assert.deepEqual(h.calls.map(([,a])=>a.action),['status']);assert.match(h.text(),/29,977/);assert.match(h.text(),/28,687/);
 assert.match(h.text(),/不调用 Hunter/);assert.match(h.text(),/可撤销/);
});
test('one start freezes organization and starts one durable run; repeated click while pending cannot duplicate',async t=>{
 const pending=deferred();let starts=0;
 const h=await mount(t,async args=>args.action==='status'?state():(starts++,pending.promise));
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
