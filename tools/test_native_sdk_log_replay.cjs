const assert=require('node:assert/strict'),test=require('node:test');
const {contract,owner,row,page,deferred,mount,flush}=require('./native_sdk_log_harness.cjs');
const hint=(sequence=1,changes={})=>({scanId:'A',attempt:1,ownerId:'a'.repeat(64),domain:'root',dispatchKey:'b'.repeat(64),runId:'root-A',workerId:null,sequence,...changes});
const sent=(sequence=2,changes={})=>row(sequence,{ordinal:2,stage:'sent',...changes});
test('SDK consumer uses actual lifecycle and replays committed pages despite lost hints',async t=>{
 const h=await mount((_s,_a,_c,after)=>after===0?page([row(1)],{more:true,latestSequence:2}):page([sent(2)],{requestedAfterSequence:1}));t.after(h.unmount);h.live.fire();await flush();
 assert.deepEqual(h.calls,[['A',1,undefined,0,300,undefined],['A',1,1,1,300,undefined]]);assert.equal(h.state.replay.value.cursor,2);assert.equal(h.live.listening,true);
});
test('SDK hints only wake the dedicated reader; duplicate hints never become model content',async t=>{
 let initial=true;const h=await mount((_s,_a,_c,after)=>initial?(initial=false,page([row(1)])):page([],{requestedAfterSequence:after,afterSequence:after,latestSequence:after}));t.after(h.unmount);h.live.fire();await flush();
 for(let n=0;n<40;n++)h.live.emit(hint(1,{prompt:'private-event-body',message:'never-a-row'}));assert.equal(h.live.timers,1);h.live.fire();await flush();assert.equal(h.state.replay.value.rows.length,1);assert.ok(!JSON.stringify(h.state.replay.value).includes('private-event-body'));
 for(const bad of [null,{},hint(2,{scanId:'B'}),hint(2,{attempt:2}),hint(2,{ownerId:'invalid'}),hint(2,{workerId:'forged'}),hint(2,{domain:'process',dispatchClaimId:'fake'}),hint(0)])h.live.emit(bad);assert.equal(h.live.timers,0);
});
test('SDK scan/attempt/owner switches fence late actual responses and clear prior state',async t=>{
 const late=deferred();let reads=0;const h=await mount((scan,attempt,_c,after,_l,selectedOwner)=>++reads===1?late.promise:page([row(2,{scanId:scan,attempt,...(selectedOwner?{ownerId:selectedOwner}:{})})],{scanId:scan,attempt,requestedAfterSequence:after,ownerId:selectedOwner??null,owners:[owner({scanId:scan,attempt,...(selectedOwner?{ownerId:selectedOwner}:{})})],incompleteOwners:[selectedOwner??'a'.repeat(64)]}));t.after(h.unmount);
 const first=h.state.refresh();h.selection.scanId='B';h.selection.attempt=3;h.selection.ownerId='d'.repeat(64);assert.equal(h.state.replay.value.rows.length,0);await h.state.refresh();late.resolve(page([row(1)]));await first;
 assert.equal(h.state.replay.value.rows[0].scanId,'B');assert.equal(h.state.replay.value.rows[0].ownerId,'d'.repeat(64));assert.equal(h.state.loading.value,false);h.selection.active=false;assert.equal(h.state.replay.value.rows.length,0);
});
test('SDK latest attempt resets only from authoritative page, never the event owner',async t=>{
 let actual=1;const h=await mount((_s,_a,cursor,after)=>page([row(actual,{attempt:actual})],{attempt:actual,requestedAfterSequence:after,resetCursor:cursor!==undefined&&cursor!==actual,owners:[owner({attempt:actual})]}));t.after(h.unmount);
 h.selection.attempt=0;await flush();h.live.fire();await flush();actual=2;h.live.emit(hint(20,{attempt:2}));h.live.fire();await flush();assert.equal(h.state.replay.value.attempt,2);assert.equal(h.state.replay.value.rows.length,1);assert.deepEqual(h.calls.at(-1),['A',0,1,1,300,undefined]);
});
test('SDK malformed owner/page preserves last committed prefix and exposes read failure',async t=>{
 let corrupt=false;const h=await mount((_s,_a,_c,after)=>corrupt?page([sent(2,{workerId:'forged'})],{requestedAfterSequence:after}):page([row(1)]));t.after(h.unmount);h.live.fire();await flush();corrupt=true;h.live.emit(hint(2));h.live.fire();await flush();
 assert.equal(h.state.readFailed.value,true);assert.equal(h.state.replay.value.cursor,1);assert.equal(h.state.replay.value.rows[0].stage,'prepared');
});
test('SDK subscription failure reconnects, visibility suppresses reads, recovery catches missed gaps',async t=>{
 const options={failure:true};let next=false;const h=await mount((_s,_a,_c,after)=>next?page([sent(2)],{requestedAfterSequence:after,gaps:[{ownerId:'a'.repeat(64),afterOrdinal:2,failedStage:'response_received',code:'stage_write_failed',time:'saved'}]}):page([row(1)]),options);t.after(h.unmount);
 assert.equal(h.state.connectionUnavailable.value,true);options.failure=false;h.live.reconcile();await flush();h.live.fire();await flush();assert.equal(h.state.connectionUnavailable.value,false);
 h.visibility(true);next=true;h.live.emit(hint(2));h.live.reconcile();h.live.fire();await flush();assert.equal(h.calls.length,1);h.visibility(false);h.live.fire();await flush();assert.equal(h.state.replay.value.hasGap,true);assert.equal(h.state.replay.value.cursor,2);
});
test('SDK late listener and unmounted read release original lifecycle without duplicate timers',async()=>{
 const registration=deferred(),late=await mount(()=>page(),{registration:registration.promise});late.unmount();registration.resolve();await flush();assert.equal(late.live.releases,1);assert.equal(late.live.intervals,0);assert.equal(late.live.timers,0);assert.equal(late.live.observing,false);
 const response=deferred(),h=await mount(()=>response.promise);const loading=h.state.refresh();h.unmount();response.resolve(page([row(1)]));await loading;assert.equal(h.state.replay.value.rows.length,0);
});
test('SDK scope rejects process pages, owner rebinding, false terminal and future cursor',()=>{
 const {acceptSdkPage,emptySdkReplay}=contract(),scope={scanId:'A',attempt:1},old=acceptSdkPage(page([row(1)]),scope,emptySdkReplay());
 for(const bad of [page([sent(2)],{channel:'native_process',requestedAfterSequence:1}),page([sent(2,{requestHash:'d'.repeat(64)})],{requestedAfterSequence:1}),page([row(2,{ordinal:2,stage:'validated'})],{requestedAfterSequence:1}),page([row(2,{ordinal:2,stage:'terminal',costPhase:'received',terminalState:'returned'})],{requestedAfterSequence:1}),page([sent(2)],{requestedAfterSequence:1,afterSequence:9}),page([],{requestedAfterSequence:1,more:true,afterSequence:1,latestSequence:2}),page([sent(2)],{requestedAfterSequence:1,ownerId:'d'.repeat(64)})])assert.throws(()=>acceptSdkPage(bad,scope,old));
});
test('SDK explicit gaps and unfinished identities do not fabricate missing stages',()=>{
 const {acceptSdkPage,emptySdkReplay}=contract();const state=acceptSdkPage(page([row(1)],{gaps:[{ownerId:'a'.repeat(64),afterOrdinal:1,failedStage:'sent',code:'stage_write_failed',time:'saved'}]}),{scanId:'A',attempt:1},emptySdkReplay());
 assert.equal(state.hasGap,true);assert.deepEqual(state.rows.map(r=>r.stage),['prepared']);assert.deepEqual(state.incompleteOwners,['a'.repeat(64)]);assert.ok(!state.rows.some(r=>r.stage==='terminal'));
 const none=acceptSdkPage(page([],{available:false,owners:[],incompleteOwners:[]}),{scanId:'A',attempt:1},emptySdkReplay());assert.equal(none.available,false);
});
test('SDK global sequence skips from unrelated owners are not durable stage loss',()=>{
 const {acceptSdkPage,emptySdkReplay}=contract(),scope={scanId:'A',attempt:1},old=acceptSdkPage(page([row(10)]),scope,emptySdkReplay());
 const next=acceptSdkPage(page([sent(20)],{requestedAfterSequence:10}),scope,old);assert.equal(next.hasGap,false);assert.equal(next.cursor,20);
});
test('SDK view bounds 300 rows and retains committed cursor; extra model body is never retained',()=>{
 const {acceptSdkPage,emptySdkReplay}=contract(),scope={scanId:'A',attempt:1};let state=emptySdkReplay();
 for(let n=0;n<301;n++){const oid=n.toString(16).padStart(64,'0');const o=owner({ownerId:oid,dispatchKey:(n+1).toString(16).padStart(64,'0'),runId:`root-${n}`,rootRunId:`root-${n}`});state=acceptSdkPage(page([row(n+1,{...o,prompt:'private-ipc-body',response:'private-response-body'})],{requestedAfterSequence:n,owners:[o],incompleteOwners:[oid]}),scope,state);}
 assert.equal(state.rows.length,300);assert.equal(state.cursor,301);assert.equal(state.evicted,1);assert.ok(!JSON.stringify(state).includes('private-ipc-body'));assert.ok(!JSON.stringify(state).includes('private-response-body'));
});

test('SDK actual SFC consumes committed gap and unfinished prefix without invented thinking or task completion',async t=>{
 const h=await mount(()=>page([row(1)],{gaps:[{ownerId:'a'.repeat(64),afterOrdinal:1,failedStage:'sent',code:'stage_write_failed',time:'saved'}]}),{view:true});t.after(h.unmount);h.live.fire();await flush();
 const text=h.text();assert.match(text,/原调用已准备/);assert.match(text,/采集存在缺口/);assert.match(text,/尚无完整诊断终结记录/);assert.match(text,/SDK 终结不代表智能体/);assert.doesNotMatch(text,/SDK 检查已通过|原结果已返回|正在思考|private-ipc-body/);assert.equal(h.calls.length,1);
});
test('SDK actual SFC failed read and unavailable owner remain unknown with last committed stage',async t=>{
 let failure=false;const h=await mount(()=>{if(failure)throw Error('provider-private-body');return page([row(1)]);},{view:true});t.after(h.unmount);h.live.fire();await flush();failure=true;await h.state.refresh();await flush();
 assert.match(h.text(),/记录无法核验/);assert.match(h.text(),/原调用已准备/);assert.doesNotMatch(h.text(),/provider-private-body|原结果已返回/);
 const empty=await mount(()=>page([],{available:false,owners:[],incompleteOwners:[]}),{view:true});t.after(empty.unmount);empty.live.fire();await flush();assert.match(empty.text(),/不能推断是否已经调用模型/);assert.doesNotMatch(empty.text(),/SDK 检查已通过|原结果已返回/);
});

// The original production scheduler uses a deterministic logical assignment;
// only its separately issued physical worker and lease attempt are UUIDs.
test('SDK child replay accepts original asg identity and rejects foreign logical assignments',()=>{
 const {acceptSdkPage,emptySdkReplay}=contract(),scope={scanId:'A',attempt:1};
 for(const domain of ['specialist','source_round']){
  const original=owner({domain,runId:'run-asg-'+ 'd'.repeat(24),assignmentId:'asg-'+ 'd'.repeat(24),workerId:'22222222-2222-4222-8222-222222222222'});
  const actual=page([row(1,original)],{owners:[original]});
  assert.equal(acceptSdkPage(actual,scope,emptySdkReplay()).rows[0].assignmentId,original.assignmentId);
  for(const assignmentId of ['11111111-1111-4111-8111-111111111111','asg-'+ 'd'.repeat(23),'asg-'+ 'g'.repeat(24),'foreign-assignment']){
   const wrong={...original,assignmentId};
   assert.throws(()=>acceptSdkPage(page([row(1,wrong)],{owners:[wrong]}),scope,emptySdkReplay()));
  }
 }
});
