import test from 'node:test';
import assert from 'node:assert/strict';
import { readHandoff } from '../src/handoff.mjs';
const envelope = payload => ({protocol:'oviraptor.handoff.v1',purpose:'draft-only',sourceAppId:'example.sender',projectId:7,payload});
test('handoff data is a separate immutable clone bound to the current Host project',()=>{
  const input=envelope({kind:'scan.targets.v1',targets:['https://example.invalid/']});
  const result=readHandoff(input,7);input.payload.targets.push('https://changed.invalid/');
  assert.deepEqual(result.payload.targets,['https://example.invalid/']);
  assert.ok(Object.isFrozen(result)&&Object.isFrozen(result.payload.targets));
  assert.throws(()=>readHandoff(input,8),{code:'app_page_handoff_invalid'});
  for(const value of [{...input,token:'forged'},{...input,purpose:'execute'},{...input,sourceAppId:''},{...input,payload:[]}])assert.throws(()=>readHandoff(value,7),{code:'app_page_handoff_invalid'});
});
test('handoff validation rejects executable, cyclic, oversized and deeply nested data without truncation',()=>{
  const cycle={};cycle.self=cycle;
  let deep={};for(let i=0;i<10;i++)deep={next:deep};
  let invoked=false;const getter=Object.defineProperty({},'value',{enumerable:true,get(){invoked=true;return 'unsafe';}});
  for(const payload of [{code:()=>{}},{value:NaN},{value:undefined},cycle,deep,getter,{value:new Date()},{items:new Array(3)},{constructor:'forged'},{value:'\u0000'},{items:Array(1025).fill(1)},{text:'界'.repeat(1366)},{items:Array(200).fill('a'.repeat(4096))}])assert.throws(()=>readHandoff(envelope(payload),7),{code:'app_page_handoff_invalid'});
  assert.equal(invoked,false);
  assert.equal(readHandoff(envelope({targets:Array(100).fill('https://example.invalid/'+ 'x'.repeat(2000))}),7).payload.targets.length,100);
});
