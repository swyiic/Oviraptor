const test=require('node:test');
const assert=require('node:assert/strict');
const {mountAssetAccuracy,flush,deferred}=require('./asset_accuracy_ui_harness.cjs');
async function mount(t,file,props,commands){const h=await mountAssetAccuracy(file,props,{commands,events:['changed']});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[])});return h;}
const path='src/features/assets/components/CertificateTaskPanel.vue';
const task=(status='paused')=>({roots:['example.test'],tasks:[{id:'task-one',status,reason:'saved',workers:4,progress:{total:8,captured:2,failed:1,skipped:0,pending:5,running:0,uniqueFingerprints:1,newFingerprints:1,attempts:3},items:[]}]});
test('pause and resume call the shared durable task and retain its identity',async t=>{const calls=[];const h=await mount(t,path,{projectId:91},{certificate_task_action:async input=>{calls.push(input);return task(input.action==='resume'?'running':'paused')}});await h.state().act('resume');assert.equal(calls.find(c=>c.action==='resume').args.taskId,'task-one');await h.state().act('pause');assert.equal(calls.find(c=>c.action==='pause').projectId,91);assert.equal(h.state().task.value.id,'task-one');});
test('late progress for a previous project cannot replace the newly selected project',async t=>{const delayed=deferred();const h=await mount(t,path,{projectId:91},{certificate_task_action:async({projectId})=>projectId===91?delayed.promise:{roots:['two.test'],tasks:[]}});h.props.projectId=92;await flush();delayed.resolve(task());await flush();assert.deepEqual(h.state().data.value.roots,['two.test']);assert.equal(h.state().task.value,undefined);});
test('archived projects cannot create or resume a certificate collection task',async t=>{const h=await mount(t,path,{projectId:91,archived:true},{certificate_task_action:async()=>task()});await h.state().act('start');await h.state().act('resume');assert(h.calls.every(([command,args])=>command==='certificate_task_action'&&args.action==='status'));});
test('current confirmed scope is separate from old task statistics and details stay collapsed',async t=>{
 const value=task('completed');value.roots=['cmsrict.com'];value.tasks[0].selection={roots:['lnhcp.com'],httpsAssets:14902,outsideConfirmedScope:14898,alreadyHaveFingerprint:4,queued:0};
 const h=await mount(t,path,{projectId:91},{certificate_task_action:async()=>value});
 assert.match(h.text(),/当前已确认范围cmsrict.com/);assert.match(h.text(),/当前查看的是旧范围的任务记录/);assert.match(h.text(),/下方统计仅属于此任务，不代表当前范围的补采进度/);assert.match(h.text(),/按当前范围检查并补采/);assert.equal(h.state().scopeChanged.value,true);assert.equal(h.state().historical.value,true);assert.equal(h.state().expanded.value,false);
});
test('refresh has real in-flight state and overlapping refreshes share one request',async t=>{
 let next=false;const pending=deferred();const h=await mount(t,path,{projectId:91},{certificate_task_action:async()=>next?pending.promise:task('completed')});next=true;
 const first=h.state().load(),second=h.state().load();assert.equal(first,second);await flush();assert.equal(h.state().reading.value,true);assert.match(h.text(),/正在读取…/);assert.equal(h.matching('data-certificate-action','refresh')[0].props.disabled,true);assert.equal(h.calls.filter(([,a])=>a.action==='status').length,2);
 pending.resolve(task('completed'));await first;await flush();assert.equal(h.state().reading.value,false);assert.equal(h.matching('data-certificate-action','refresh')[0].props.disabled,false);
});
test('failed refresh preserves the task and exposes the actual error',async t=>{
 let fail=false;const h=await mount(t,path,{projectId:91},{certificate_task_action:async()=>{if(fail)throw Error('TLS task status offline');return task('completed');}});fail=true;await h.state().load();assert.equal(h.state().reading.value,false);assert.equal(h.state().task.value.id,'task-one');assert.match(h.text(),/TLS task status offline/);assert.equal(h.matching('role','alert').length,1);
});
test('old refresh completion cannot clear a new project read or alter its progress',async t=>{
 const old=deferred(),current=deferred();const h=await mount(t,path,{projectId:91},{certificate_task_action:({projectId})=>projectId===91?old.promise:current.promise});h.props.projectId=92;await flush();assert.equal(h.state().reading.value,true);
 old.resolve(task());await flush();assert.equal(h.state().reading.value,true);assert.equal(h.state().data.value,undefined);current.resolve({roots:['current.test'],tasks:[]});await flush();assert.equal(h.state().reading.value,false);assert.deepEqual(h.state().data.value.roots,['current.test']);
});
test('changing task history collapses details and does not start a task',async t=>{
 const value=task('completed');value.tasks.unshift({...value.tasks[0],id:'task-two',createdAt:'2026-10-22 10:00:00'});const h=await mount(t,path,{projectId:91},{certificate_task_action:async()=>value});h.state().expanded.value=true;h.state().taskId.value='task-one';await flush();assert.equal(h.state().expanded.value,false);assert.equal(h.state().historical.value,true);assert.match(h.text(),/历史任务进度/);assert(h.calls.every(([,a])=>a.action==='status'));
});
test('the current-scope start keeps concurrency controls separate from historical statistics',async t=>{
 const value=task('completed');value.roots=['new.test'];value.tasks[0].selection={roots:['old.test'],httpsAssets:20,outsideConfirmedScope:10,alreadyHaveFingerprint:4,queued:6};let started;
 const h=await mount(t,path,{projectId:91},{certificate_task_action:async a=>{if(a.action==='start')started=a;return value;}});h.state().workers.value=8;await h.state().act('start');assert.deepEqual(started,{projectId:91,action:'start',args:{workers:8,timeoutSeconds:8}});assert.equal(h.state().expanded.value,false);
});
test('legacy records without selection metadata cannot look like current-scope progress or crash rendering',async t=>{
 const value=task('completed');value.tasks[0].selection={};const h=await mount(t,path,{projectId:91},{certificate_task_action:async()=>value});assert.match(h.text(),/此历史任务没有记录创建时的范围/);assert.match(h.text(),/历史任务进度/);assert.equal(h.state().historical.value,true);assert.equal(h.state().hasSelectionCounts.value,false);assert.equal(h.matching('class','certificate-task-selection').length,0);
});
test('a paused old-scope task offers a primary current-scope start without removing explicit historical resume',async t=>{
 const value=task('paused');value.roots=['new.test'];value.tasks[0].selection={roots:['old.test'],httpsAssets:20,outsideConfirmedScope:10,alreadyHaveFingerprint:4,queued:6};let action;
 const h=await mount(t,path,{projectId:91},{certificate_task_action:async a=>{if(a.action!=='status')action=a.action;return structuredClone(value);}});assert.equal(h.state().canStartCurrent.value,true);assert.equal(h.matching('data-certificate-action','start').length,1);assert.equal(h.matching('data-certificate-action','resume').length,1);assert.match(h.text(),/继续此历史任务/);await h.click('data-certificate-action','start');assert.equal(action,'start');
 value.tasks[0].selection.roots=['new.test'];await h.state().load();await flush();assert.equal(h.state().canStartCurrent.value,false);
});
