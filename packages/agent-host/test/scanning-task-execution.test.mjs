import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp,realpath,rm,readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ScanningTasks } from '../src/scanning-tasks.mjs';
import { HostStore } from '../src/store.mjs';
import { AppManager } from '../src/apps.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { scanningTaskIntent } from '../src/quality-prompt.mjs';
import { resolveDesktopAssetIds } from '../../app-sdk/src/desktop-asset-sync.mjs';
import { ensureAssetInventorySchema } from '../../app-sdk/src/asset-inventory-schema.mjs';

test('Assets identity translation distinguishes colliding numbers and refuses other projects, providers and invalid external IDs',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-asset-identity-'));t.after(()=>rm(root,{recursive:true,force:true}));const path=join(root,'inventory.sqlite3');const db=new DatabaseSync(path);ensureAssetInventorySchema(db);
 const insert=db.prepare('INSERT INTO inventory(id,project_id,source,external_id,observation_json,received_at) VALUES(?,?,?,?,?,?)');
 for(const row of [[11,1,'desktop-assets','99'],[99,1,'desktop-assets','100'],[12,2,'desktop-assets','101'],[13,1,'hunter','102'],[14,1,'desktop-assets','-1'],[15,1,'desktop-assets','9007199254740992']])insert.run(...row,'{}','now');
 const before=db.prepare('SELECT * FROM inventory ORDER BY id').all();db.close();
 const resolve=(ids,space)=>resolveDesktopAssetIds({inventoryDbPath:path,hostProjectId:1,assetIds:ids,assetIdNamespace:space});
 assert.deepEqual(resolve([99],'inventory').desktopAssetIds,[100]);assert.deepEqual(resolve([99],'desktop').desktopAssetIds,[99]);
 for(const ids of [[12],[13],[14],[15],[11,11]])assert.throws(()=>resolve(ids,'inventory'));
 assert.throws(()=>resolve([11],undefined));const after=new DatabaseSync(path,{readOnly:true});assert.deepEqual(after.prepare('SELECT * FROM inventory ORDER BY id').all(),before);after.close();
});

test('desktop scan bridge translates only saved project bindings and rejects response mismatch',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-scan-channel-'));const store=new HostStore(join(root,'host.sqlite'));const project=store.createProject('one',{externalId:'91'});let seen,bound=91;
 const server=createServer(async(req,res)=>{let body='';for await(const chunk of req)body+=chunk;seen={route:req.url,body:JSON.parse(body)};res.end(JSON.stringify({ok:true,result:{projectId:bound,task:{id:'original-task',status:'paused'}}}));});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 t.after(async()=>{await new Promise(resolve=>server.close(resolve));store.close();await rm(root,{recursive:true,force:true});});
 const bridge=new ScanningTasks({origin:`http://127.0.0.1:${server.address().port}`,token:'a'.repeat(64)});
 const result=await bridge.call(store,project.id,'scan_pause',{taskId:'original-task'},'pause-once');
 assert.equal(seen.route,'/scan-task');assert.equal(seen.body.projectId,91);assert.equal(seen.body.action,'pause');assert.equal(result.structuredContent.projectId,project.id);assert.equal(result.structuredContent.desktopProjectId,91);
 bound=92;await assert.rejects(bridge.call(store,project.id,'scan_status',{taskId:'original-task'},'read'),{code:'host_scan_project_binding_changed'});
 const unmapped=store.createProject('unmapped');await assert.rejects(bridge.call(store,unmapped.id,'scan_list',{},'read'),{code:'host_scan_project_unmapped'});
});

test('AppManager separates read, draft, execution and pause permissions; desktop calls never reach Node worker',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-scan-app-'));const store=new HostStore(join(root,'host.sqlite'));const apps=new AppManager(store,join(root,'apps'));t.after(async()=>{await apps.close();store.close();await rm(root,{recursive:true,force:true});});
 const sourceDirectory=fileURLToPath(new URL('../../../apps/scanning/',import.meta.url));const spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,'package')});
 await apps.install(pack.directory);await apps.setEnabled(spec.manifest.id,true);const project=store.createProject('bound',{externalId:'91'});const id=spec.manifest.id;const calls=[];
 const assetsSource=fileURLToPath(new URL('../../../apps/assets/',import.meta.url));const assetsSpec=JSON.parse(await readFile(join(assetsSource,'app.package.json'),'utf8'));const assetsPack=await createDirectoryPackage({...assetsSpec,sourceDirectory:assetsSource,outputDirectory:join(root,'assets-package')});await apps.install(assetsPack.directory);await apps.setEnabled(assetsSpec.manifest.id,true);
 const db=new DatabaseSync(join(apps.dataDirectoryFor(store.application(assetsSpec.manifest.id)),'inventory.sqlite3'));
 db.prepare("INSERT INTO inventory(id,project_id,source,external_id,observation_json,received_at) VALUES(42,?,'desktop-assets','9001','{}','now')").run(project.id);db.close();
 apps.scanningTasks={available:true,async call(_s,p,name,args,operation){calls.push({p,name,args,operation});return{structuredContent:{projectId:p,task:{id:'scan',status:'draft'}}};}};
 let grant=apps.setGrants(id,project.id,['investigation.read','investigation.draft.write'],undefined,'runtime:1');
 assert(apps.capabilities(project.id,{runtimeIdentity:'runtime:1'}).some(t=>t.name.endsWith('scan_list')));
 assert(!apps.capabilities(project.id,{runtimeIdentity:'runtime:1'}).some(t=>t.name.endsWith('scan_start')));
 const handoff=await apps.call(id,'scan_from_assets',project.id,{assetIds:[42],assetIdNamespace:'inventory',maxModelRequests:3,maxModelTokens:64000},grant.revision,'handoff');
 assert.deepEqual(calls[0].args.assetIds,[9001]);assert.equal(handoff.structuredContent.assetBinding.bindings[0].inventoryRecordId,42);
 assert.equal(calls[0].args.maxModelRequests,3);assert.equal(calls[0].args.maxModelTokens,64000);
 await assert.rejects(apps.call(id,'scan_from_assets',project.id,{assetIds:[42],assetIdNamespace:'desktop'},grant.revision,'wrong-namespace'),{code:'host_scan_asset_identity_unavailable'});
 await assert.rejects(apps.call(id,'scan_from_assets',project.id,{assetIds:[42]},grant.revision,'missing-namespace'));
 await assert.rejects(apps.call(id,'scan_start',project.id,{taskId:'scan'},grant.revision,'denied'),{code:'host_tool_permission_denied'});assert.equal(calls.length,1);
 grant=apps.setGrants(id,project.id,[...grant.permissions,'investigation.execute','investigation.control'],grant.revision,'runtime:1');
 await apps.call(id,'scan_start',project.id,{taskId:'scan'},grant.revision,'start');await apps.call(id,'scan_pause',project.id,{taskId:'scan'},grant.revision,'pause');
 await assert.rejects(apps.call(id,'scan_resume',project.id,{taskId:'scan',projectId:999},grant.revision,'spoof'));
 assert.deepEqual(calls.map(v=>v.name),['scan_from_assets','scan_start','scan_pause']);
 apps.scanningTasks.available=false;assert(!apps.capabilities(project.id,{runtimeIdentity:'runtime:1'}).some(t=>t.name.endsWith('scan_list')));
});

test('scan control intent does not turn negative or explanatory requests into writes',()=>{
 assert.equal(scanningTaskIntent('暂停当前扫描任务').action,'pause');assert.equal(scanningTaskIntent('恢复扫描任务 abc').action,'resume');
 assert.equal(scanningTaskIntent('查看当前扫描任务，不要启动').action,'status');
 assert.equal(scanningTaskIntent('不要开始扫描'),null);assert.equal(scanningTaskIntent('如何启动扫描'),null);
 assert.equal(scanningTaskIntent('扫描启动前检查模型与预算').action,'preflight');
 assert.equal(scanningTaskIntent('当前扫描任务是否可以启动').action,'preflight');
 assert.equal(scanningTaskIntent('把资产交给扫描，先创建草稿'),null);
 assert.equal(scanningTaskIntent('找到当前项目 bi.example.com 的 HTTPS 库存，新建 quick 原生扫描草稿。先不要启动。随后用 scan_preflight 核对模型与上限。'),null);
});

test('inventory handoff followed by preflight retains the complete read-create-read tool path',async()=>{
 const {OpenAiCompatibleRuntime}=await import('../src/provider.mjs');const calls=[];let step=0;
 const sequence=[['assets__inventory_query',{sql:'SELECT id FROM assets'}],['scanning__scan_from_assets',{assetIds:[42],assetIdNamespace:'inventory',maxModelRequests:3,maxModelTokens:128000}],['scanning__scan_preflight',{taskId:'new-draft'}]];
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic-api-key'},fetch:async(_url,init)=>{
  const body=JSON.parse(init.body),call=sequence[step++];
  if(call)assert(body.tools.some(t=>t.function.name===call[0]),`missing handoff tool ${call[0]}`);
  return{ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:call?{tool_calls:[{index:0,id:'handoff-'+step,function:{name:call[0],arguments:JSON.stringify(call[1])}}]}:{content:'草稿已建立并读取预检，没有启动。'},finish_reason:call?'tool_calls':'stop'}]})}\n\n`;})()};
 }});
 runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:['assets__inventory_query','scanning__scan_from_assets','scanning__scan_preflight','scanning__scan_start','scanning__scan_status'].map(name=>({name,inputSchema:{type:'object'}}))};calls.push(params.name);return{structuredContent:params.name.endsWith('inventory_query')?{rows:[{id:42}]}:{task:{id:'new-draft',status:'draft'}}};}});
 const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'conversation'}]}]});
 const text='找到当前项目 bi.example.com 的 HTTPS 库存，新建一个独立 quick 原生扫描测试草稿，仅这一个目标，maxModelRequests=3、maxModelTokens=128000。先不要启动。随后用 scan_preflight 核对模型、目标和上限，返回新任务 ID。';
 const result=await runtime.prompt({sessionId,prompt:text,userText:text});
 assert.equal(result.stopReason,'end_turn');assert.deepEqual(calls,sequence.map(([name])=>name));
});

test('startup questions can find a task and inspect saved model limits without execution tools or a control call',async()=>{
 const {OpenAiCompatibleRuntime}=await import('../src/provider.mjs');const requests=[],calls=[];
 const response=(name,args,text)=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:name?{tool_calls:[{index:0,id:'preflight-'+requests.length,function:{name:'scanning__'+name,arguments:JSON.stringify(args)}}]}:{content:text},finish_reason:name?'tool_calls':'stop'}]})}\n\n`;})()});
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic-api-key'},fetch:async(_url,init)=>{
  const body=JSON.parse(init.body);requests.push(body);
  if(requests.length===1){assert(!body.tools.some(t=>/scan_(start|pause|resume)$/.test(t.function.name)));return response('scan_list',{});}
  if(requests.length===2){assert(body.tools.some(t=>t.function.name.endsWith('scan_preflight')));return response('scan_preflight',{taskId:'native-scan'});}
  assert.equal(body.tools,undefined);return response(null,null,'已读取保存的模型和请求限额；没有启动扫描。');
 }});
 runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:['scan_list','scan_status','scan_preflight','scan_start','scan_pause','scan_resume'].map(name=>({name:'scanning__'+name,inputSchema:{type:'object'}}))};calls.push(params.name);return{structuredContent:params.name.endsWith('scan_list')?{tasks:[{id:'native-scan',status:'draft'}]}:{task:{id:'native-scan',status:'draft'},modelConfigured:true,engineAdmissionValidated:false,executionAuthorizedByThisRead:false}};}});
 const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'conversation'}]}]});
 const text='当前扫描任务是否可以启动？先检查模型与预算。';const result=await runtime.prompt({sessionId,prompt:text,userText:text});
 assert.equal(result.stopReason,'end_turn');assert.deepEqual(calls,['scanning__scan_list','scanning__scan_preflight']);
});

for(const uncertain of [false,true])test(`Agent performs one scan control and one live read, outcomeUnknown=${uncertain}`,async()=>{
 const {OpenAiCompatibleRuntime}=await import('../src/provider.mjs');const requests=[],calls=[];
 const response=(name,args,text)=>({ok:true,status:200,body:(async function*(){yield `data: ${JSON.stringify({choices:[{delta:name?{tool_calls:[{index:0,id:'call-'+requests.length,function:{name:'scanning__'+name,arguments:JSON.stringify(args)}}]}:{content:text},finish_reason:name?'tool_calls':'stop'}]})}\n\n`;})()});
 const runtime=new OpenAiCompatibleRuntime({config:{provider:'openai-compatible',baseUrl:'https://fixture.invalid/v1',model:'fixture-model',apiKey:'synthetic-api-key'},fetch:async(_url,init)=>{
  const body=JSON.parse(init.body);requests.push(body);
  if(requests.length===1){assert(!body.tools.some(t=>/host_task|inventory/.test(t.function.name)));return response('scan_pause',{taskId:'native-scan'});}
  if(requests.length===2){assert.deepEqual(body.tools.map(t=>t.function.name),['scanning__scan_status']);return response('scan_status',{taskId:'native-scan'});}
  assert.equal(body.tools,undefined);return response(null,null,'根据 scan_status，任务已暂停。');
 }});
 runtime.bindHost({async mcp(_t,_c,method,params){if(method==='tools/list')return{tools:['scan_list','scan_status','scan_start','scan_pause','scan_resume'].map(name=>({name:'scanning__'+name,inputSchema:{type:'object'}}))};calls.push(params.name);return{structuredContent:uncertain&&params.name.endsWith('pause')?{outcomeUnknown:true,retryable:false,taskId:'native-scan'}:{task:{id:'native-scan',status:'paused'}}};}});
 const {sessionId}=await runtime.newSession({mcpServers:[{env:[{name:'OVIRAPTOR_MCP_TOKEN',value:'scope'},{name:'OVIRAPTOR_CONVERSATION',value:'conversation'}]}]});
 const result=await runtime.prompt({sessionId,prompt:'暂停当前扫描任务 native-scan，并核对状态。',userText:'暂停当前扫描任务 native-scan，并核对状态。'});
 assert.equal(result.stopReason,'end_turn');assert.deepEqual(calls,['scanning__scan_pause','scanning__scan_status']);assert.equal(requests.length,3);
});
