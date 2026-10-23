const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const file='src/features/assets/components/ContinuousDiscovery.vue';
const view=(status=null)=>({roots:['example.com'],run:status?{id:'saved-id',status,enabled:status==='running',continuous:true,intervalSeconds:3600,workers:4,counts:{done:4,pending:6,retry:2},operations:6,reason:'saved'}:null,events:[],conflicts:[]});
async function mount(t,command=async()=>view(),props={}){const h=await mountAssetAccuracy(file,{projectId:1,...props},{events:['changed'],commands:{enrichment_action:command}});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;}
test('one shared queue replaces per-asset clicking without granting scope or calling Hunter',async t=>{const h=await mount(t,async a=>a.action==='start'?view('running'):view());await h.state().control('start');const call=h.calls.find(c=>c[1].action==='start');assert.deepEqual(call[1],{projectId:1,action:'start',args:{continuous:true,intervalSeconds:3600,workers:4}});assert.match(h.text(),/Hunter 0 积分/);assert.match(h.text(),/错误待重试 2/);assert.equal(h.calls.some(c=>c[0]!=='enrichment_action'),false);});
test('double-click is suppressed, paused run resumes the same ID and conflict acknowledgement grants nothing',async t=>{const hold=deferred();const h=await mount(t,async a=>a.action==='resume'?hold.promise:view('paused'));const pending=h.state().control('resume');await h.state().control('resume');assert.equal(h.calls.filter(c=>c[1].action==='resume').length,1);hold.resolve(view('running'));await pending;assert.deepEqual(h.calls.find(c=>c[1].action==='resume')[1].args,{taskId:'saved-id'});await h.state().control('review_conflict',{kind:'certificate_hostname_mismatch',identity:'a',revision:4});assert.deepEqual(h.calls.at(-1)[1].args,{kind:'certificate_hostname_mismatch',identity:'a',expectedRevision:4,decision:'acknowledged'});});
test('a late project response cannot leak old progress or keep the new project busy',async t=>{const hold=deferred();const h=await mount(t,async a=>a.action==='start'?hold.promise:view());const pending=h.state().control('start');h.props.projectId=2;await flush();hold.resolve(view('running'));await pending;await flush();assert.equal(h.state().data.value.run,null);assert.equal(h.state().busy.value,false);});
test('terminal errors are distinct from durable backoff and never labelled saved or retrying',async t=>{const value=view('completed');value.run.counts={done:7,retry:0,failed:2};value.events=[{kind:'tls',target:'fixture.example.test',outcome:'failed',detail:'non_public_address',at:1}];const h=await mount(t,async()=>value);assert.match(h.text(),/失败待处理 2/);assert.match(h.text(),/永久错误或已达到重试上限/);assert(!h.text().includes('TLS 证书 · 已保存'));});
for(const fails of [false,true])test(`a pre-start status ${fails?'failure':'snapshot'} cannot override the admitted running task`,async t=>{
 let read;const h=await mount(t,async a=>a.action==='start'?view('running'):read?read.promise:view('completed'));read=deferred();const pending=h.state().load();await h.state().control('start');
 if(fails)read.reject(Error('obsolete status error'));else read.resolve(view('completed'));await pending;await flush();assert.equal(h.state().run.value.status,'running');assert.equal(h.state().error.value,'');assert.equal(h.state().readError.value,'');
});
test('concurrent refreshes share one read and cannot issue status requests during a mutation',async t=>{
 let read,action;const h=await mount(t,async a=>a.action==='start'?action.promise:read?read.promise:view());read=deferred();const before=h.calls.length,first=h.state().load(),second=h.state().load();assert.equal(h.calls.length,before+1);read.resolve(view());await Promise.all([first,second]);
 action=deferred();const starting=h.state().control('start'),count=h.calls.length;await h.state().load();assert.equal(h.calls.length,count);action.resolve(view('running'));await starting;
});
test('storage contention remains active and polling recovers without a manual resume',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});let value=view('waiting_storage');value.run.enabled=true;const h=await mount(t,async()=>value);assert.equal(h.state().active.value,true);assert.match(h.text(),/等待本地存储/);assert(!h.text().includes('从断点继续'));
 value=view('running');t.mock.timers.tick(2500);await flush();assert.equal(h.state().run.value.status,'running');assert.equal(h.calls.every(c=>c[1].action==='status'),true);
});
test('archived projects remain readable without admitting supplement or conflict actions',async t=>{
 const h=await mount(t,async()=>view(),{archived:true});await h.state().control('start');await h.state().control('review_conflict',{kind:'x',identity:'y',revision:1});assert.equal(h.calls.every(c=>c[1].action==='status'),true);assert.match(h.text(),/已归档/);
});
test('a successful reconciliation read retains the failed action reason without replaying that action',async t=>{
 const h=await mount(t,async a=>{if(a.action==='start')throw Error('control delivery failed');return view('paused');});await h.state().control('start');await flush();assert.match(h.state().error.value,/control delivery failed/);assert.equal(h.state().readError.value,'');assert.equal(h.state().run.value.status,'paused');assert.equal(h.calls.filter(c=>c[1].action==='start').length,1);
});
