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
