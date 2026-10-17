import test from 'node:test';
import assert from 'node:assert/strict';
import {HostApi,SseParser} from '../src/api.js';
import {ChatController} from '../src/controller.js';
import {appPageUrl,bridgeMessage,BRIDGE_PROTOCOL} from '../src/security.js';
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};};
const json=(value,status=200)=>new Response(JSON.stringify(value),{status,headers:{'Content-Type':'application/json'}});
const fixture=(overrides={})=>{const calls=[];const api={list:async path=>{calls.push(['list',path]);return path.includes('conversations?')?[{id:'chat-'+new URL('http://local'+path).searchParams.get('projectId'),title:'Chat'}]:[];},post:async(path,input)=>{calls.push(['post',path,input]);return {};},...overrides};const c=new ChatController(api);return {c,calls,api};};
test('SSE accepts arbitrary byte chunks, CRLF, comments, and explicit event type',()=>{const rows=[],p=new SseParser(x=>rows.push(x));for(const part of [': beat\r\neve','nt: message.delta\r\ndata: {"conver','sationId":"a",\r\ndata: "delta":"hello"}\r\n\r','\n'])p.push(part);assert.deepEqual(rows,[{type:'message.delta',conversationId:'a',delta:'hello'}]);});
test('SSE rejects malformed or unbounded frames instead of inventing a reply',()=>{assert.throws(()=>new SseParser(()=>{}).push('data: invalid\n\n'),/Invalid event/);assert.throws(()=>new SseParser(()=>{}).push('x'.repeat(1024*1024+1)),/limit/);});
test('host credentials use a header and never enter URL or request body',async()=>{const calls=[];const api=new HostApi({token:'synthetic-session',fetchImpl:async(...args)=>{calls.push(args);return json({items:[]});}});await api.list('/api/projects');await api.post('/api/conversations/a/messages',{text:'hello'});assert.equal(calls[0][0],'/api/projects');assert.equal(calls[0][1].headers['X-Oviraptor-Session'],'synthetic-session');assert.equal(calls[1][1].body,'{"text":"hello"}');assert(!JSON.stringify(calls.map(x=>x[0])).includes('synthetic-session'));});
test('missing session blocks HTTP and host rejections retain status and code',async()=>{let calls=0;await assert.rejects(new HostApi({fetchImpl:()=>calls++}).request('/api/projects'),/session/);assert.equal(calls,0);const api=new HostApi({token:'x',fetchImpl:async()=>json({error:{code:'app_busy',message:'Application busy'}},409)});await assert.rejects(api.post('/api/apps/a/state',{enabled:false}),e=>e.status===409&&e.code==='app_busy');});
test('fresh host starts with no applications and no business API calls',async()=>{const calls=[];const {c}=fixture({list:async path=>{calls.push(path);return [];},request:async path=>{calls.push(path);return {runtime:{state:'unconfigured'},protocolVersion:1};}});await c.init();assert.deepEqual(c.state.apps,[]);assert.equal(c.state.runtime.state,'unconfigured');assert.deepEqual(calls.sort(),['/api/apps','/api/projects','/api/status']);});
test('late old project list cannot overwrite the current project conversation',async()=>{const old=deferred();const {c}=fixture({list:async path=>path.includes('projectId=1')?old.promise:path.includes('projectId=2')?[{id:'b'}]:[]});const a=c.selectProject(1);await c.selectProject(2);old.resolve([{id:'old'}]);await a;assert.equal(c.state.projectId,2);assert.equal(c.state.conversationId,'b');assert.deepEqual(c.state.conversations,[{id:'b'}]);});
test('history read cannot erase streaming tokens received while it was in flight',async()=>{const d=deferred();const {c}=fixture({list:()=>d.promise});c.state.conversationId='a';const read=c.reloadMessages('a');c.event({type:'message.delta',conversationId:'a',runId:'r',messageId:'m',delta:'live'});d.resolve([]);await read;assert.equal(c.state.messages[0].text,'live');assert.equal(c.state.messages[0].streaming,true);});
test('double send persists one request and a rejection preserves the draft without retry',async()=>{const d=deferred();const {c,calls}=fixture({post:async(...args)=>{calls.push(args);return d.promise;}});c.state.projectId=1;c.state.conversationId='a';c.setDraft('keep me');const first=c.send();assert.equal(await c.send(),false);assert.equal(calls.length,1);d.reject(Error('synthetic admission rejected'));assert.equal(await first,false);assert.equal(c.draft(),'keep me');assert.equal(calls.length,1);});
test('send completion cannot clear another conversation draft or switch its history',async()=>{const d=deferred();const {c}=fixture({post:()=>d.promise});c.state.projectId=1;c.state.conversationId='a';c.setDraft('old');const first=c.send();c.state.projectId=2;c.state.conversationId='b';c.setDraft('new');d.resolve({});await first;assert.equal(c.state.conversationId,'b');assert.equal(c.draft(),'new');});
test('conversation creation cannot replace a newly selected project after its list resolves',async()=>{const d=deferred();let creates=false;const {c}=fixture({post:async()=>{creates=true;return {id:'new'};},list:async path=>creates&&path.includes('projectId=1')?d.promise:[]});c.state.projectId=1;const pending=c.createConversation();await Promise.resolve();await c.selectProject(2);d.resolve([{id:'new'}]);await pending;assert.equal(c.state.projectId,2);assert.deepEqual(c.state.conversations,[]);});
test('delta scope and message IDs are separate from transport event identity',()=>{const {c}=fixture();c.state.conversationId='a';c.event({type:'message.delta',conversationId:'b',messageId:'other',delta:'private'});assert.equal(c.state.messages.length,0);c.event({type:'message.delta',id:'event-1',messageId:'m',conversationId:'a',delta:'one'});c.event({type:'message.delta',id:'event-2',messageId:'m',conversationId:'a',delta:'two'});assert.deepEqual(c.state.messages.map(x=>[x.id,x.text]),[['m','onetwo']]);});
test('permission replies require a matching conversation and a declared option and are single flight',async()=>{const d=deferred();const {c,calls}=fixture({post:async(...args)=>{calls.push(args);return d.promise;}});c.state.conversationId='a';c.event({type:'permission.request',id:'p',conversationId:'a',runId:'r',options:[{id:'deny',label:'Deny'}]});await c.approve('p','invented');assert.equal(calls.length,0);const one=c.approve('p','deny');await c.approve('p','deny');assert.equal(calls.length,1);d.resolve({});await one;assert.equal(c.state.approvals.length,0);});
test('old terminal event cannot cancel a newer run or remove its permission request',()=>{const {c}=fixture();c.event({type:'runtime.status',conversationId:'a',runId:'new',state:'running'});c.event({type:'permission.request',conversationId:'a',runId:'new',id:'p',options:[]});c.event({type:'runtime.status',conversationId:'a',runId:'old',state:'cancelled'});assert.equal(c.state.runs.get('a').id,'new');assert.equal(c.state.approvals.length,1);});
test('sandbox pages accept only registered same-origin routes and contain no host token',()=>{assert.equal(appPageUrl('/apps/example/1.0/pages/home','example','http://localhost:9000'),'http://localhost:9000/apps/example/1.0/pages/home');for(const url of ['/api/status','https://foreign.test/apps/example/1.0/pages/home','/apps/other/1.0/pages/home','/apps/example/1.0/files/a.js','/apps/example/1.0/pages/home?token=x'])assert.throws(()=>appPageUrl(url,'example','http://localhost:9000'));});
test('iframe bridge checks source, opaque origin, nonce and a bounded lifecycle-only allowlist',()=>{const source={},context={source,appId:'a',nonce:'n'},data={protocol:BRIDGE_PROTOCOL,appId:'a',nonce:'n',type:'ready'};assert.deepEqual(bridgeMessage({source,origin:'null',data},context),{type:'ready'});for(const event of [{source:{},origin:'null',data},{source,origin:'http://localhost',data},{source,origin:'null',data:{...data,nonce:'old'}},{source,origin:'null',data:{...data,type:'api',path:'/api/projects'}}])assert.equal(bridgeMessage(event,context),null);assert.deepEqual(bridgeMessage({source,origin:'null',data:{...data,type:'resize',height:99999}},context),{type:'resize',height:1200});});
test('host reconnect snapshot restores active runs and approvals and clears obsolete ones',()=>{const {c}=fixture();c.state.runs.set('old',{id:'old'});c.event({type:'host.snapshot',runtime:{state:'ready'},activeRuns:[{id:'r',conversationId:'a',state:'waiting_permission'}],permissions:[{id:'p',conversationId:'a',runId:'r'}]});assert.deepEqual([...c.state.runs.keys()],['a']);assert.equal(c.state.approvals[0].id,'p');});
test('real host completed-only terminal contract clears busy state and keeps one streamed message',()=>{const {c}=fixture();c.state.conversationId='a';c.event({type:'runtime.status',conversationId:'a',runId:'r',state:'running'});c.event({type:'message.delta',conversationId:'a',runId:'r',delta:'hello'});c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'actual-message',content:'hello',status:'completed'});assert.equal(c.state.runs.size,0);assert.equal(c.state.messages.length,1);assert.equal(c.state.messages[0].id,'actual-message');assert.equal(c.state.messages[0].text,'hello');assert.equal(c.state.messages[0].streaming,false);});
test('terminal stream received before HTTP admission response cannot resurrect an already completed run',async()=>{const d=deferred();const {c}=fixture({post:()=>d.promise});c.state.projectId=1;c.state.conversationId='a';c.setDraft('hello');const send=c.send();c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'m',content:'done',status:'completed'});d.resolve({run:{id:'r',conversationId:'a',state:'starting'}});await send;assert.equal(c.state.runs.size,0);});
test('native-style fetch is invoked with the global receiver rather than a HostApi instance',async()=>{const api=new HostApi({token:'x',fetchImpl:function(){assert.equal(this,globalThis);return Promise.resolve(json({items:[]}));}});await api.list('/api/projects');});
test('browser source imports stay inside this UI package and contain no business application dependency',async()=>{const fs=await import('node:fs/promises'),path=await import('node:path'),url=await import('node:url');const root=url.fileURLToPath(new URL('../src/',import.meta.url));for(const name of await fs.readdir(root)){if(!name.endsWith('.js'))continue;const source=await fs.readFile(path.join(root,name),'utf8');for(const match of source.matchAll(/\bfrom\s+['"]([^'"]+)['"]/g)){assert(match[1].startsWith('./'),`${name} imports ${match[1]}`);assert(path.resolve(root,match[1]).startsWith(root));}}});
test('a copied standalone package builds without the repository, business apps, or node_modules',async()=>{const fs=await import('node:fs/promises'),path=await import('node:path'),os=await import('node:os'),url=await import('node:url'),child=await import('node:child_process');const packageRoot=url.fileURLToPath(new URL('../',import.meta.url)),temp=await fs.mkdtemp(path.join(os.tmpdir(),'standalone-agent-ui-'));try{for(const name of ['index.html','package.json','src','scripts'])await fs.cp(path.join(packageRoot,name),path.join(temp,name),{recursive:true});const result=child.spawnSync(process.execPath,['scripts/build.mjs'],{cwd:temp,encoding:'utf8',env:{...process.env,NODE_PATH:''}});assert.equal(result.status,0,result.stderr);const manifest=JSON.parse(await fs.readFile(path.join(temp,'dist/build-manifest.json'),'utf8'));assert.equal(manifest.format,'native-esm');assert.deepEqual(manifest.files.map(f=>f.path).sort(),['index.html',...(await fs.readdir(path.join(temp,'src'))).map(name=>'src/'+name)].sort());assert(manifest.files.some(f=>f.path==='index.html'));await assert.rejects(fs.stat(path.join(temp,'node_modules')),e=>e.code==='ENOENT');}finally{await fs.rm(temp,{recursive:true,force:true});}});
test('a history read issued after a live delta cannot replace it with the database running placeholder',async()=>{const {c}=fixture({list:async()=>[{id:'m',role:'assistant',runId:'r',content:'',status:'running'}]});c.state.conversationId='a';c.event({type:'message.delta',conversationId:'a',runId:'r',messageId:'m',delta:'already visible'});await c.reloadMessages('a');assert.equal(c.state.messages.length,1);assert.equal(c.state.messages[0].text,'already visible');assert.equal(c.state.messages[0].streaming,true);});

test('cancel terminal receipt before HTTP acknowledgement cannot restore a waiting notice',async()=>{const d=deferred();const {c}=fixture({post:()=>d.promise});c.state.conversationId='a';c.state.runs.set('a',{id:'r',state:'running'});const cancel=c.cancel();c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'m',content:'partial',status:'cancelled'});d.resolve({});await cancel;assert.equal(c.state.runs.size,0);assert.equal(c.state.notice,'Run cancelled. No automatic retry.');assert(!c.state.notice.includes('Waiting'));});
test('a later terminal receipt replaces the cancellation pending notice',async()=>{const {c}=fixture();c.state.conversationId='a';c.state.runs.set('a',{id:'r',state:'running'});await c.cancel();assert.match(c.state.notice,/Waiting/);c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'m',content:'',status:'cancelled'});assert.equal(c.state.notice,'Run cancelled. No automatic retry.');});
test('late cancellation acknowledgement cannot mark a different newer run as cancelling',async()=>{const d=deferred();const {c}=fixture({post:()=>d.promise});c.state.conversationId='a';c.state.runs.set('a',{id:'old',state:'running'});const cancel=c.cancel();c.event({type:'message.completed',conversationId:'a',runId:'old',messageId:'m',status:'cancelled'});c.state.runs.set('a',{id:'new',state:'running'});d.resolve({});await cancel;assert.equal(c.state.runs.get('a').state,'running');assert.equal(c.state.runs.get('a').id,'new');});
test('unknown cancellation outcome must not claim a successful cancellation',async()=>{const d=deferred();const {c}=fixture({post:()=>d.promise});c.state.conversationId='a';c.state.runs.set('a',{id:'r',state:'running'});const cancel=c.cancel();c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'m',status:'failed',outcomeUnknown:true,failureCode:'runtime_disconnect',cancelRequested:true});d.resolve({});await cancel;assert.match(c.state.notice,/结果未知/);assert(!c.state.notice.includes('Run cancelled'));assert.equal(c.state.messages[0].outcomeUnknown,true);assert.equal(c.state.runs.size,0);});
test('persisted incomplete and refused receipts replace live rows rather than pretending to keep receiving',async()=>{for(const status of ['incomplete','refused']){const {c}=fixture({list:async()=>[{id:'m',role:'assistant',runId:'r',content:'partial',status}]});c.state.conversationId='a';c.event({type:'message.delta',conversationId:'a',runId:'r',messageId:'m',delta:'partial'});await c.reloadMessages();assert.equal(c.state.messages[0].status,status);assert.notEqual(c.state.messages[0].streaming,true);}});
test('a prior run notice does not carry into another conversation or a newly submitted turn',async()=>{const {c}=fixture();c.state.conversationId='a';c.state.notice='Run cancelled. No automatic retry.';await c.selectConversation('b');assert.equal(c.state.notice,'');c.state.notice='Run cancelled. No automatic retry.';c.setDraft('new request');await c.send();assert.equal(c.state.notice,'');});
test('string host error codes remain typed instead of being parsed from display text',async()=>{const api=new HostApi({token:'x',fetchImpl:async()=>json({error:'host_runtime_identity_changed'},409)});await assert.rejects(api.post('/api/apps/a/grants',{}),error=>error.code==='host_runtime_identity_changed'&&error.status===409);});
test('global runtime identity from a status event replaces the previous recipient identity',()=>{const {c}=fixture();c.state.runtime={state:'configured',identity:'grok-acp:1'};c.event({type:'runtime.status',state:'unconfigured',identity:'none:2'});assert.equal(c.state.runtime.identity,'none:2');});

test('runtime.plan and runtime.step update conversation plan progress and clear on completion',()=>{
  const {c}=fixture();
  c.state.conversationId='a';
  c.event({type:'runtime.plan',conversationId:'a',runId:'r',steps:[{id:'s1',title:'First',status:'in_progress',index:0}]});
  assert.equal(c.state.plans.get('a').steps[0].title,'First');
  c.event({type:'runtime.step',conversationId:'a',runId:'r',id:'s1',title:'First',status:'completed'});
  assert.equal(c.state.plans.get('a').steps[0].status,'completed');
  c.event({type:'runtime.step',conversationId:'a',runId:'r',id:'s2',title:'Second',status:'in_progress'});
  assert.equal(c.state.plans.get('a').steps.length,2);
  c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'m',content:'done',status:'completed'});
  assert.equal(c.state.plans.has('a'),false);
});

test('conversation.updated refreshes list title without a reload',()=>{
  const {c}=fixture();
  c.state.conversations=[{id:'a',title:'New conversation'},{id:'b',title:'Other'}];
  c.event({type:'conversation.updated',conversationId:'a',title:'统计资产'});
  assert.equal(c.state.conversations[0].title,'统计资产');
  assert.equal(c.state.conversations[1].title,'Other');
});

test('runtime.heartbeat keeps run alive UI state and clears on completion',()=>{
  const {c}=fixture();
  c.state.conversationId='a';
  c.event({type:'runtime.status',conversationId:'a',runId:'r',state:'running'});
  c.event({type:'runtime.heartbeat',conversationId:'a',runId:'r',elapsedMs:12000,lastEventAt:'2026-10-17T10:00:00.000Z',stepTitle:'查询资产'});
  assert.equal(c.state.heartbeats.get('a').elapsedMs,12000);
  assert.equal(c.state.heartbeats.get('a').stepTitle,'查询资产');
  c.event({type:'message.completed',conversationId:'a',runId:'r',messageId:'m',content:'done',status:'completed'});
  assert.equal(c.state.heartbeats.has('a'),false);
  assert.equal(c.state.runs.size,0);
});

test('startScratchConversation creates scratch-backed chat and selects it',async()=>{
  const calls=[];
  const {c}=fixture({
    post:async(path,input)=>{calls.push(['post',path,input]);return path==='/api/conversations'?{id:'scratch-chat',projectId:9,title:'New conversation'}:{id:9,name:'Scratch · 2026-10-17',kind:'scratch'};},
    list:async path=>{calls.push(['list',path]);return path==='/api/projects'?[{id:9,name:'Scratch · 2026-10-17',kind:'scratch'}]:path.includes('projectId=9')?[{id:'scratch-chat',title:'New conversation'}]:path.includes('/messages')?[]:[];},
  });
  await c.startScratchConversation();
  assert.equal(c.state.projectId,9);
  assert.equal(c.state.conversationId,'scratch-chat');
  assert.equal(c.currentProject()?.kind,'scratch');
  assert.equal(calls[0][0],'post');
  assert.deepEqual(calls[0][2],{scratch:true,title:'New conversation'});
});

test('promoteCurrentProject renames scratch and refreshes project list',async()=>{
  const {c}=fixture({
    post:async(path,input)=>{assert.equal(path,'/api/projects/9/promote');assert.equal(input.name,'Asset review');return {id:9,name:'Asset review',kind:'project'};},
    list:async path=>path==='/api/projects'?[{id:9,name:'Asset review',kind:'project'}]:[],
  });
  c.state.projectId=9;
  c.state.projects=[{id:9,name:'Scratch · day',kind:'scratch'}];
  const row=await c.promoteCurrentProject('Asset review');
  assert.equal(row.kind,'project');
  assert.equal(c.state.projects[0].name,'Asset review');
  assert.equal(c.state.projects[0].kind,'project');
});

test('project.updated event refreshes kind and name in place',()=>{
  const {c}=fixture();
  c.state.projects=[{id:3,name:'Scratch · day',kind:'scratch'}];
  c.event({type:'project.updated',projectId:3,name:'Named',kind:'project'});
  assert.equal(c.state.projects[0].name,'Named');
  assert.equal(c.state.projects[0].kind,'project');
});

test('createConversation still requires a selected project',async()=>{
  const calls=[];
  const {c}=fixture({post:async(...args)=>{calls.push(args);return {id:'x'};}});
  await c.createConversation();
  assert.equal(calls.length,0);
  c.state.projectId=2;
  await c.createConversation();
  assert.equal(calls.length,1);
  assert.deepEqual(calls[0][1],{projectId:2,title:'New conversation'});
});
