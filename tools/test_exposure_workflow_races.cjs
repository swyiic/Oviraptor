const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const dir = path.join(__dirname, '../src/features/assets/exposure');
const modules = {};
function moduleFor(name) {
  if (modules[name]) return modules[name];
  const exports = {};
  const code = ts.transpileModule(fs.readFileSync(path.join(dir, name + '.ts'), 'utf8'), {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
  new Function('require','exports',code)(id => id === 'vue' ? vue : id === './api' ? {} : moduleFor(id.replace('./','')), exports);
  return modules[name] = exports;
}
const deferred = () => {let resolve, reject;const promise = new Promise((a,b)=>{resolve=a;reject=b});return {promise,resolve,reject}};
const flush = async () => {for(let i=0;i<12;i++)await vue.nextTick()};
const page = (items=[],over={}) => ({items,total:items.length,hasMore:false,nextCursor:'',pageSize:50,...over});
const evidence = id => ({finding:{id,category:'source_code',title:'fixture',confidence:70,status:'new'},observations:[],versions:[],reviewEvents:[],reviewNeeded:null});
function mount(t, name, api) {
 const scope=vue.reactive({projectId:7,findingId:41}),events=[];let state;
 const renderer=vue.createRenderer({createComment:()=>({}),insert(){},remove(){},parentNode:()=>null,nextSibling:()=>null});
 const app=renderer.createApp({setup(){state=moduleFor(name)[name]({projectId:()=>scope.projectId,findingId:()=>scope.findingId,active:()=>true,api,notify:(...v)=>events.push(v)});return()=>null}});
 app.mount({});t.after(()=>app.unmount());return {state,scope,events};
}
test('closing evidence cancels a pending open instead of reopening the panel',async t=>{
 const gate=deferred(),h=mount(t,'useExposureEvidence',{readExposureEvidence:()=>gate.promise});
 const read=h.state.openEvidence(41);h.state.closeEvidence();gate.resolve(evidence(41));await read;
 assert.equal(h.state.evidence.value,null);assert.equal(h.state.evidenceLoading.value,false);
});
test('refreshing the pending queue does not abandon an evidence read',async t=>{
 const gate=deferred(),h=mount(t,'useExposureEvidence',{readExposureEvidence:()=>gate.promise,pageExposurePendingDiscoveries:async()=>page()});
 const read=h.state.openEvidence(41);await h.state.loadPending();gate.resolve(evidence(41));await read;
 assert.equal(h.state.evidence.value?.finding.id,41);assert.equal(h.state.evidenceLoading.value,false);
});
test('pending pagination reuses its filter and coalesces double clicks',async t=>{
 const gate=deferred(),calls=[],h=mount(t,'useExposureEvidence',{pageExposurePendingDiscoveries:async(...a)=>{calls.push(a);return a[3]?gate.promise:page([],{hasMore:true,nextCursor:'next'})}});
 await h.state.loadPending('accepted');const a=h.state.loadMorePending(),b=h.state.loadMorePending();gate.resolve(page());await Promise.all([a,b]);
 assert.equal(calls.length,2);assert.equal(calls[1][1],'accepted');
});
test('evidence and pending state disappear immediately on project change',async t=>{
 const h=mount(t,'useExposureEvidence',{readExposureEvidence:async()=>evidence(41),pageExposurePendingDiscoveries:async()=>page()});
 await h.state.openEvidence(41);await h.state.loadPending();h.scope.projectId=8;await flush();
 assert.equal(h.state.evidence.value,null);assert.equal(h.state.pending.value,null);
});
test('reopening a different finding does not display the previous evidence while loading',async t=>{
 const gate=deferred(),h=mount(t,'useExposureEvidence',{readExposureEvidence:async id=>id===41?evidence(id):gate.promise});
 await h.state.openEvidence(41);const read=h.state.openEvidence(42);assert.equal(h.state.evidence.value,null);gate.resolve(evidence(42));await read;
});
test('a late resume receipt cannot notify the next project',async t=>{
 const gate=deferred(),h=mount(t,'useExposureEvidence',{resumeExposureScan:()=>gate.promise});
 const a=h.state.resumeRun(1),b=h.state.resumeRun(1);h.scope.projectId=8;await flush();gate.resolve(1);await Promise.all([a,b]);
 assert.equal(h.events.length,0);
});
test('review drafts and actionable snapshots reset when the selected finding changes',async t=>{
 const h=mount(t,'useExposureReview',{readExposureEvidence:async id=>evidence(id),reviewExposureFindingScoped:async()=>1});
 await h.state.load();h.state.reason.value='old reason';h.state.remediationNote.value='old fix';h.state.actor.value='reviewer';h.scope.findingId=42;await flush();
 assert.equal(h.state.loaded.value,false);assert.equal(h.state.evidence.value,null);assert.equal(h.state.reason.value,'');assert.equal(h.state.remediationNote.value,'');assert.equal(h.state.actor.value,'reviewer');
});
test('a review cannot be submitted against an unbound snapshot',async t=>{
 let writes=0;const h=mount(t,'useExposureReview',{reviewExposureFindingScoped:async()=>{writes++;return 1}});
 h.state.actor.value='reviewer';h.state.reason.value='reason';assert.equal(await h.state.review('confirmed'),false);assert.equal(writes,0);
});
test('late review errors cannot change the next finding or notify it',async t=>{
 const gate=deferred(),h=mount(t,'useExposureReview',{readExposureEvidence:async id=>evidence(id),reviewExposureFindingScoped:()=>gate.promise});
 await h.state.load();h.state.actor.value='reviewer';h.state.reason.value='reason';const write=h.state.review('confirmed');h.scope.findingId=42;await flush();gate.reject(Error('old failure'));await write;
 assert.equal(h.state.error.value,'');assert.equal(h.events.length,0);
});
test('a previous review read cannot clear a newer read loading state',async t=>{
 const first=deferred(),second=deferred(),h=mount(t,'useExposureReview',{readExposureEvidence:id=>id===41?first.promise:second.promise,reviewExposureFindingScoped:async()=>1});
 const a=h.state.load();h.scope.findingId=42;await flush();const b=h.state.load();first.resolve(evidence(41));await a;assert.equal(h.state.busy.value,true);second.resolve(evidence(42));await b;assert.equal(h.state.busy.value,false);
});
test('review history uses the finding-scoped native page instead of walking the project',async t=>{
 const calls=[],h=mount(t,'useExposureReview',{readExposureEvidence:async id=>evidence(id),reviewExposureFindingScoped:async()=>1,
 pageExposureFindingRemediations:async(...args)=>{calls.push(args);return page()},pageExposureRemediations:async()=>{throw Error('must not scan project')}});
 assert.equal(await h.state.load(),true);assert.deepEqual(calls,[[7,41,50,'']]);
});
const incident = (id, projectId=7) => ({id,projectId,revision:1,status:'candidate',title:'fixture',activeLocations:1,units:{unknown:true},handlingState:'none',locations:page()});
test('switching incidents immediately hides the old actionable detail',async t=>{
 const gate=deferred(),h=mount(t,'useExposureIncidents',{readExposureIncident:async(_p,id)=>id===1?incident(1):gate.promise});
 await h.state.open(1);const read=h.state.open(2);assert.equal(h.state.detail.value,null);gate.resolve(incident(2));await read;
});
test('an old incident action cannot reopen the old incident after selecting another',async t=>{
 const gate=deferred(),h=mount(t,'useExposureIncidents',{readExposureIncident:async(_p,id)=>incident(id),actOnExposureIncident:()=>gate.promise,pageExposureIncidents:async()=>page()});
 await h.state.open(1);const write=h.state.act({action:'review',status:'reviewing',actor:'alice',reason:'confirmed test reason'});await h.state.open(2);gate.resolve({});await write;
 assert.equal(h.state.detail.value?.id,2);
});
test('incident merge target reads discard slower previous targets',async t=>{
 const a=deferred(),b=deferred(),h=mount(t,'useExposureIncidents',{readExposureIncident:(_p,id)=>id===1?Promise.resolve(incident(1)):id===2?a.promise:b.promise});
 await h.state.open(1);const x=h.state.readMergeTarget(2),y=h.state.readMergeTarget(3);b.resolve(incident(3));await y;a.resolve(incident(2));await x;assert.equal(h.state.mergeTarget.value?.id,3);
});
test('historic incident proposals coalesce double clicks and do not notify the next project',async t=>{
 const gate=deferred();let calls=0;const h=mount(t,'useExposureIncidents',{proposeExposureIncidents:()=>{calls++;return gate.promise},pageExposureIncidents:async()=>page()});
 const a=h.state.proposeHistory(),b=h.state.proposeHistory();h.scope.projectId=8;await flush();gate.resolve({});await Promise.all([a,b]);assert.equal(calls,1);assert.equal(h.events.length,0);
});
test('changing cluster filters clears obsolete rows before the next response',async t=>{
 const gate=deferred();let calls=0;const h=mount(t,'useExposureClusters',{pageExposureClusters:async()=>++calls===1?page([{id:1}]):gate.promise});
 await h.state.load();h.state.searchFilter.value='new';const read=h.state.load();assert.deepEqual(h.state.clusters.value,[]);gate.resolve(page());await read;
});
