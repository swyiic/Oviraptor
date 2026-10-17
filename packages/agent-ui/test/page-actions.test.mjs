import test from 'node:test';
import assert from 'node:assert/strict';
import { bridgeMessage, BRIDGE_PROTOCOL, NATIVE_BRIDGE_PROTOCOL, eventInvalidatesPage } from '../src/security.js';
const source = {}, context = { source, appId: 'example.native', nonce: 'only-this-page', protocol: NATIVE_BRIDGE_PROTOCOL };
const request = () => ({ protocol: NATIVE_BRIDGE_PROTOCOL, appId: context.appId, nonce: context.nonce, type: 'action.call', requestId: 1, name: 'assets_list', arguments: { query: { page: 1 } }, operationId: 'original-operation' });
const event = data => ({ source, origin: 'null', data });
test('registry changes invalidate exactly the affected native page before a refresh or late result',()=>{
  const page={protocol:NATIVE_BRIDGE_PROTOCOL,appId:'example.native',projectId:7};
  assert.equal(eventInvalidatesPage({type:'applications.changed',appId:page.appId,projectId:7},page),true);
  assert.equal(eventInvalidatesPage({type:'applications.changed',appId:page.appId},page),true);
  for(const input of [{type:'applications.changed',appId:'other.app',projectId:7},{type:'applications.changed',appId:page.appId,projectId:8},{type:'applications.changed',projectId:7},{type:'application.activity',appId:page.appId}])assert.equal(eventInvalidatesPage(input,page),false);
  assert.equal(eventInvalidatesPage({type:'applications.changed',appId:page.appId},{...page,protocol:BRIDGE_PROTOCOL}),false);
});
test('native page bridge admits only a bounded declared-method request shape', () => {
  const input = request(), result = bridgeMessage(event(input), context);
  assert.deepEqual(result, { type: 'action.call', requestId: 1, name: 'assets_list', arguments: input.arguments, operationId: 'original-operation' });
  for (const field of ['projectId', 'token', 'url', 'command', 'permission', 'expectedGrantRevision']) assert.equal(bridgeMessage(event({ ...request(), [field]: 'forged' }), context), null);
});
test('native page bridge rejects stale identity and unsafe request values', () => {
  for (const override of [{ nonce: 'old' }, { appId: 'another.app' }, { requestId: -1 }, { requestId: Number.MAX_SAFE_INTEGER + 1 }, { name: '../arbitrary' }, { operationId: '' }, { arguments: [] }, { arguments: { callback() {} } }, { arguments: { value: Number.NaN } }]) assert.equal(bridgeMessage(event({ ...request(), ...override }), context), null);
  assert.equal(bridgeMessage({ ...event(request()), source: {} }, context), null);
  assert.equal(bridgeMessage({ ...event(request()), origin: 'http://127.0.0.1' }, context), null);
  assert.equal(bridgeMessage(event({ ...request(), arguments: { value: '界'.repeat(400000) } }), context), null);
});
test('version1 pages retain their lifecycle-only bridge and cannot gain native action access', () => {
  assert.equal(bridgeMessage(event({ ...request(), protocol: BRIDGE_PROTOCOL }), { ...context, protocol: BRIDGE_PROTOCOL }), null);
  assert.deepEqual(bridgeMessage(event({ protocol: BRIDGE_PROTOCOL, appId: context.appId, nonce: context.nonce, type: 'ready' }), { ...context, protocol: BRIDGE_PROTOCOL }), { type: 'ready' });
});
test('artifact download is a native scoped method request and not an arbitrary URL or file proxy',()=>{
  const message={...request(),type:'artifact.download',name:'read_artifact',arguments:{artifactId:'owned-export',filename:'assets.csv'}};
  assert.equal(bridgeMessage(event(message),context).type,'artifact.download');
  assert.equal(bridgeMessage(event({...message,url:'file:///private'}),context),null);
  assert.equal(bridgeMessage(event({...message,protocol:BRIDGE_PROTOCOL}),{...context,protocol:BRIDGE_PROTOCOL}),null);
});
test('navigation bridge admits only two native review requests and no source authority or ticket',()=>{
  const external={...request(),type:'navigation.external',name:'external',arguments:{url:'https://example.invalid'}};
  assert.equal(bridgeMessage(event(external),context).type,'navigation.external');
  assert.equal(bridgeMessage(event({...external,name:'run_arbitrary'}),context),null);
  assert.equal(bridgeMessage(event({...external,arguments:{url:'https://example.invalid',token:'forged'}}),context),null);
  assert.equal(bridgeMessage(event({...external,projectId:7}),context),null);
  const transfer={...request(),type:'navigation.handoff',name:'handoff',arguments:{targetAppId:'example.receiver',targetPageId:'review',payload:{kind:'scan.targets.v1',targets:[]}}};
  assert.equal(bridgeMessage(event(transfer),context).type,'navigation.handoff');
  assert.equal(bridgeMessage(event({...transfer,arguments:{...transfer.arguments,scope:{projectId:7}}}),context),null);
  assert.equal(bridgeMessage(event({...transfer,protocol:BRIDGE_PROTOCOL}),{...context,protocol:BRIDGE_PROTOCOL}),null);
});
test('navigation availability requests cannot select the source scope or a different project',()=>{
  const message={...request(),type:'navigation.capabilities',name:'capabilities',arguments:{targetAppId:'example.receiver',targetPageId:'review'}};
  assert.equal(bridgeMessage(event(message),context).type,'navigation.capabilities');
  for(const arguments_ of [{...message.arguments,projectId:8},{...message.arguments,permission:'ui.handoff'},{...message.arguments,token:'forged'}])assert.equal(bridgeMessage(event({...message,arguments:arguments_}),context),null);
});
test('handoff acknowledgement is bound to the current native frame and exact request identity',()=>{
  const message={protocol:NATIVE_BRIDGE_PROTOCOL,appId:context.appId,nonce:context.nonce,type:'navigation.received',requestId:4};
  assert.deepEqual(bridgeMessage(event(message),context),{type:'navigation.received',requestId:4});
  for(const extra of [{token:'forged'},{targetAppId:'example.receiver'},{requestId:0},{nonce:'old'}])assert.equal(bridgeMessage(event({...message,...extra}),context),null);
  assert.equal(bridgeMessage(event({...message,protocol:BRIDGE_PROTOCOL}),{...context,protocol:BRIDGE_PROTOCOL}),null);
});
