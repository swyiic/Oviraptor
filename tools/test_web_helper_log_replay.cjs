const assert=require('node:assert/strict'),test=require('node:test');
const {contract,row,page,mount,flush}=require('./web_helper_frontend_harness.cjs');
const web=(seq,changes={})=>row(seq,{branch:'web',stage:'node_ast:parse',message:'native_ast:parse',...changes});
test('Web helper rows retain actual Native process scope and same-text independent executions',()=>{
 const c=contract(),scope={scanId:'A',attempt:1};const records=[web(10,{streamSequence:1}),web(15,{streamSequence:1,executionId:'33333333-3333-4333-8333-333333333333',stage:'node_browser:probe'})];
 const accepted=c.acceptNativePage(page(records),scope,c.emptyNativeReplay());assert.equal(accepted.rows.length,2);assert.equal(accepted.hasGap,false);assert.equal(accepted.cursor,15);
 assert.equal(c.acceptsNativeHint({scanId:'A',attempt:1,ownerId:'a'.repeat(64),domain:'source_round',dispatchKey:'b'.repeat(64),sequence:16},scope),false);
 const gap=c.acceptNativePage(page([web(20,{streamSequence:1,stream:'gap',gap:true,message:'queue gap',executionId:'33333333-3333-4333-8333-333333333333',stage:'node_browser:probe'})],{requestedAfterSequence:15}),scope,accepted);assert.equal(gap.hasGap,true);
});
test('Actual current Native process SFC identifies two same-stage Web helper processes with committed IDs',async t=>{
 const rows=[web(10,{streamSequence:1}),web(11,{streamSequence:1,executionId:'33333333-3333-4333-8333-333333333333'})];
 const h=await mount(()=>page(rows),{view:true});t.after(h.unmount);h.live.fire();await flush();assert.match(h.text(),/22222222/);assert.match(h.text(),/33333333/);assert.match(h.text(),/node_ast:parse/);assert.equal(h.calls.length,1);
});
test('Actual readonly Web helper consumer shows gap/read failure without SDK or BrowserBroker completion',async t=>{
 let fail=false;const h=await mount(()=>{if(fail)throw Error('private-http-token');return page([web(10,{streamSequence:1,stream:'gap',gap:true,message:'record omitted whole'})]);},{view:true});t.after(h.unmount);h.live.fire();await flush();assert.match(h.text(),/日志采集存在缺口/);
 fail=true;await h.state.refresh();await flush();assert.match(h.text(),/暂时不可读取/);assert.match(h.text(),/record omitted whole/);assert.doesNotMatch(h.text(),/private-http-token|正在思考|BrowserBroker已完成|SDK 检查已通过/);
});
