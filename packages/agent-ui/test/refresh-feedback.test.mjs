import test from 'node:test';
import assert from 'node:assert/strict';
import {createRefreshFeedback} from '../src/refresh-feedback.js';
import {createSkillsPanel} from '../src/panels.js';

class Element {
  constructor(tag){this.tagName=tag;this.attributes={};this.children=[];this.listeners={};this.disabled=false;this.open=false;}
  setAttribute(key,value){this.attributes[key]=String(value);if(key==='disabled')this.disabled=true;}
  addEventListener(type,handler){(this.listeners[type]??=[]).push(handler);}
  append(...children){this.children.push(...children);}
  replaceChildren(...children){this.children=[...children];}
  get textContent(){return this.children.map(c=>typeof c==='string'?c:c.textContent).join('');}
  async click(){if(this.disabled)return false;return Promise.all((this.listeners.click||[]).map(fn=>fn({target:this})));}
  showModal(){this.open=true;}
  close(){this.open=false;}
}
const document={createElement:tag=>new Element(tag),createTextNode:text=>String(text)};
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};};
const flush=async()=>{for(let i=0;i<8;i++)await Promise.resolve();};
function clock(){let time=0;const timers=new Map();let id=0;return {now:()=>time,schedule:(fn,delay)=>{timers.set(++id,{fn,at:time+delay});return id;},cancel:key=>timers.delete(key),tick(ms){time+=ms;for(const [key,value]of [...timers])if(value.at<=time){timers.delete(key);value.fn();}}};}
function dom(t){const previous=globalThis.document;globalThis.document=document;t.after(()=>{globalThis.document=previous;});}
const walk=node=>[node,...node.children.filter(c=>c instanceof Element).flatMap(walk)];
const find=(node,text)=>walk(node).find(e=>e.tagName==='button'&&e.textContent.includes(text));

test('manual reads show pending until the promise completes and reject duplicate clicks',async t=>{
 dom(t);const pending=deferred(),timer=clock();let calls=0;
 const refresh=createRefreshFeedback(()=>{calls++;return pending.promise;},{...timer,minimumMs:0});t.after(refresh.dispose);
 const button=refresh.button('Refresh applications');const first=button.click();
 assert.equal(calls,1);assert.equal(button.disabled,true);assert.equal(button.attributes['aria-busy'],'true');assert.equal(button.attributes['data-refresh-state'],'loading');assert.match(button.textContent,/Refreshing/);
 await button.click();await refresh.run();assert.equal(calls,1);
 pending.resolve(true);await first;assert.equal(button.disabled,false);assert.equal(button.attributes['data-refresh-state'],'success');assert.match(button.textContent,/Updated/);
 timer.tick(1400);assert.equal(button.attributes['data-refresh-state'],'idle');assert(!button.textContent.includes('Updated'));
});
test('returned false, caught controller errors and thrown requests never show success',async t=>{
 dom(t);for(const action of [async()=>false,async()=>{throw Error('offline');},async()=>true]){
  const timer=clock();const refresh=createRefreshFeedback(action,{...timer,minimumMs:0,failed:()=>true});const button=refresh.button('Refresh skills');
  assert.equal(await refresh.run(),false);assert.equal(button.attributes['data-refresh-state'],'error');assert.match(button.textContent,/Refresh failed/);assert(!button.textContent.includes('Updated'));assert.equal(button.disabled,false);refresh.dispose();
 }
});
test('fast requests retain an observable animation for the minimum time',async t=>{
 dom(t);const timer=clock(),refresh=createRefreshFeedback(async()=>true,{...timer,minimumMs:300});t.after(refresh.dispose);const button=refresh.button('Refresh');const request=refresh.run();await flush();
 assert.equal(button.attributes['data-refresh-state'],'loading');timer.tick(299);await flush();assert.equal(button.attributes['data-refresh-state'],'loading');timer.tick(1);await request;assert.equal(button.attributes['data-refresh-state'],'success');
});
test('a panel re-render keeps pending feedback on the replacement button',async t=>{
 dom(t);const pending=deferred(),timer=clock();let button,refresh;
 refresh=createRefreshFeedback(()=>pending.promise,{...timer,minimumMs:0,changed:()=>{button=refresh.button('Refresh skills');}});t.after(refresh.dispose);
 const original=refresh.button('Refresh skills');const request=original.click();assert.notEqual(button,original);assert.equal(button.disabled,true);assert.match(button.textContent,/Refreshing/);
 pending.resolve(true);await request;assert.equal(button.attributes['data-refresh-state'],'success');assert.match(button.textContent,/Updated/);
});
test('an old conversation read cannot display completion for the new conversation',async t=>{
 dom(t);let conversation='old';const pending=deferred(),timer=clock(),refresh=createRefreshFeedback(()=>pending.promise,{...timer,minimumMs:0,scope:()=>conversation});t.after(refresh.dispose);
 refresh.button('Reload saved selection');const request=refresh.run();conversation='new';const current=refresh.button('Reload saved selection');assert.equal(current.attributes['data-refresh-state'],'idle');pending.resolve(true);assert.equal(await request,false);assert.equal(current.attributes['data-refresh-state'],'idle');assert(!current.textContent.includes('Updated'));
});
test('actual skills panel refresh uses its promise and preserves existing read-only selection rules',async t=>{
 dom(t);const timer=clock();t.mock.method(globalThis,'setTimeout',timer.schedule);t.mock.method(globalThis,'clearTimeout',timer.cancel);const pending=deferred(),calls=[];let blocked=false;
 const api={list:async path=>{calls.push(path);if(blocked)await pending.promise;return[];},request:async()=>({revision:1,selectedSkillIds:[],runtimeIdentity:'runtime:1',skillRevision:'a'.repeat(64)}),post:()=>assert.fail('refresh must not mutate or grant')};
 const panel=createSkillsPanel(api,()=>null);panel.open();await flush();blocked=true;
 const button=find(panel.dialog,'Refresh skills');const request=button.click();await flush();const next=find(panel.dialog,'Refreshing');assert(next);assert.equal(next.disabled,true);const before=calls.length;await next.click();assert.equal(calls.length,before);
 pending.resolve();await flush();timer.tick(300);await request;assert(find(panel.dialog,'Updated'));assert.equal(panel.model.state.consent,false);assert.deepEqual([...panel.model.state.selected],[]);
});

test('background application reads disable the same refresh control without duplicate requests',async t=>{
 dom(t);let reading=false,calls=0;const timer=clock(),refresh=createRefreshFeedback(async()=>{calls++;},{...timer,minimumMs:0,disabled:()=>reading});t.after(refresh.dispose);const button=refresh.button('Refresh applications');reading=true;refresh.sync();assert.equal(button.disabled,true);await refresh.run();assert.equal(calls,0);reading=false;refresh.sync();assert.equal(button.disabled,false);await button.click();assert.equal(calls,1);
});
