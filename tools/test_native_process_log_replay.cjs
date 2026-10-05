const assert=require('node:assert/strict'),test=require('node:test');
const {contract,row,page,flush,deferred,mount}=require('./native_process_log_harness.cjs');
const hint=(sequence=1,changes={})=>row(sequence,{message:'untrusted-notification-secret',...changes});

test('native process replay subscribes actual protocol and restores committed pages despite lost hints',async t=>{
  const h=await mount((_scan,_attempt,_cursor,after)=>after===0?page([row(1),row(2)],{more:true,latestSequence:3}):page([row(3)],{requestedAfterSequence:2,latestSequence:3}));t.after(h.unmount);
  h.live.fire();await flush();
  assert.deepEqual(h.calls,[['A',1,undefined,0,300],['A',1,1,2,300]]);
  assert.equal(h.state.replay.value.cursor,3);assert.deepEqual(h.state.replay.value.rows.map(r=>r.message),['same','same','same']);
  assert.equal(h.state.replay.value.more,false);assert.equal(h.live.listening,true);
});

test('native process replay treats duplicate hints as wakeups and never deduplicates identical row text',async t=>{
  let initial=true;
  const h=await mount((_s,_a,_c,after)=>{if(initial){initial=false;return page([row(1),row(2)]);}return page([],{requestedAfterSequence:after,afterSequence:after,latestSequence:after});});t.after(h.unmount);
  h.live.fire();await flush();for(let i=0;i<40;i++)h.live.emit(hint(2));
  assert.equal(h.live.timers,1);h.live.fire();await flush();
  assert.equal(h.state.replay.value.rows.length,2);assert.equal(new Set(h.state.replay.value.rows.map(r=>r.sequence)).size,2);
  assert.ok(!JSON.stringify(h.state.replay.value).includes('untrusted-notification-secret'));
  for(const bad of [null,{},hint(3,{scanId:'B'}),hint(3,{attempt:2}),hint(3,{dispatchClaimId:'fake'}),hint(3,{invocationKey:'url'}),hint(3,{sequence:0})])h.live.emit(bad);
  assert.equal(h.live.timers,0);
});

test('native process replay fences actual scan and attempt switches and clears stale rows immediately',async t=>{
  const late=deferred();let reads=0;
  const h=await mount((scan,attempt,_c,after)=>++reads===1?late.promise:page([row(2,{scanId:scan,attempt})],{scanId:scan,attempt,requestedAfterSequence:after}));t.after(h.unmount);
  const first=h.state.refresh();h.selection.scanId='B';h.selection.attempt=3;
  assert.equal(h.state.replay.value.rows.length,0);await h.state.refresh();
  late.resolve(page([row(1)]));await first;
  assert.deepEqual(h.state.replay.value.rows.map(r=>[r.scanId,r.attempt]),[['B',3]]);
  assert.equal(h.state.loading.value,false);
  h.selection.active=false;assert.equal(h.state.replay.value.rows.length,0);
});

test('native process replay follows latest committed attempt using server cursor reset, never event scope minting',async t=>{
  let actual=1;
  const h=await mount((_s,_a,cursor,after)=>page([row(actual,{attempt:actual,streamSequence:1,executionId:actual===1?'22222222-2222-4222-8222-222222222222':'33333333-3333-4333-8333-333333333333'})],{attempt:actual,requestedAfterSequence:after,resetCursor:cursor!==undefined&&cursor!==actual}));t.after(h.unmount);
  h.selection.attempt=0;await flush();h.live.fire();await flush();assert.equal(h.state.replay.value.attempt,1);
  actual=2;h.live.emit(hint(99,{attempt:2}));h.live.fire();await flush();
  assert.equal(h.state.replay.value.attempt,2);assert.equal(h.state.replay.value.rows.length,1);
  assert.equal(h.state.replay.value.rows[0].attempt,2);assert.deepEqual(h.calls.at(-1),['A',0,1,1,300]);
});

test('native process replay rejects wrong scope and cursor without discarding prior committed rows',async t=>{
  let corrupt=false;
  const h=await mount((_s,_a,_c,after)=>corrupt?page([row(2,{scanId:'B'})],{requestedAfterSequence:after}):page([row(1)]));t.after(h.unmount);
  h.live.fire();await flush();corrupt=true;h.live.emit(hint(2));h.live.fire();await flush();
  assert.equal(h.state.readFailed.value,true);assert.equal(h.state.replay.value.cursor,1);
  assert.equal(h.state.replay.value.rows[0].scanId,'A');
});

test('native process replay reconnects, catches visibility gap, keeps bounded view and releases late listener',async t=>{
  const options={failure:true};let n=1;
  const h=await mount((_s,_a,_c,after)=>page([row(n)],{requestedAfterSequence:after}),options);t.after(h.unmount);
  assert.equal(h.state.connectionUnavailable.value,true);options.failure=false;h.live.reconcile();await flush();h.live.fire();await flush();
  assert.equal(h.state.connectionUnavailable.value,false);assert.equal(h.state.replay.value.cursor,1);
  h.visibility(true);n=2;h.live.emit(hint(2));h.live.reconcile();h.live.fire();await flush();assert.equal(h.calls.length,1);
  h.visibility(false);h.live.fire();await flush();assert.equal(h.state.replay.value.cursor,2);
  const registration=deferred(), late=await mount(()=>page(),{registration:registration.promise});late.unmount();registration.resolve();await flush();
  assert.equal(late.live.releases,1);assert.equal(late.live.intervals,0);assert.equal(late.live.timers,0);assert.equal(late.live.observing,false);
});

test('native process contract refuses per-stream rollback, execution rebinding and inconsistent page high-water',()=>{
  const {acceptNativePage,emptyNativeReplay}=contract(),scope={scanId:'A',attempt:1};
  const old=acceptNativePage(page([row(10,{streamSequence:1})]),scope,emptyNativeReplay());
  const valid=acceptNativePage(page([row(12,{streamSequence:3,gap:true})],{requestedAfterSequence:10}),scope,old);
  assert.equal(valid.hasGap,true);assert.equal(valid.cursor,12);
  for(const corrupt of [
    page([row(11,{streamSequence:1})],{requestedAfterSequence:10}),
    page([row(11,{streamSequence:2,stage:'semgrep:step-1'})],{requestedAfterSequence:10}),
    page([row(11,{streamSequence:2})],{requestedAfterSequence:10,latestSequence:10}),
    page([],{requestedAfterSequence:10,afterSequence:10,more:true,latestSequence:12}),
    page([row(11,{streamSequence:2})],{requestedAfterSequence:10,afterSequence:99}),
  ])assert.throws(()=>acceptNativePage(corrupt,scope,old));
  // Global sequence 10→12 alone is not missing output from this stream.
  const unrelated=acceptNativePage(page([row(12,{streamSequence:2})],{requestedAfterSequence:10}),scope,old);
  assert.equal(unrelated.hasGap,false);
});

test('native process contract bounds 300 displayed rows while retaining committed continuation cursor',()=>{
  const {acceptNativePage,emptyNativeReplay}=contract(),scope={scanId:'A',attempt:1};
  let state=emptyNativeReplay();
  state=acceptNativePage(page(Array.from({length:300},(_,i)=>row(i+1)),{more:true,latestSequence:301}),scope,state);
  state=acceptNativePage(page([row(301)],{requestedAfterSequence:300}),scope,state);
  assert.equal(state.rows.length,300);assert.equal(state.rows[0].sequence,2);assert.equal(state.cursor,301);assert.equal(state.evicted,1);
});
