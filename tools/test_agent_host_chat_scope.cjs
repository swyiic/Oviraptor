const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {renderToString}=require('@vue/server-renderer');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const filename='src/features/agentHost/components/AgentHostChat.vue';
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'chat-scope'});
const deferred=()=>{let resolve;const promise=new Promise(done=>{resolve=done;});return{promise,resolve};};
const flush=async()=>{for(let i=0;i<4;i++)await new Promise(setImmediate);};

async function mount(t,overrides={}) {
  let listener;
  const unmount=[];const calls=[];const emitted=[];
  const client={
    ensureProject:async(_name,id)=>id,
    conversations:async projectId=>[{id:`conversation-${projectId}`,projectId,title:'fixture'}],
    messages:async()=>[], status:async()=>({permissions:[],authorizations:[]}),
    createConversation:async projectId=>({id:`new-${projectId}`,projectId,title:'fixture'}),
    send:async(...args)=>{calls.push(['send',...args]);},
    accessPolicy:async projectId=>({projectId,mode:'ask',workspacePermissions:[],apps:{},runtimeIdentity:'xai-api:1',revision:0}),cancel:async()=>{},listApps:async()=>({items:[]}),workspaceGrants:async()=>({permissions:[]}),clearWorkspaceGrants:async()=>({cleared:true}),
    resolveAuthorization:async(...args)=>{calls.push(['authorization',...args]);},
    decidePermission:async(...args)=>{calls.push(['runtime-permission',...args]);},
    events:callback=>{listener=callback;return()=>{};},...overrides,
  };
  const exports={};
  new Function('require','exports',transpile(script.content))(name=>{
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
    if(name==='@lucide/vue')return new Proxy({},{get:()=>({inheritAttrs:false,render:()=>null})});
    if(name.endsWith('/i18n'))return{useI18n:()=>({tr:(zh)=>zh})};
    if(name.endsWith('AgentPermissionMode.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name.endsWith('AgentTaskCollaboration.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name.endsWith('DiscoveryAssistPanel.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name.endsWith('ConversationMessage.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name.endsWith('ConversationFinder.vue'))return{default:{inheritAttrs:false,render:()=>null}};
    if(name==='../providerPresets')return loadPure('src/features/agentHost/providerPresets.ts');
    if(name==='../errors')return loadPure('src/features/agentHost/errors.ts');
    if(name==='../chatClient')return loadPure('src/features/agentHost/chatClient.ts');
    throw Error(`Unexpected chat import: ${name}`);
  },exports);
  const props=vue.reactive({client,settings:{runtime:{id:'xai-api'}},projectId:1,projectName:'A'});
  const scope=vue.effectScope();
  const b=scope.run(()=>exports.default.setup(props,{expose(){},emit(...args){emitted.push(args);}}));
  t.after(()=>{unmount.forEach(callback=>callback());scope.stop();});
  await flush();
  return{b,props,calls,emitted,event:event=>listener(event)};
}
test('a rejected steering send restores the draft while an accepted correction is read back from durable messages',async t=>{
 const h=await mount(t,{steer:async()=>{throw Error('host_reply_unavailable');}});
 h.b.running.value=true;h.b.draft.value='只读证书，不启动扫描';await h.b.send();
 assert.equal(h.b.draft.value,'只读证书，不启动扫描');assert.match(h.b.error.value,/host_reply_unavailable/);
 const correction={id:'steer-message',role:'user',content:h.b.draft.value,status:'completed',runId:'current',stopReason:'steer_queued'};
 h.props.client.steer=async()=>({queued:true,message:correction});h.props.client.messages=async()=>[correction,{id:'answer',role:'assistant',content:'',status:'running',runId:'current'}];
 h.b.running.value=true;await h.b.send();assert.equal(h.b.draft.value,'');assert.equal(h.b.messages.value[0].id,'steer-message');
});
test('late steering failure cannot overwrite another project draft and usage is bound to its exact assistant run',async t=>{
 const hold=deferred(),h=await mount(t,{steer:async()=>{await hold.promise;throw Error('late old-project failure');}});h.b.running.value=true;h.b.draft.value='old-project correction';const send=h.b.send();
 h.props.projectId=2;h.props.projectName='B';await flush();h.b.draft.value='new-project draft';hold.resolve();await send;assert.equal(h.b.draft.value,'new-project draft');
 h.b.messages.value=[{id:'assistant',role:'assistant',content:'answer',status:'running',runId:'current'}];
 const usage={requests:2,reportedRequests:1,unknownRequests:1,inputTokens:120,outputTokens:30};
 h.event({type:'runtime.usage',conversationId:'conversation-1',runId:'current',usage});assert.equal(h.b.messages.value[0].usage,undefined);
 h.event({type:'runtime.usage',conversationId:'conversation-2',runId:'old',usage});assert.equal(h.b.messages.value[0].usage,undefined);
 h.event({type:'runtime.usage',conversationId:'conversation-2',runId:'current',usage});assert.deepEqual(h.b.messages.value[0].usage,usage);
});

test('completed read answer uses its exact message/run scope while original task remains awaiting input',async t=>{
 const rows=[{id:'read-user',role:'user',content:'只读取当前项目扫描结果',status:'completed'},
  {id:'read-answer',role:'assistant',content:'已根据实际结果核对。',status:'completed',stopReason:'end_turn',runId:'read-run'}];
 const task={id:'task',revision:3,conversationId:'conversation-1',projectId:1,createdAt:'now',status:'awaiting_input',goal:'read',steps:[],evidence:[],artifacts:[],
  readOnlyAnswer:{kind:'read_only_answer',runId:'read-run',userMessageId:'read-user',assistantMessageId:'read-answer',historicalWorkPending:false}};
 const h=await mount(t,{messages:async()=>rows,task:async()=>task});
 assert.equal(h.b.readOnlyAnswered.value,true);assert.equal(h.b.task.value.status,'awaiting_input');
 assert.equal(h.b.taskDisplayGoal.value,rows[0].content);
 const output=compileTemplate({source:descriptor.template.content,filename,id:'read-view',compilerOptions:{bindingMetadata:script.bindings}}),rendered={};
 new Function('require','exports',transpile(output.code))(name=>name==='vue'?vue:require(name),rendered);
 const html=()=>renderToString(vue.createSSRApp({render(){return rendered.render({},[],h.props,vue.proxyRefs(h.b),{},{});}}));
 assert.match(await html(),/本轮只读核对已答复/);assert.doesNotMatch(await html(),/继续原任务/);
 h.b.task.value={...task,outcomeUnknown:true,readOnlyAnswer:{...task.readOnlyAnswer,historicalWorkPending:true}};
 const history=await html();assert.match(history,/历史任务仍需处理/);assert.match(history,/上次操作结果未确认/);assert.match(history,/继续历史任务/);
 h.b.task.value=task;
 h.b.messages.value=[...rows,{id:'newer',role:'user',content:'new question',status:'completed'}];assert.equal(h.b.readOnlyAnswered.value,false);
 h.b.messages.value=rows;h.b.running.value=true;assert.equal(h.b.readOnlyAnswered.value,false);
 h.b.running.value=false;h.b.task.value={...task,projectId:2};assert.equal(h.b.readOnlyAnswered.value,false);
 h.b.task.value={...task,status:'paused_budget',readOnlyAnswer:null};h.b.messages.value=[];
 const capped=await html();assert.match(capped,/请求额度暂停/);assert.doesNotMatch(capped,/paused_budget/);assert.match(capped,/继续原任务/);
});

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

test('left chat history supports search and deletion without changing the active conversation',async t=>{
  const h=await mount(t,{conversations:async()=>[{id:'first',projectId:1,title:'证书记录'},{id:'second',projectId:1,title:'资产统计'}],deleteConversation:async(...args)=>{h.calls.push(['delete',...args]);}});
  assert.equal(h.b.selectedId.value,'second');h.b.historyQuery.value='证书';
  assert.deepEqual(h.b.visibleConversations.value.map(item=>item.id),['first']);
  await h.b.removeConversation('first');assert.equal(h.b.selectedId.value,'second');
  assert.deepEqual(h.calls[0],['delete','first',1]);
});
test('finder opens from the shell without sending, bounds recent rows and retains an older selection',async t=>{
 const h=await mount(t,{conversations:async projectId=>Array.from({length:500},(_,i)=>({id:`${projectId}-${i}`,projectId,title:`证书记录 ${i}`}))});
 assert.equal(h.b.recentConversations.value.length,20);
 h.props.historySearchRequest=1;await flush();assert.equal(h.b.finderOpen.value,true);
 h.b.findConversation('1-10');await flush();assert.equal(h.b.selectedId.value,'1-10');
 assert.equal(h.b.recentConversations.value.length,20);assert.ok(h.b.recentConversations.value.some(item=>item.id==='1-10'));
 assert.equal(h.calls.some(item=>item[0]==='send'),false);
 h.props.projectId=2;h.props.projectName='B';await flush();assert.equal(h.b.finderOpen.value,false);
 h.b.findConversation('1-10');assert.equal(h.b.selectedId.value,'2-499');
});
test('permission details show only current project grants and stale reads cannot cross conversations',async t=>{
  const gate=deferred();
  const h=await mount(t,{listApps:async()=>({items:[{id:'assets',grants:[{projectId:1,permissions:['inventory.read'],disclosureRuntime:'runtime-1'},{projectId:2,permissions:['inventory.write']}]}]}),workspaceGrants:()=>gate.promise});
  const reading=h.b.togglePermissions();h.b.selectedId.value='another';gate.resolve({permissions:['workspace.read']});await reading;
  assert.deepEqual(h.b.savedFilePermissions.value,[]);assert.equal(h.b.permissionsOpen.value,false);
  h.props.client.workspaceGrants=async()=>({permissions:['workspace.read']});await h.b.togglePermissions();
  assert.deepEqual(h.b.savedFilePermissions.value,['workspace.read']);
  assert.deepEqual(h.b.savedAppPermissions.value[0].labels,['读取资产']);
});
test('composer owns model and permissions controls; top duplicated session controls are absent',()=>{
  assert(!descriptor.template.content.includes('agent-host-session-switcher'));
  const formEnd=descriptor.template.content.indexOf('</form>');
  const controls=descriptor.template.content.indexOf('data-testid="agent-host-composer-controls"');
  assert(formEnd<controls);assert(controls<descriptor.template.content.indexOf('data-testid="agent-host-model-picker"'));
  assert(descriptor.template.content.includes('data-testid="agent-host-history"'));
});

// Draft generation is separate from execution and must not bind an old task.
const seedDraft=(id,projectId=1)=>({id,projectId,prompt:`readonly seed analysis ${id}`,analysis:{projectId}});
test('seed refinement starts an independent chat draft, preserves the old draft and never sends automatically',async t=>{
 let created=0;const h=await mount(t,{createConversation:async projectId=>{created++;return{id:`seed-${created}`,projectId,title:'Seed analysis'};}});
 h.b.draft.value='unfinished previous question';h.props.draftRequest=seedDraft(1);await flush();
 assert.equal(created,1);assert.equal(h.b.selectedId.value,'seed-1');assert.equal(h.b.draft.value,'readonly seed analysis 1');assert.equal(h.calls.some(c=>c[0]==='send'),false);
 assert.deepEqual(h.emitted.filter(e=>e[0]==='draftApplied'),[['draftApplied',1]]);
 await h.b.applyDiscoveryDraft();assert.equal(created,1);
 h.b.selectedId.value='conversation-1';await flush();assert.equal(h.b.draft.value,'unfinished previous question');
});
test('a stale project draft is discarded while a replacement queued during chat creation is applied once',async t=>{
 const first=deferred();let created=0;
 const h=await mount(t,{createConversation:async projectId=>{created++;return created===1?first.promise:{id:`seed-${created}`,projectId,title:'Seed analysis'};}});
 h.props.draftRequest=seedDraft(1);await flush();h.props.draftRequest=seedDraft(2);await flush();
 first.resolve({id:'stale-seed',projectId:1,title:'Old request'});await flush();
 assert.equal(created,2);assert.equal(h.b.draft.value,'readonly seed analysis 2');assert.deepEqual(h.emitted.filter(e=>e[0]==='draftApplied'),[['draftApplied',2]]);
 h.props.projectId=2;h.props.draftRequest=seedDraft(3,1);await flush();assert.equal(created,2);assert.equal(h.b.draft.value,'');assert.equal(h.calls.length,0);
});
test('seed draft waits for an active response to finish before opening another chat',async t=>{
 let created=0;const h=await mount(t,{createConversation:async projectId=>{created++;return{id:'seed',projectId,title:'Seed analysis'};}});
 h.b.running.value=true;h.props.draftRequest=seedDraft(1);await flush();assert.equal(created,0);
 h.b.running.value=false;await flush();assert.equal(created,1);assert.equal(h.b.draft.value,'readonly seed analysis 1');assert.equal(h.calls.length,0);
});

test('history rename and pin mutate the clicked row without changing selection; old project responses are discarded',async t=>{
 const hold=deferred();let delayed=false;const h=await mount(t,{conversations:async projectId=>[{id:`first-${projectId}`,projectId,title:'First',createdAt:'2026-10-20T00:00:00Z'},{id:`second-${projectId}`,projectId,title:'Second',createdAt:'2026-10-23T00:00:00Z'}],updateConversation:async(id,projectId,patch)=>{h.calls.push(['update',id,projectId,patch]);if(delayed)return hold.promise;return{id,projectId,title:patch.title||'First',pinned:patch.pinned,createdAt:'2026-10-20T00:00:00Z'};}});
 await h.b.updateHistoryConversation('first-1',{title:'我的证书分析'});assert.equal(h.b.selectedId.value,'second-1');assert.equal(h.b.conversations.value.find(c=>c.id==='first-1').title,'我的证书分析');
 await h.b.updateHistoryConversation('first-1',{pinned:true});assert.equal(h.b.visibleConversations.value[0].id,'first-1');
 delayed=true;const saving=h.b.updateHistoryConversation('first-1',{title:'Late'});h.props.projectId=2;h.props.projectName='Other';await flush();hold.resolve({id:'first-1',projectId:1,title:'Late'});await saving;
 assert.equal(h.b.selectedId.value,'second-2');assert.equal(h.b.conversations.value.some(c=>c.id==='first-1'),false);
});
test('reveal workspace carries only native project/conversation IDs and a stale or mismatched read cannot open a folder',async t=>{
 const previous=globalThis.__TAURI_INTERNALS__;globalThis.__TAURI_INTERNALS__={};t.after(()=>{if(previous===undefined)delete globalThis.__TAURI_INTERNALS__;else globalThis.__TAURI_INTERNALS__=previous;});
 const h=await mount(t,{conversationWorkspace:async(id,projectId)=>({conversationId:id,projectId,path:'/private/workspace',exists:true,shared:true})});
 assert.equal(await h.b.revealHistoryConversation('conversation-1'),'/private/workspace');assert.deepEqual(h.calls[0],['native','agent_host_chat_file_action',{projectId:1,conversationId:'conversation-1',action:'reveal'}]);
 h.props.client.conversationWorkspace=async()=>({conversationId:'other',projectId:1,path:'/unexpected',shared:true});await assert.rejects(h.b.revealHistoryConversation('conversation-1'),/scope_mismatch/);assert.equal(h.calls.length,1);
 const hold=deferred();h.props.client.conversationWorkspace=()=>hold.promise;const pending=h.b.revealHistoryConversation('conversation-1');h.props.projectId=2;h.props.projectName='Other';await flush();hold.resolve({conversationId:'conversation-1',projectId:1,path:'/old',shared:true});await pending;assert.equal(h.calls.length,1);
});
test('history and real-file client routes preserve session and scope while encoding file names once',async()=>{
 const {createAgentHostChatClient}=loadPure('src/features/agentHost/chatClient.ts'),calls=[];
 const client=createAgentHostChatClient({origin:'http://127.0.0.1:9',session:'session'},async(url,init={})=>{calls.push({url,init});return{ok:true,json:async()=>({})}});
 await client.updateConversation('chat/id',7,{title:'我的分析',pinned:true});await client.conversationWorkspace('chat/id',7);await client.conversationFiles('chat/id',7);await client.previewConversationFile('chat/id',7,'reports/a #1.md');
 assert.equal(calls[0].init.method,'PATCH');assert.deepEqual(JSON.parse(calls[0].init.body),{projectId:7,title:'我的分析',pinned:true});assert.equal(calls[0].init.headers['X-Oviraptor-Session'],'session');
 assert.match(calls[1].url,/conversations\/chat%2Fid\/workspace\?projectId=7$/);assert.match(calls[2].url,/\/files\?projectId=7$/);assert.equal(new URL(calls[3].url).searchParams.get('path'),'reports/a #1.md');
});
