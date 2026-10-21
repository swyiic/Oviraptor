const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const FILE='src/features/agentHost/components/AgentPermissionMode.vue';
const view=(projectId=1,mode='ask',revision=0)=>({projectId,mode,savedMode:mode,revision,runtimeIdentity:'xai-api:1',needsReview:false,apps:{},workspacePermissions:mode==='full'?['workspace.read','workspace.write']:[]});
async function mount(t,client,props={},host={}){const h=await mountAssetAccuracy(FILE,{client,projectId:1,runtimeIdentity:'xai-api:1',...props},{events:['changed'],host});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;}
test('mode picker loads the shared project policy and saves an explicit selection exactly once',async t=>{
 const writes=[],pending=deferred();const client={accessPolicy:async p=>view(p),setAccessPolicy:async input=>{writes.push(input);return pending.promise;}};const h=await mount(t,client);
 const first=h.state().change({target:{value:'full'}});await h.state().change({target:{value:'full'}});assert.equal(writes.length,1);
 assert.deepEqual(writes[0],{projectId:1,mode:'full',expectedRevision:0,expectedRuntimeIdentity:'xai-api:1',operatorConfirmed:true});pending.resolve(view(1,'full',1));await first;assert.equal(h.state().policy.value.mode,'full');assert.match(h.text(),/Full access/);
});
test('a failed policy save leaves the displayed saved mode intact instead of claiming full access',async t=>{
 const h=await mount(t,{accessPolicy:async p=>view(p),setAccessPolicy:async()=>{throw Error('conflict');}});const target={value:'full'};await h.state().change({target});assert.equal(h.state().policy.value.mode,'ask');assert.equal(target.value,'ask');assert.match(h.text(),/not saved/);
});
test('late reads and writes cannot change another selected project',async t=>{
 const pending=deferred(),h=await mount(t,{accessPolicy:async p=>view(p),setAccessPolicy:()=>pending.promise});const save=h.state().change({target:{value:'full'}});h.props.projectId=2;await flush();pending.resolve(view(1,'full',1));await save;assert.equal(h.state().policy.value.projectId,2);assert.equal(h.state().policy.value.mode,'ask');
 const old=deferred();h.props.client={accessPolicy:p=>p===2?old.promise:Promise.resolve(view(p)),setAccessPolicy:async()=>{throw Error('unused');}};await flush();h.props.projectId=3;await flush();old.resolve(view(2,'full',8));await flush();assert.equal(h.state().policy.value.projectId,3);assert.equal(h.state().policy.value.mode,'ask');
});
test('configuration and composer instances synchronize through the same saved workspace event',async t=>{
 let saved=view();const bus=new EventTarget();const client={accessPolicy:async()=>saved,setAccessPolicy:async input=>(saved=view(input.projectId,input.mode,saved.revision+1))};
 const first=await mount(t,client,{}, {window:bus}),second=await mount(t,client,{compact:true},{window:bus});await first.state().change({target:{value:'full'}});await flush();assert.equal(second.state().policy.value.mode,'full');await second.state().change({target:{value:'ask'}});await flush();assert.equal(first.state().policy.value.mode,'ask');
});
