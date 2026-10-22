const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const filename='src/features/agentHost/components/AgentHostChat.vue';
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'durable-chat-task'});
const flush=async()=>{for(let i=0;i<4;i++)await new Promise(setImmediate);};
const savedTask=(projectId=1)=>({id:`task-${projectId}`,revision:3,projectId,conversationId:`conversation-${projectId}`,goal:'Original durable goal',status:'interrupted',outcomeUnknown:true,steps:[{id:'write',revision:2,title:'Write once',status:'interrupted',dependsOn:[],evidenceRefs:[],verified:false}],evidence:[],artifacts:[],summary:'',createdAt:'2026-10-18T00:00:00.000Z',updatedAt:'2026-10-18T00:01:00.000Z'});
async function mount(t,overrides={}){
  let listener;const calls=[],unmount=[];
  const client={ensureProject:async(_name,id)=>id,conversations:async projectId=>[{id:`conversation-${projectId}`,projectId,title:'fixture'}],messages:async()=>[],status:async()=>({permissions:[],authorizations:[]}),task:async(_id,projectId)=>savedTask(projectId),events:callback=>{listener=callback;return()=>{};},send:async(...args)=>calls.push(['send',...args]),continueTask:async(...args)=>calls.push(['continue',...args]),...overrides};
  const exports={};new Function('require','exports',transpile(script.content))(name=>{
    if(name.endsWith('/shared/directives/refreshFeedback'))return loadPure('src/shared/directives/refreshFeedback.ts');
    if(name==='vue')return{...vue,onBeforeUnmount:callback=>unmount.push(callback)};
    if(name==='@tauri-apps/api/core')return{invoke:async(...args)=>{calls.push(['native',...args]);}};
    if(name.endsWith('ConversationHistoryList.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name==='../conversationHistory')return loadPure('src/features/agentHost/conversationHistory.ts');
    if(name==='../useConversationFiles'){
      const loaded={};new Function('require','exports',transpile(fs.readFileSync('src/features/agentHost/useConversationFiles.ts','utf8')))(dependency=>{
        if(dependency==='vue')return{...vue,onBeforeUnmount:callback=>unmount.push(callback)};
        if(dependency==='@tauri-apps/api/core')return{invoke:async(...args)=>{calls.push(['native',...args]);}};
        throw Error(`Unexpected file composable import: ${dependency}`);
      },loaded);return loaded;
    }
    if(name.endsWith('ConversationFiles.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name==='@lucide/vue')return{};
    if(name.endsWith('/i18n'))return{useI18n:()=>({tr:zh=>zh})};
    if(name.endsWith('AgentTaskCollaboration.vue'))return{default:{}};
    if(name.endsWith('AgentPermissionMode.vue')||name.endsWith('DiscoveryAssistPanel.vue')||name.endsWith('ConversationFinder.vue'))return{default:{}};
    if(name.endsWith('ConversationMessage.vue'))return{default:{}};
    if(name==='../providerPresets')return loadPure('src/features/agentHost/providerPresets.ts');
    if(name==='../errors')return loadPure('src/features/agentHost/errors.ts');
    if(name==='../chatClient')return loadPure('src/features/agentHost/chatClient.ts');
    throw Error(`Unexpected task UI import: ${name}`);
  },exports);
  const props=vue.reactive({client,settings:{runtime:{id:'xai-api'}},projectId:1,projectName:'Fixture'}),scope=vue.effectScope();
  const b=scope.run(()=>exports.default.setup(props,{expose(){},emit(){}}));t.after(()=>{unmount.forEach(fn=>fn());scope.stop();});await flush();return{b,props,calls,event:event=>listener(event)};
}

test('desktop chat restores durable task state and binds both continue and a follow-up prompt to that exact task',async t=>{
  const h=await mount(t);assert.equal(h.b.task.value.id,'task-1');assert.equal(h.b.task.value.status,'interrupted');
  await h.b.continueTask();assert.deepEqual(h.calls[0],['continue','conversation-1',1,'task-1',undefined]);
  h.b.draft.value='Check the prior write before proceeding';await h.b.send();assert.deepEqual(h.calls[1],['send','conversation-1',1,'Check the prior write before proceeding','task-1']);
});
test('late task snapshots and events cannot overwrite newer task revisions or cross a project switch',async t=>{
  const h=await mount(t);h.event({type:'task.updated',conversationId:'conversation-1',task:{...savedTask(),revision:5,summary:'latest'}});
  await h.b.loadMessages();assert.equal(h.b.task.value.revision,5);
  h.event({type:'task.updated',conversationId:'conversation-1',task:{...savedTask(),revision:2,summary:'stale'}});assert.equal(h.b.task.value.summary,'latest');
  h.props.projectId=2;h.props.projectName='Other';await flush();
  h.event({type:'task.updated',conversationId:'conversation-2',task:{...savedTask(),revision:99}});assert.equal(h.b.task.value.id,'task-2');
  await h.b.continueTask();assert.deepEqual(h.calls.at(-1),['continue','conversation-2',2,'task-2',undefined]);
});
test('task client routes carry the session, conversation, project and original task IDs',async()=>{
  const {createAgentHostChatClient}=loadPure('src/features/agentHost/chatClient.ts'),calls=[];
  const client=createAgentHostChatClient({origin:'http://127.0.0.1:9',session:'fixture-session'},async(url,init={})=>{calls.push({url,init});return{ok:true,json:async()=>({task:savedTask()})};});
  await client.task('conversation-1',1);await client.continueTask('conversation-1',1,'task-1','verify effects');await client.send('conversation-1',1,'follow-up','task-1');
  assert.equal(new URL(calls[0].url).searchParams.get('projectId'),'1');assert.equal(calls[1].init.headers['X-Oviraptor-Session'],'fixture-session');
  assert.deepEqual(JSON.parse(calls[1].init.body),{projectId:1,taskId:'task-1',text:'verify effects'});assert.equal(JSON.parse(calls[2].init.body).taskId,'task-1');
});
test('the task view compiles and labels artifacts as reported locations with separate evidence references',()=>{
  const result=compileTemplate({source:descriptor.template.content,filename,id:'durable-chat-task',compilerOptions:{bindingMetadata:script.bindings}});
  assert.deepEqual(result.errors,[]);assert.match(result.code,/agent-task-continue/);assert.match(descriptor.template.content,/Agent-reported artifact location/);assert.match(descriptor.template.content,/Plan status; unverified/);
});
test('deleting an interrupted task reports a pending-effect refusal without losing its durable state',async t=>{
  const h=await mount(t,{deleteConversation:async()=>{throw Error('host_task_effect_pending');}});
  await h.b.removeConversation();assert.equal(h.b.error.value,'host_task_effect_pending');assert.equal(h.b.task.value.id,'task-1');assert.equal(h.b.selectedId.value,'conversation-1');
});

const child=(status='running')=>({id:'child-1',runId:'child-run',parentRunId:'parent-run',kind:'research',goal:'Read once',status,provenance:'advisory-child-report',resultRef:'child-report-1'});
test('child events and even a child snapshot cannot replace the original parent task',async t=>{
 const h=await mount(t);
 h.event({type:'task.updated',conversationId:'conversation-1',task:{...savedTask(),id:'child-1',parentTaskId:'task-1',revision:99}});
 assert.equal(h.b.task.value.id,'task-1');
 h.props.client.task=async()=>({...savedTask(),id:'child-1',parentTaskId:'task-1',revision:99});
 await h.b.loadMessages();assert.equal(h.b.task.value.id,'task-1');
});
test('child cancellation binds original parent run and ignores late responses after project changes',async t=>{
 let release;const calls=[];
 const h=await mount(t,{task:async(_id,p)=>({...savedTask(p),children:[child()]}),cancelChild:async(...args)=>{calls.push(args);return new Promise(resolve=>{release=resolve;});}});
 const pending=h.b.cancelChild('child-1','parent-run');
 await h.b.cancelChild('child-1','parent-run');assert.equal(calls.length,1);
 assert.deepEqual(calls[0],['conversation-1',1,'task-1','child-1','parent-run']);
 h.props.projectId=2;await flush();release({task:{...savedTask(),revision:99,children:[child('cancelled')]}});await pending;
 assert.equal(h.b.task.value.id,'task-2');assert.deepEqual(h.b.cancellingChildren.value,[]);
});
test('cancel child HTTP client carries session and exact project, parent task, and parent run IDs',async()=>{
 const {createAgentHostChatClient}=loadPure('src/features/agentHost/chatClient.ts');let sent;
 const client=createAgentHostChatClient({origin:'http://127.0.0.1:9',session:'session'},async(url,init)=>{sent={url,init};return{ok:true,json:async()=>({task:savedTask(),childTask:{}})};});
 await client.cancelChild('conversation-1',1,'task-1','child/1','parent-run');
 assert.match(sent.url,/children\/child%2F1\/cancel$/);assert.equal(sent.init.headers['X-Oviraptor-Session'],'session');
 assert.deepEqual(JSON.parse(sent.init.body),{projectId:1,taskId:'task-1',parentRunId:'parent-run'});
});
test('child report rendering escapes model content and an expired receipt never displays current acceptance',async()=>{
 const file='src/features/agentHost/components/AgentTaskCollaboration.vue';
 const {descriptor:d}=parse(fs.readFileSync(file,'utf8'),{filename:file});
 const sc=compileScript(d,{id:'task-collaboration'}),tc=compileTemplate({source:d.template.content,filename:file,id:'task-collaboration',compilerOptions:{bindingMetadata:sc.bindings}});assert.deepEqual(tc.errors,[]);
 const module={};new Function('require','exports',transpile(sc.content))(name=>name==='vue'?vue:{useI18n:()=>({tr:(_zh,en)=>en})},module);
 const render={};new Function('require','exports',transpile(tc.code))(require,render);
 const props={cancelling:[],task:{...savedTask(),children:[{...child('reported'),report:{text:'<script>unsafe()</script>'}}],acceptanceValid:false,acceptance:{provenance:'host-finite-checks',ok:true,checks:[]}}};
 const b=module.default.setup(props,{expose(){}});
 const {renderToString}=require('@vue/server-renderer');const html=await renderToString(vue.createSSRApp({render:()=>render.render({},[],props,vue.proxyRefs(b),{}, {})}));
 assert.match(html,/&lt;script&gt;/);assert.doesNotMatch(html,/<script>/);assert.match(html,/current task needs a new check/);assert.doesNotMatch(html,/Finite checks passed for the current/);assert.match(html,/reports do not replace tool evidence/);
});
