const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const filename='src/features/agentHost/components/AgentHostChat.vue';
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'chat-scope'});
const deferred=()=>{let resolve;const promise=new Promise(done=>{resolve=done;});return{promise,resolve};};
const flush=async()=>{for(let i=0;i<4;i++)await new Promise(setImmediate);};

async function mount(t,overrides={}) {
  let listener;
  const unmount=[];const calls=[];
  const client={
    ensureProject:async(_name,id)=>id,
    conversations:async projectId=>[{id:`conversation-${projectId}`,projectId,title:'fixture'}],
    messages:async()=>[], status:async()=>({permissions:[],authorizations:[]}),
    createConversation:async projectId=>({id:`new-${projectId}`,projectId,title:'fixture'}),
    send:async(...args)=>{calls.push(['send',...args]);},
    cancel:async()=>{},listApps:async()=>({items:[]}),
    resolveAuthorization:async(...args)=>{calls.push(['authorization',...args]);},
    decidePermission:async(...args)=>{calls.push(['runtime-permission',...args]);},
    events:callback=>{listener=callback;return()=>{};},...overrides,
  };
  const exports={};
  new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return{...vue,onBeforeUnmount:callback=>unmount.push(callback)};
    if(name==='@lucide/vue')return{};
    if(name.endsWith('/i18n'))return{useI18n:()=>({tr:(zh)=>zh})};
    if(name.endsWith('AgentTaskCollaboration.vue'))return{default:{}};
    if(name.endsWith('ConversationMessage.vue'))return{default:{}};
    if(name==='../providerPresets')return loadPure('src/features/agentHost/providerPresets.ts');
    if(name==='../errors')return loadPure('src/features/agentHost/errors.ts');
    if(name==='../chatClient')return loadPure('src/features/agentHost/chatClient.ts');
    throw Error(`Unexpected chat import: ${name}`);
  },exports);
  const props=vue.reactive({client,settings:{runtime:{id:'xai-api'}},projectId:1,projectName:'A'});
  const scope=vue.effectScope();
  const b=scope.run(()=>exports.default.setup(props,{expose(){},emit(){}}));
  t.after(()=>{unmount.forEach(callback=>callback());scope.stop();});
  await flush();
  return{b,props,calls,event:event=>listener(event)};
}

test('late project load cannot overwrite the current conversation or send into another project',async t=>{
  const pending=deferred();
  const h=await mount(t,{ensureProject:async(_name,id)=>id===1?pending.promise:id});
  h.props.projectId=2;h.props.projectName='B';await flush();
  pending.resolve(1);await flush();
  assert.equal(h.b.hostProjectId.value,2);assert.equal(h.b.selectedId.value,'conversation-2');
  h.b.draft.value='intended for B';await h.b.send();
  assert.deepEqual(h.calls.find(call=>call[0]==='send'),['send','conversation-2',2,'intended for B']);
});
test('grant decisions bind the original request and do not erase a following queued permission',async t=>{
  const h=await mount(t);
  const offer={appId:'workspace',revision:0,permissions:[{permission:'workspace.read',tools:['workspace_read']}]};
  const pending={type:'authorization.requested',conversationId:'conversation-1',projectId:1,identity:'xai-api:1',requestId:'read',runId:'run-1',offers:[offer]};
  h.event(pending);
  h.props.client.resolveAuthorization=async(...args)=>{h.calls.push(['authorization',...args]);h.event({...pending,requestId:'write',offers:[{...offer,permissions:[{permission:'workspace.write',tools:['workspace_write']}]}]});};
  await h.b.acceptGrant();
  assert.deepEqual(h.calls[0],['authorization','conversation-1',true,{requestId:'read',runId:'run-1',remember:true}]);
  assert.equal(h.b.grantPrompt.value.requestId,'write');
  h.b.selectedId.value='conversation-2';
  assert.equal(h.b.grantPrompt.value,null);
});
test('runtime approvals and snapshot restoration use exact permission IDs',async t=>{
  const h=await mount(t);
  h.event({type:'host.snapshot',authorizations:[],permissions:[{id:'runtime-7',conversationId:'conversation-1',description:'fixture write',options:[{id:'reject_once',label:'Reject'}]}]});
  assert.equal(h.b.runtimePermission.value.id,'runtime-7');
  await h.b.decidePermission('reject_once');
  assert.deepEqual(h.calls[0],['runtime-permission','runtime-7','reject_once']);
  assert.equal(h.b.runtimePermission.value,null);
});
test('an approval cancelled while apps load cannot persist any grants',async t=>{
  const pending=deferred();let writes=0;
  const h=await mount(t,{listApps:()=>pending.promise,setGrants:async()=>{writes++;}});
  h.event({type:'authorization.requested',conversationId:'conversation-1',projectId:1,identity:'xai-api:1',requestId:'expired',runId:'run-1',offers:[{appId:'fixture-app',revision:0,permissions:[{permission:'inventory.write',tools:['inventory_import']}]}]});
  const accepting=h.b.acceptGrant();
  h.event({type:'authorization.cancelled',conversationId:'conversation-1',requestId:'expired',runId:'run-1'});
  pending.resolve({items:[]});await accepting;
  assert.equal(writes,0);assert.equal(h.calls.length,0);
});
test('IME selection Enter leaves the draft intact while plain Enter sends once',async t=>{
  const h=await mount(t);h.b.draft.value='中文输入';
  let prevented=0;
  const key={key:'Enter',shiftKey:false,ctrlKey:false,altKey:false,metaKey:false,isComposing:true,keyCode:229,preventDefault(){prevented++;}};
  h.b.onComposerKeydown(key);await flush();
  assert.equal(h.calls.length,0);assert.equal(h.b.draft.value,'中文输入');assert.equal(prevented,0);
  h.b.onComposerKeydown({...key,isComposing:false,keyCode:13});await flush();
  assert.equal(h.calls.filter(call=>call[0]==='send').length,1);assert.equal(prevented,1);
});
test('the live chat template compiles with shared safe Markdown and connection status',()=>{
  const result=compileTemplate({source:descriptor.template.content,filename,id:'chat-scope',compilerOptions:{bindingMetadata:script.bindings}});
  assert.deepEqual(result.errors,[]);
  assert.match(result.code,/ConversationMessage/);
});

test('chat history remains readable when the task endpoint fails and background load errors are handled',async t=>{
  const rows=[{id:'saved',role:'assistant',content:'Saved history',status:'completed'}];
  const h=await mount(t,{messages:async()=>rows,task:async()=>{throw Error('task_unavailable');}});
  assert.equal(h.b.messages.value[0].content,'Saved history');
  h.props.client.messages=async()=>{throw Error('history_unavailable');};
  await h.b.loadMessages();
  assert.equal(h.b.error.value,'history_unavailable');
  assert.equal(h.b.messages.value[0].content,'Saved history');
});
test('stale authorization responses cannot masquerade as successful grants',async t=>{
  const h=await mount(t,{resolveAuthorization:async()=>({accepted:false})});
  h.event({type:'authorization.requested',conversationId:'conversation-1',projectId:1,identity:'xai-api:1',requestId:'old',runId:'r',offers:[{appId:'workspace',revision:0,permissions:[{permission:'workspace.read',tools:['workspace_read']}]}]});
  await h.b.acceptGrant(); assert.equal(h.b.error.value,'host_authorization_expired');
});
