import test from 'node:test';
import assert from 'node:assert/strict';
import {pageScope,reviewNavigation} from '../src/navigation.js';
const page={appId:'example.sender',projectId:7,pageId:'review',version:'2.0.0',appRevision:3,grantRevision:2};
function fixture({kind='external',review=async()=>true}={}){
  const calls=[],token='a'.repeat(64),input=kind==='external'?{url:'https://example.invalid/review'}:{targetAppId:'example.receiver',targetPageId:'review',payload:{kind:'scan.targets.v1',targets:[]}};
  let current=true;
  const prepared={token,kind,projectId:7,expiresAt:Date.now()+30000,...(kind==='external'?{url:input.url}:{targetAppId:input.targetAppId,targetPageId:input.targetPageId,preview:input.payload})};
  const confirmed=kind==='external'?{kind,url:input.url}:{kind,targetAppId:input.targetAppId,targetPageId:input.targetPageId,projectId:7,deliveryToken:'b'.repeat(64)};
  const api={async post(path,body){calls.push({path,body});return path.endsWith('/confirm')?confirmed:path.endsWith('/cancel')?{cancelled:true}:prepared;}};
  return {calls,prepared,confirmed,input,api,invalidate:()=>{current=false;},run:()=>reviewNavigation({api,page,type:`navigation.${kind}`,input,isCurrent:()=>current,review})};
}
test('parent prepares and confirms navigation with its own immutable page scope',async()=>{
  const f=fixture();assert.deepEqual(await f.run(),{kind:'external',url:f.input.url});
  assert.deepEqual(f.calls.map(call=>call.path),['/api/apps/example.sender/navigation/external','/api/navigation/confirm']);
  assert.deepEqual(f.calls[0].body,{scope:pageScope(page),input:f.input});
  assert.deepEqual(f.calls[1].body,{token:f.prepared.token,scope:pageScope(page)});
});
test('cancel does not confirm, fetch an external site, or deliver handoff data',async()=>{
  const f=fixture({kind:'handoff',review:async()=>false});assert.deepEqual(await f.run(),{cancelled:true});
  assert.deepEqual(f.calls.map(call=>call.path),['/api/apps/example.sender/navigation/handoff','/api/navigation/cancel']);
});
test('revocation during each await withholds destination and cancels unconfirmed tickets',async()=>{
  for(const phase of ['prepare','review','confirm']){
    const f=fixture({review:async()=>{if(phase==='review')f.invalidate();return true;}}),post=f.api.post;
    f.api.post=async(path,body)=>{const result=await post(path,body);if(phase==='prepare'&&path.endsWith('/external')||phase==='confirm'&&path.endsWith('/confirm'))f.invalidate();return result;};
    await assert.rejects(f.run(),{code:'host_app_page_stale'});
    if(phase!=='confirm')assert.equal(f.calls.at(-1).path,'/api/navigation/cancel');
    if(phase==='prepare')assert.ok(!f.calls.some(call=>call.path.endsWith('/confirm')));
  }
});
test('confirmed application handoff retains parent-only ticket and exact target binding',async()=>{
  const f=fixture({kind:'handoff'}),result=await f.run();assert.equal(result.deliveryToken,'b'.repeat(64));
  const bad=fixture({kind:'handoff'});bad.confirmed.targetAppId='different.receiver';await assert.rejects(bad.run(),/Invalid application destination/);
  const unsafe=fixture();unsafe.confirmed.url='javascript:alert(1)';await assert.rejects(unsafe.run(),/Invalid external destination/);
});
