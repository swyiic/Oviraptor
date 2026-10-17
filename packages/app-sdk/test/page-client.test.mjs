import test from 'node:test';
import assert from 'node:assert/strict';
import { createPageClient, PAGE_PROTOCOL } from '../src/page-client.mjs';

function fixture(options = {}) {
  const listeners = new Set(), sent = [];
  const parent = { postMessage(message, origin) { sent.push({ message, origin }); } };
  const view = { parent, crypto: { randomUUID: () => 'page-operation-123456789' }, addEventListener: (_type, listener) => listeners.add(listener), removeEventListener: (_type, listener) => listeners.delete(listener) };
  const appId = 'example.native', nonce = 'a-valid-nonce-at-least-16';
  const emit = (data, extra = {}) => { for (const listener of listeners) listener({ source: parent, origin: 'http://127.0.0.1:49152', data: { protocol: PAGE_PROTOCOL, appId, nonce, ...data }, ...extra }); };
  const client = createPageClient({ appId, window: view, timeoutMs: 1000, ...options });
  return { client, sent, emit, listeners, init: extra => emit({ type: 'init', projectId: 7, appRevision: 3, grantRevision: 2, ...extra }) };
}
test('opaque page ignores foreign sources/origins and binds the actual parent context', async () => {
  const f = fixture();
  f.emit({ type: 'init', projectId: 999 }, { source: {} });
  f.emit({ type: 'init', projectId: 999 }, { origin: 'https://foreign.example' });
  f.emit({ type: 'init', projectId: 999 }, { origin: 'null' });
  assert.equal(f.client.context, undefined);
  f.init(); assert.equal((await f.client.ready).projectId, 7);
  assert.equal(f.sent[0].message.type, 'ready');
  assert.equal(f.sent[0].origin, 'http://127.0.0.1:49152');
  f.client.destroy(); assert.equal(f.listeners.size, 0);
});
test('operator action returns the raw DTO and preserves bounded CAS error codes', async () => {
  const f = fixture(); f.init();
  const call = f.client.callAction('assets_list', { page: 1 }, { operationId: 'original-operation' });
  await Promise.resolve(); const request = f.sent.at(-1).message;
  assert.equal(request.type, 'action.call'); assert.equal(request.operationId, 'original-operation');
  assert.equal(Object.hasOwn(request, 'projectId'), false);
  const dto = { items: [{ id: 2, projectId: 19 }], total: 1 };
  f.emit({ type: 'result', requestId: request.requestId, result: dto });
  assert.deepEqual(await call, dto);
  const conflict = f.client.callAction('ownership_decide', {}); await Promise.resolve();
  f.emit({ type: 'result', requestId: f.sent.at(-1).message.requestId, error: { code: 'OWNERSHIP_DECISION_PRECONDITION_FAILED' } });
  await assert.rejects(conflict, { code: 'OWNERSHIP_DECISION_PRECONDITION_FAILED' }); f.client.destroy();
});
test('stale nonce and unrelated request cannot settle a pending call', async () => {
  const f = fixture(); f.init();
  const call = f.client.callTool('history', {}); await Promise.resolve();
  const id = f.sent.at(-1).message.requestId;
  f.emit({ type: 'result', requestId: id, nonce: 'stale-nonce', result: { wrong: true } });
  f.emit({ type: 'result', requestId: id + 1, result: { wrong: true } });
  f.emit({ type: 'result', requestId: id, result: { content: [] } });
  assert.deepEqual(await call, { content: [] }); f.client.destroy();
});
test('project/nonce changes reject in-flight results instead of returning old project data', async () => {
  const f = fixture(); f.init();
  const call = f.client.callAction('assets_list', {}); await Promise.resolve();
  f.init({ projectId: 8, nonce: 'another-valid-nonce-123' });
  await assert.rejects(call, { code: 'app_page_context_changed' });
  assert.equal(f.client.context.projectId, 8); f.client.destroy();
});
test('page rejects executable/oversized inputs and never resends an uncertain operation', async () => {
  const f = fixture({ timeoutMs: 15 }); f.init();
  await assert.rejects(f.client.callAction('assets_list', { code: () => {} }), { code: 'app_page_arguments_invalid' });
  await assert.rejects(f.client.callAction('assets_list', { value: '界'.repeat(400000) }), { code: 'app_page_request_too_large' });
  const call = f.client.callAction('assets_list', {}, { operationId: 'exactly-once-intent' });
  await assert.rejects(call, { code: 'app_page_result_unknown' });
  assert.equal(f.sent.filter(item => item.message.type === 'action.call').length, 1); f.client.destroy();
});
test('page close rejects pending calls and disposes its listener', async () => {
  const f = fixture(); f.init(); const call = f.client.callAction('assets_list', {}); await Promise.resolve();
  f.client.close(); await assert.rejects(call, { code: 'app_page_closed' });
  assert.equal(f.sent.at(-1).message.type, 'close'); assert.equal(f.listeners.size, 0);
});
test('activity is a current-page refresh hint and carries no assumed business authority', async () => {
  const f = fixture(); f.init(); let observed = 0;
  const remove = f.client.subscribe(() => { observed++; });
  f.emit({ type: 'activity', nonce: 'stale' }); assert.equal(observed, 0);
  f.emit({ type: 'activity' }); assert.equal(observed, 1);
  remove(); f.emit({ type: 'activity' }); assert.equal(observed, 1); f.client.destroy();
});
test('artifact download requests remain scoped parent messages and never expose a Host credential or URL', async () => {
  const f=fixture();f.init();
  const pending=f.client.downloadArtifact('read_artifact','owned-export',{filename:'assets.csv'});await Promise.resolve();
  const message=f.sent.at(-1).message;
  assert.equal(message.type,'artifact.download');assert.deepEqual(message.arguments,{artifactId:'owned-export',filename:'assets.csv'});
  assert.equal(Object.hasOwn(message,'url'),false);assert.equal(Object.hasOwn(message,'token'),false);
  f.emit({type:'result',requestId:message.requestId,result:{downloadStarted:true,totalBytes:12}});
  assert.deepEqual(await pending,{downloadStarted:true,totalBytes:12});f.client.destroy();
});
test('navigation requests ask the parent for explicit operator review without receiving confirmation tickets',async()=>{
  const f=fixture();f.init();
  const external=f.client.openExternal('https://example.invalid/review');await Promise.resolve();
  let request=f.sent.at(-1).message;
  assert.equal(request.type,'navigation.external');assert.equal(request.name,'external');
  assert.deepEqual(request.arguments,{url:'https://example.invalid/review'});
  f.emit({type:'result',requestId:request.requestId,result:{offered:true}});
  assert.deepEqual(await external,{offered:true});
  const payload={kind:'scan.targets.v1',targets:['https://example.invalid/']};
  const transfer=f.client.handoff('example.receiver','review',payload);await Promise.resolve();
  request=f.sent.at(-1).message;
  assert.equal(request.type,'navigation.handoff');assert.equal(request.name,'handoff');
  assert.deepEqual(request.arguments,{targetAppId:'example.receiver',targetPageId:'review',payload});
  for(const field of ['scope','token','projectId','expectedGrantRevision'])assert.equal(Object.hasOwn(request,field),false);
  f.emit({type:'result',requestId:request.requestId,result:{cancelled:true}});
  assert.deepEqual(await transfer,{cancelled:true});f.client.destroy();
});
test('only a valid project-bound draft handoff can enter the initialized page context',async()=>{
  const f=fixture(),handoff={protocol:'oviraptor.handoff.v1',purpose:'draft-only',sourceAppId:'example.sender',projectId:7,payload:{kind:'scan.targets.v1',targets:['https://example.invalid/']}};
  f.init({handoff:{...handoff,projectId:8}});assert.equal(f.client.context,undefined);
  f.init({handoff});const context=await f.client.ready;
  assert.deepEqual(context.handoff,handoff);assert.ok(Object.isFrozen(context.handoff.payload));
  f.client.destroy();
});
test('navigation availability queries and change hints never become activity, permissions or automatic handoffs',async()=>{
  const f=fixture();f.init();let hints=0,activity=0;
  const remove=f.client.subscribeNavigation(()=>hints++);f.client.subscribe(()=>activity++);
  const pending=f.client.navigationCapabilities('example.receiver','review');await Promise.resolve();
  const request=f.sent.at(-1).message;
  assert.equal(request.type,'navigation.capabilities');assert.equal(request.name,'capabilities');
  assert.deepEqual(request.arguments,{targetAppId:'example.receiver',targetPageId:'review'});
  f.emit({type:'result',requestId:request.requestId,result:{externalOpen:false,handoff:false}});assert.deepEqual(await pending,{externalOpen:false,handoff:false});
  f.emit({type:'navigation.changed',nonce:'old'});assert.equal(hints,0);
  f.emit({type:'navigation.changed'});assert.equal(hints,1);assert.equal(activity,0);
  remove();f.emit({type:'navigation.changed'});assert.equal(hints,1);f.client.destroy();
});
test('handoff acknowledgement is sent only after the page can consume its successful result',async()=>{
  const f=fixture();f.init();
  const pending=f.client.handoff('example.receiver','review',{kind:'scan.targets.v1',targets:[]});await Promise.resolve();
  const request=f.sent.at(-1).message;let received=false;
  const consumed=pending.then(result=>{assert.equal(result.handoffAccepted,true);received=true;});
  f.emit({type:'result',requestId:request.requestId,result:{handoffAccepted:true,draftOnly:true}});
  await consumed;assert.equal(f.sent.some(item=>item.message.type==='navigation.received'),false);
  await new Promise(resolve=>setTimeout(resolve,0));
  const acknowledgement=f.sent.find(item=>item.message.type==='navigation.received').message;
  assert.equal(received,true);assert.equal(acknowledgement.requestId,request.requestId);assert.equal(Object.hasOwn(acknowledgement,'token'),false);f.client.destroy();
});
test('destroying or rebinding the page discards a queued navigation acknowledgement',async()=>{
  for(const rebind of [false,true]){
    const f=fixture();f.init();const pending=f.client.handoff('example.receiver','review',{});await Promise.resolve();
    f.emit({type:'result',requestId:f.sent.at(-1).message.requestId,result:{handoffAccepted:true,draftOnly:true}});await pending;
    if(rebind)f.init({projectId:8,nonce:'new-page-nonce-at-least-16'});else f.client.destroy();
    await new Promise(resolve=>setTimeout(resolve,0));assert.equal(f.sent.some(item=>item.message.type==='navigation.received'),false);f.client.destroy();
  }
});
