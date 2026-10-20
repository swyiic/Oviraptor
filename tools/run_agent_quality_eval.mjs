// Opt-in live evaluation. Only synthetic project data is sent to the already
// selected provider. No production DB writes, user conversations or credentials
// enter the report; temporary credentials/state are deleted on completion.
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';
import { hostError } from '../packages/agent-host/src/store.mjs';
import { serveHost } from '../packages/agent-host/src/server.mjs';
import { AgentHost } from '../packages/agent-host/src/service.mjs';
import { ProviderStore } from '../packages/agent-host/src/provider.mjs';
import { HostNetworkSettings } from '../packages/agent-host/src/network-settings.mjs';
import { createDirectoryPackage } from '../packages/app-sdk/src/index.mjs';
const args=process.argv.slice(2),sourceIndex=args.indexOf('--source-host'),outputIndex=args.indexOf('--output');
if(sourceIndex<0||!args[sourceIndex+1])throw Error('Explicit --source-host is required for live provider evaluation');
const source=resolve(args[sourceIndex+1]),output=outputIndex>=0?resolve(args[outputIndex+1]):'/private/tmp/oviraptor-agent-quality-eval.json';
const store=new ProviderStore(source),config=store.read(),apiKey=store.bearer();
if(!config||!apiKey||config.authentication==='oauth')throw Error('Select an existing API-key provider first; OAuth refresh is not part of this evaluator');
const root=await mkdtemp('/private/tmp/oviraptor-agent-quality-'),events=[],network=[];
const policy=new HostNetworkSettings(source),transport=policy.createFetch();
const host=new AgentHost({dataDir:join(root,'host'),providerFetch:async(url,init)=>{const started=Date.now();try{const response=await transport(url,{...init,signal:AbortSignal.any([init.signal,AbortSignal.timeout(45000)].filter(Boolean))});network.push({status:response.status,ms:Date.now()-started});return response;}catch(error){network.push({error:error.code||error.name,ms:Date.now()-started});throw error;}},onEvent:event=>{if(event.type==='authorization.requested')events.push({type:event.type});}});
const results=[];let web;
try{
 web=await serveHost(host);
 const desktop=new DatabaseSync(join(root,'oviraptor.sqlite3'));
 desktop.exec("CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,ip TEXT,port INTEGER,protocol TEXT,domain TEXT,title TEXT,status_code INTEGER,probe_outcome TEXT);CREATE TABLE project_assets(asset_id INTEGER,project_id INTEGER,is_deleted INTEGER);CREATE TABLE asset_discovery_seeds(project_id INTEGER,host TEXT,source TEXT);INSERT INTO assets VALUES(1,'one.fixture.invalid','https://one.fixture.invalid','192.0.2.1',443,'https','one.fixture.invalid','Synthetic one',200,'web_alive'),(2,'two.fixture.invalid','https://two.fixture.invalid','192.0.2.2',443,'https','two.fixture.invalid','Synthetic two',200,'web_alive'),(3,'three.fixture.invalid','https://three.fixture.invalid','192.0.2.3',443,'https','three.fixture.invalid','Synthetic three',401,'web_restricted');INSERT INTO project_assets VALUES(1,42,0),(2,42,0),(3,42,0);INSERT INTO asset_discovery_seeds VALUES(42,'fixture-cert-a','certificate_sha256'),(42,'fixture-cert-b','certificate_sha256'),(42,'one.fixture.invalid','certificate_domain');");desktop.close();
 const repo=fileURLToPath(new URL('../',import.meta.url)),sourceDirectory=join(repo,'apps/assets');
 const spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const packaged=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,'assets-package')});
 await host.apps.install(packaged.directory);await host.apps.setEnabled(spec.manifest.id,true);
 await host.setProvider({provider:config.provider,model:config.model,baseUrl:config.baseUrl,apiKey});await host.setRuntime('xai-api');
 host.runtime.stepLimitOverride=6;
 let project=host.store.ensureProjectByExternalId('42','Synthetic evaluation project');let conversation=host.store.createConversation(project.id);
 host.apps.setGrants(spec.manifest.id,project.id,['inventory.read'],undefined,host.runtimeIdentity());
 let failNextRead=false;
 const appCall=host.apps.call.bind(host.apps);
 host.apps.call=async(...args)=>{if(failNextRead&&/^(inventory_stats|inventory_query|project_overview)$/.test(args[1])){failNextRead=false;throw hostError('host_app_tool_rejected');}return appCall(...args);};
 for(const scenario of [
  {id:'project_asset_count',text:'只查询当前项目，告诉我资产总数。不要推测，依据实际工具结果回答。',check:(text)=>/3|三/.test(text)},
  {id:'certificate_followup',text:'再查一下：当前项目已保存的证书 SHA-256 指纹一共有多少个？用工具查询，回答指纹数量，不要回答域名数量。',check:text=>/2|两|二/.test(text)},
  {id:'empty_filtered_result',text:'查询当前本地项目中域名精确等于 absent.fixture.invalid 的资产有多少。请精确过滤，只回答此查询范围内的结果。',check:text=>/0|零|未.*查到|没有.*匹配|没有.*记录/.test(text)},
  {id:'recover_read_failure',text:'请重新查询当前项目的资产总数。如果某个读取工具暂时失败，可以换一个适用的只读工具核实，不能把失败当作没有资产。',check:text=>/3|三/.test(text),injectFailure:true},
  {id:'concept_after_data',text:'现在只用一句话解释 CNAME 是什么，不要调用任何工具或查询项目数据。',check:text=>/CNAME|别名/.test(text)},
 ]){
  failNextRead=Boolean(scenario.injectFailure);
  const first=network.length;await host.send(conversation.id,{projectId:project.id,text:scenario.text});const run=host.runs.get(conversation.id);await run.completion;
  const message=host.store.messages(conversation.id,project.id).find(item=>item.runId===run.id&&item.role==='assistant');
  const task=host.task(conversation.id),evidence=task.evidence.filter(item=>item.runId===run.id);
  const ok=message.status==='completed'&&scenario.check(message.content)&&(scenario.injectFailure?evidence.some(item=>item.status==='completed'):evidence.every(item=>item.status==='completed'))&&(scenario.id!=='concept_after_data'||evidence.length===0)&&(!['project_asset_count','certificate_followup'].includes(scenario.id)||message.content.length<=350);
  results.push({id:scenario.id,ok,status:message.status,failureCode:message.failureCode,stopReason:message.stopReason,answer:message.content,tools:evidence.map(item=>({name:item.toolName,status:item.status})),requests:network.slice(first)});
 }
 project=host.store.ensureProjectByExternalId('43','Other synthetic project');conversation=host.store.createConversation(project.id);
 host.apps.setGrants(spec.manifest.id,project.id,['inventory.read'],undefined,host.runtimeIdentity());
 const before=network.length;await host.send(conversation.id,{projectId:project.id,text:'查询当前项目的资产总数，不读取其他项目。'});
 const isolatedRun=host.runs.get(conversation.id);await isolatedRun.completion;
 const isolated=host.store.messages(conversation.id,project.id).find(item=>item.runId===isolatedRun.id&&item.role==='assistant');
 results.push({id:'other_project_isolation',ok:isolated.status==='completed'&&/0|零|没有.*资产|无资产/.test(isolated.content),status:isolated.status,answer:isolated.content,requests:network.slice(before)});
 const report={syntheticData:true,productionDatabaseModified:false,productionConfigModified:false,model:config.model,results,authorizationPrompts:events.length,requests:network.length,ok:results.every(item=>item.ok)&&events.length===0};
 await writeFile(output,JSON.stringify(report,null,2)+'\n',{mode:0o600});console.log(JSON.stringify({output,ok:report.ok,scenarios:results.map(({id,ok,status,failureCode})=>({id,ok,status,failureCode})),requests:network.length}));
 if(!report.ok)process.exitCode=1;
}finally{for(const run of host.runs.values())await host.cancel(run.conversationId).catch(()=>{});await web?.close();await host.close();await rm(root,{recursive:true,force:true});}
