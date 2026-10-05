// Exercise the real SFC setup and render, including asynchronous context races.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const filename = path.join(__dirname, '../src/features/sentinel/components/AgentRequestReview.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'review-test' });
const transpile = source => ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'review-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const templateExports = {};
new Function('require', 'exports', transpile(template.code))(require, templateExports);
const render = b => renderToString(vue.createSSRApp({
  render() { return templateExports.render({}, [], {}, vue.proxyRefs(b), {}, {}); },
}));
const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {}, parentNode: () => null, nextSibling: () => null });
const flush = async () => { for (let i = 0; i < 6; i++) await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes,no) => { resolve=yes; reject=no; }); return { promise,resolve,reject }; };
const page = (scanId='A', targetUrl='http://localhost/a', attemptNumber=1) => ({
  scanId, targetUrl, attemptNumber, automaticReplayAllowed:false, executionUnlocked:false,
  unitemizedExecutorRequests:2, items:[{ requestKey:'key', snapshotHash:'hash',
    snapshot:{ source:'native_http', sourceAttempt:1, runId:'run-A', invocationId:'invocation', requestIndex:1 },
    receiptState:'headers_recorded_body_not_asserted', sourceChangedSinceReview:false, reviews:[] }],
});
const receipt = input => ({ id:input.operationId, previousReviewId:input.previousReviewId,
  snapshotHash:input.snapshotHash, snapshot:page().items[0].snapshot, disposition:input.disposition,
  note:input.note, actor:'local_operator', createdAt:'2026-01-01T00:00:00Z', reviewedAttempt:input.attemptNumber });
async function mount(overrides={}) {
  const calls=[];
  const api={ getAgentRequestReviews:async (s,a,u)=>page(s,u,a), recordAgentRequestReview:async input=>{
    calls.push(input); return receipt(input);
  }, ...overrides };
  const exports={};
  new Function('require','exports',transpile(script.content))((name)=> {
    if(name==='vue') return vue;
    if(name==='../api') return { sentinelApi:api };
    throw Error(`Unexpected import ${name}`);
  },exports);
  let b;
  const original=exports.default.setup;
  const component={ ...exports.default, setup(props,ctx) { b=original(props,ctx); return ()=>null; } };
  const props=vue.reactive({scanId:'A',attemptNumber:1,targetUrl:'http://localhost/a'});
  const app=renderer.createApp({setup:()=>()=>vue.h(component,{...props})}); app.mount({}); await flush();
  return {b,props,calls,unmount:()=>app.unmount()};
}
function fill(b) { b.choose(b.page.value.items[0]); b.note.value='核对关联日志，结果仍未知。'; b.confirmed.value=true; }

test('actual UI lists header-only uncertainty and requires explicit confirmation, with no replay',async t=>{
  const {b,calls,unmount}=await mount(); t.after(unmount);
  await b.load(); b.choose(b.page.value.items[0]); b.note.value='some evidence';
  await b.submit(); assert.equal(calls.length,0);
  b.confirmed.value=true; await b.submit(); assert.equal(calls.length,1);
  assert.equal(calls[0].snapshotHash,'hash'); assert.equal(calls[0].operatorConfirmed,true);
  const html=await render(b);
  for(const text of ['不证明响应体完整或目标效果','2 条历史执行器计数','人工核对已保存','没有发送目标请求']) assert.ok(html.includes(text),text);
});

test('ambiguous submit retries exact operation without accepting form changes or duplicate clicks',async t=>{
  const gate=deferred(); const calls=[];
  const {b,unmount}=await mount({recordAgentRequestReview:input=>{calls.push({...input}); return calls.length===1?gate.promise:Promise.resolve(receipt(input));}});
  t.after(unmount); await b.load(); fill(b); const first=b.submit(); await b.submit();
  assert.equal(calls.length,1); gate.reject(Error('lost IPC')); await first;
  assert.ok(b.pending.value); b.note.value='different'; b.disposition.value='effect_observed';
  await b.submit(); assert.equal(calls.length,2); assert.deepEqual(calls[0],calls[1]);
  assert.equal(b.pending.value,null);
});

test('late list results and errors are discarded across A-B-A target changes',async t=>{
  for(const fail of [false,true]) {
    const gate=deferred(); const {b,props,unmount}=await mount({getAgentRequestReviews:()=>gate.promise}); t.after(unmount);
    const work=b.load(); props.targetUrl='http://localhost/b'; await flush(); props.targetUrl='http://localhost/a'; await flush();
    if(fail) gate.reject(Error('old error')); else gate.resolve(page()); await work;
    assert.equal(b.page.value,null); assert.equal(b.error.value,''); assert.equal(b.loading.value,false);
  }
});

test('late save response cannot mutate the next task or initiate its refresh',async t=>{
  const gate=deferred(); let loads=0;
  const {b,props,unmount}=await mount({getAgentRequestReviews:async(s,a,u)=>{loads++;return page(s,u,a);},recordAgentRequestReview:()=>gate.promise});
  t.after(unmount); await b.load(); fill(b); const work=b.submit(); const id=b.pending.value.operationId;
  props.scanId='B'; await flush(); gate.resolve({id}); await work;
  assert.equal(b.page.value,null); assert.equal(b.notice.value,''); assert.equal(b.pending.value,null); assert.equal(loads,1);
});

test('wrong scope and unsafe backend flags cannot open an editable form',async t=>{
  for(const override of [{scanId:'B'},{targetUrl:'http://localhost/b'},{attemptNumber:2},{executionUnlocked:true},{automaticReplayAllowed:true}]) {
    const {b,unmount}=await mount({getAgentRequestReviews:async()=>({...page(),...override})}); t.after(unmount);
    await b.load(); assert.equal(b.page.value,null); assert.ok(b.error.value.includes('scope_mismatch'));
  }
});

test('append correction binds previous review and escapes human notes in actual rendering',async t=>{
  const data=page(); data.items[0].sourceChangedSinceReview=true;
  data.items[0].reviews=[{id:'previous',disposition:'still_unknown',note:'<script>bad()</script>',createdAt:'stamp',reviewedAttempt:1}];
  const {b,calls,unmount}=await mount({getAgentRequestReviews:async()=>data}); t.after(unmount); await b.load();
  const html=await render(b); assert.ok(html.includes('上次人工核对后发生变化')); assert.ok(!html.includes('<script>')); assert.ok(html.includes('&lt;script&gt;'));
  fill(b); await b.submit(); assert.equal(calls[0].previousReviewId,'previous');
});

test('refresh recovers a committed review after a lost response and clears frozen pending input',async t=>{
  let data=page();
  const {b,unmount}=await mount({getAgentRequestReviews:async()=>data,recordAgentRequestReview:async input=>{
    data=page(); data.items[0].reviews=[{id:input.operationId,disposition:input.disposition,note:input.note,createdAt:'stamp',reviewedAttempt:1}];
    throw Error('lost reply after commit');
  }}); t.after(unmount); await b.load(); fill(b); await b.submit(); assert.ok(b.pending.value);
  await b.load(); assert.equal(b.pending.value,null); assert.equal(b.page.value.items[0].reviews.length,1); assert.equal(b.selected.value,null);
});

test('same-ID mismatched receipts never display success or discard the exact retry',async t=>{
  for(const override of [
    {previousReviewId:'other'}, {snapshotHash:'other'}, {snapshot:{source:'authorization_probe'}},
    {disposition:'effect_observed'}, {note:'other'}, {actor:'agent'}, {createdAt:''},
    {createdAt:'invalid'}, {reviewedAttempt:2}, {executionUnlocked:true}, {automaticReplayAllowed:true},
  ]) {
    let loads=0; const calls=[];
    const {b,unmount}=await mount({getAgentRequestReviews:async()=>{loads++;return page();},recordAgentRequestReview:async input=>{
      calls.push({...input});return calls.length===1?{...receipt(input),...override}:receipt(input);
    }}); t.after(unmount); await b.load(); fill(b); await b.submit();
    assert.ok(b.error.value.includes('receipt_mismatch'),JSON.stringify(override));
    assert.equal(b.notice.value,''); assert.ok(b.pending.value); assert.equal(loads,1);
    assert.ok(!(await render(b)).includes('人工核对已保存'));
    await b.submit(); assert.deepEqual(calls[0],calls[1]); assert.equal(b.pending.value,null);
    assert.ok(b.notice.value.includes('人工核对已保存'));
  }
});

test('incomplete receipt is uncertain while reordered snapshot fields are equivalent',async t=>{
  let count=0;
  const {b,unmount}=await mount({recordAgentRequestReview:async input=>{
    if(++count===1) return {id:input.operationId};
    const result=receipt(input); result.snapshot=Object.fromEntries(Object.entries(result.snapshot).reverse()); return result;
  }}); t.after(unmount); await b.load(); fill(b); await b.submit();
  assert.ok(b.pending.value); assert.equal(b.notice.value,'');
  await b.submit(); assert.equal(b.pending.value,null); assert.ok(b.notice.value.includes('人工核对已保存'));
});
