import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, readFile, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { DiscoveryAssist, DISCOVERY_CLEANUP_PERMISSION } from '../src/discovery-assist.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { discoveryCleanupIntent, isReadOnlyDataRequest } from '../src/quality-prompt.mjs';
import { requestedBusinessTools } from '../src/task-intent-permissions.mjs';

async function fixture(t){
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-cleanup-host-'));
 const host=new AgentHost({dataDir:join(root,'data'),runtimeSelection:'none'}),calls=[];
 const native=createServer(async(req,res)=>{let raw='';for await(const chunk of req)raw+=chunk;const v=JSON.parse(raw);calls.push(v);res.end(JSON.stringify({ok:true,result:{projectId:v.projectId,runId:'saved-cleanup',operationId:v.operationId,phase:v.action==='cleanup_pause'?'paused':'running',snapshot:{rawIndexSeeds:29977,projectLedgerSeeds:28687,groups:900},counts:{excluded:31,needs_evidence:22,conflict:2,retained:3},items:[{groupId:1,evidence:[{sourceUrl:'https://official.example/about',observedAt:'2026-10-25',organization:'第三方单位'}]}],batches:[{id:'batch-1',excludedSeeds:31,restorable:true}],assetsDeleted:0,hunterRequests:0}}));});
 await new Promise(resolve=>native.listen(0,'127.0.0.1',resolve));
 host.apps.discoveryAssist=new DiscoveryAssist({origin:`http://127.0.0.1:${native.address().port}`,token:'c'.repeat(64)});
 const sourceDirectory=fileURLToPath(new URL('../../../apps/assets/',import.meta.url)),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,'package')});
 await host.apps.install(pack.directory);await host.apps.setEnabled(spec.manifest.id,true);
 const project=host.store.createProject('one',{externalId:'3'}),other=host.store.createProject('two',{externalId:'4'});
 const grant=host.apps.setGrants(spec.manifest.id,project.id,['inventory.read','inventory.write','certificate.collect'],0,host.runtimeIdentity());
 t.after(async()=>{await host.close();await new Promise(resolve=>native.close(resolve));await rm(root,{recursive:true,force:true});});
 return{host,project,other,grant,calls,id:spec.manifest.id};
}

test('old write/read/certificate grants disclose cleanup status but never gain the new reversible-cleanup permission',async t=>{
 const f=await fixture(t),names=f.host.apps.capabilities(f.project.id,{runtimeIdentity:f.host.runtimeIdentity()}).map(x=>x.name);
 assert(names.includes(f.id+'::discovery_cleanup_status'));
 assert(!names.some(n=>/discovery_cleanup_(start|pause|resume|restore)$/.test(n)));
 for(const name of ['start','pause','resume','restore'])await assert.rejects(f.host.apps.call(f.id,'discovery_cleanup_'+name,f.project.id,name==='start'?{organization:'中国移动集团',autoExcludeUnrelated:true}:{runId:'saved-cleanup'},f.grant.revision),{code:'host_tool_permission_denied'});
 assert.equal(f.calls.length,0);
 const status=(await f.host.apps.call(f.id,'discovery_cleanup_status',f.project.id,{filter:'excluded',page:1,pageSize:50},f.grant.revision)).structuredContent;
 assert.equal(status.projectId,f.project.id);assert.equal(status.desktopProjectId,3);assert.deepEqual(status.snapshot,{rawIndexSeeds:29977,projectLedgerSeeds:28687,groups:900});assert.equal(status.humanReview,undefined);assert.equal(status.items[0].evidence[0].organization,'第三方单位');
 await f.host.apps.call(f.id,'discovery_cleanup_status',f.project.id,{},f.grant.revision);assert.equal(f.calls.at(-1).args.pageSize,5);
});

test('explicit cleanup permission routes bounded control contracts with native project binding and Host operation IDs',async t=>{
 const f=await fixture(t);let g=f.host.apps.setGrants(f.id,f.project.id,['inventory.read',DISCOVERY_CLEANUP_PERMISSION],f.grant.revision,f.host.runtimeIdentity());
 for(const[action,args]of [['start',{organization:'  中国移动集团及其下属单位  ',autoExcludeUnrelated:true}],['pause',{runId:'saved-cleanup'}],['resume',{runId:'saved-cleanup'}],['restore',{runId:'saved-cleanup',batchId:'batch-1'}]]){
  const r=(await f.host.apps.call(f.id,'discovery_cleanup_'+action,f.project.id,args,g.revision,'operation-'+action,undefined,f.host.runtimeIdentity())).structuredContent;
  assert.equal(r.assetsDeleted,0);assert.equal(r.hunterRequests,0);assert.equal(r.operationId,'operation-'+action);
 }
 assert.deepEqual(f.calls.map(v=>v.action),['cleanup_start','cleanup_pause','cleanup_resume','cleanup_restore']);
 assert.equal(f.calls[0].args.organization,'中国移动集团及其下属单位');assert(f.calls.every(v=>v.projectId===3));
 const before=f.calls.length;
 for(const args of [{organization:'中国移动集团',autoExcludeUnrelated:false},{organization:'中国移动集团',autoExcludeUnrelated:true,projectId:4},{organization:'中国移动集团'}])await assert.rejects(f.host.apps.call(f.id,'discovery_cleanup_start',f.project.id,args,g.revision));
 await assert.rejects(f.host.apps.call(f.id,'discovery_cleanup_restore',f.other.id,{runId:'saved-cleanup'},undefined),{code:'host_tool_permission_denied'});
 await assert.rejects(f.host.apps.call(f.id,'discovery_cleanup_status',f.project.id,{pageSize:51},g.revision));
 assert.equal(f.calls.length,before);
 g=f.host.apps.setGrants(f.id,f.project.id,['inventory.read'],g.revision,f.host.runtimeIdentity());
 await assert.rejects(f.host.apps.call(f.id,'discovery_cleanup_resume',f.project.id,{runId:'saved-cleanup'},g.revision),{code:'host_tool_permission_denied'});
});

test('an existing full-access snapshot does not silently expand into newly introduced cleanup capability',async t=>{
 const f=await fixture(t),identity=f.host.runtimeIdentity();
 f.host.accessPolicy.set({projectId:f.project.id,mode:'full',operatorConfirmed:true,expectedRevision:0,expectedRuntimeIdentity:identity});
 const row=f.host.store.db.prepare('SELECT apps_json FROM project_access_policies WHERE project_id=?').get(f.project.id),snapshot=JSON.parse(row.apps_json);
 // A historical full-access snapshot predating this category, kept in temporary data only.
 snapshot[f.id]=snapshot[f.id].filter(p=>p!==DISCOVERY_CLEANUP_PERMISSION);
 f.host.store.db.prepare('UPDATE project_access_policies SET apps_json=? WHERE project_id=?').run(JSON.stringify(snapshot),f.project.id);
 const grant=f.host.apps.effectiveGrant(f.host.store.application(f.id),f.project.id,identity);
 assert(!grant.permissions.includes(DISCOVERY_CLEANUP_PERMISSION));
 assert(!f.host.apps.capabilities(f.project.id,{runtimeIdentity:identity}).some(t=>t.name.endsWith('::discovery_cleanup_start')));
});

test('cleanup intent separates explicit reversible actions, pure seed reads, explanations and negative commands',()=>{
 for(const text of ['按中国移动集团联网分析整个项目种子，明确无关的自动排除，不确定的继续补证，真正冲突再汇总给我','帮我清理当前项目全部无关种子','开始种子清洗']){assert.equal(discoveryCleanupIntent(text)?.action,'start',text);assert.equal(isReadOnlyDataRequest(text),false,text);}
 for(const[text,action]of [['当前种子清理已排除了多少条？','status'],['查看种子清洗排除列表','status'],['恢复被排除的种子','restore'],['暂停项目种子清洗','pause'],['继续种子清洗','resume'],['撤销这次种子清洗排除批次','restore'],['查看种子清洗进度','status'],['种子清洗启动了吗？','status'],['只读查看种子清洗结果，不要启动任务','status']])assert.equal(discoveryCleanupIntent(text)?.action,action,text);
 for(const text of ['读取整个项目种子清单，统计哪些是移动的','只读读取种子证据，不执行任何排除','不要清理种子，只读查看台账','如何清理种子'])assert.equal(discoveryCleanupIntent(text),null,text);
 const manifest={tools:['discovery_cleanup_start','discovery_cleanup_status','discovery_cleanup_pause','discovery_cleanup_resume','discovery_cleanup_restore'].map(name=>({name}))},options={applications:[{id:'org.oviraptor.assets',enabled:true,manifest}],certificateAvailable:true};
 assert(requestedBusinessTools('开始种子清洗',options).includes('org.oviraptor.assets::discovery_cleanup_start'));
 assert(!requestedBusinessTools('只读查看种子清洗结果，不要启动任务',options).some(n=>/_(start|pause|resume|restore)$/.test(n)));
});

test('cleanup capability descriptions cannot turn explicit start into restore/pause/resume or demand renewed permission',()=>{
 const actual='现在按“中国移动集团及其下属单位”对当前项目全量种子启动持久化自动清洗。我授权本轮联网查询公开 CT/证书、DNS/CNAME 和单位关系，使用当前模型；明确无关且通过独立证据交叉核验的自动可撤销批量排除，缺证的继续自动补证，真正冲突集中汇总给我。保留原始资产、种子、历史、配置和已确认探测范围，不调用 FOFA/Hunter，不进行范围外主动采集，不改 VPN。请直接使用 discovery_cleanup_start，启动后读取原任务状态并告诉我 runId、全量快照数量与实际进度，不要让我逐条点击，也不要把这个后台任务误称已全量完成。';
 for(const text of [actual,
  '启动项目种子清理，自动做可撤销批量排除，支持暂停恢复。',
  '开始全项目种子清洗，支持撤销，便于恢复；能暂停恢复，不调用 Hunter。',
  '请启动种子清理，让排除方便恢复，允许随时暂停和恢复。',
  '启动种子清洗，保留原始数据，不要撤销本轮批次，不要暂停或恢复旧任务。',
  'Start project seed cleanup with reversible exclusion; supports pause/resume and restore.',
 ]){assert.equal(discoveryCleanupIntent(text)?.action,'start',text);assert.equal(isReadOnlyDataRequest(text),false,text);}
 for(const[text,action]of [
  ['撤销本轮已排除的种子批次','restore'],['暂停当前种子清洗','pause'],['继续本轮种子清洗','resume'],
  ['可以撤销本轮种子排除批次','restore'],['可以暂停当前种子清洗','pause'],['可以恢复本轮种子清洗','resume'],
  ['恢复被排除的种子','restore'],['可撤销种子清洗任务已启动了吗？','status'],['支持撤销的种子清洗任务有多少条记录？','status'],
  ['只读查看种子清理进度，支持撤销但不要开始、暂停或恢复任务。','status'],
 ])assert.equal(discoveryCleanupIntent(text)?.action,action,text);
 for(const text of ['不要启动或撤销种子清洗，只读查看台账','不要清洗种子；可撤销只是功能说明，不是指令'])assert.notEqual(discoveryCleanupIntent(text)?.action,'start',text);
});

test('cleanup count constraints stay read-only and object-first continuation resumes the existing job',()=>{
 const manifest={tools:['discovery_cleanup_start','discovery_cleanup_status','discovery_cleanup_pause','discovery_cleanup_resume','discovery_cleanup_restore'].map(name=>({name}))},options={applications:[{id:'org.oviraptor.assets',enabled:true,manifest}],certificateAvailable:true};
 for(const text of ['种子清洗还有多少，先不要暂停','种子清洗多少条了，不要撤销','种子清理排除多少了，别暂停','查看种子清洗进度，不要暂停','只查看种子清洗进度，不要撤销']){
  assert.equal(discoveryCleanupIntent(text)?.action,'status',text);assert.equal(isReadOnlyDataRequest(text),true,text);
  assert.deepEqual(requestedBusinessTools(text,options),['org.oviraptor.assets::discovery_cleanup_status'],text);
 }
 for(const text of ['已暂停的种子清洗继续执行','把当前种子清洗继续下去','当前种子清洗恢复执行','种子清洗继续']){
  assert.equal(discoveryCleanupIntent(text)?.action,'resume',text);assert.equal(isReadOnlyDataRequest(text),false,text);
  assert.deepEqual(requestedBusinessTools(text,options),['org.oviraptor.assets::discovery_cleanup_status','org.oviraptor.assets::discovery_cleanup_resume'],text);
 }
 for(const[text,action]of [['种子清洗恢复了多少条','status'],['暂停当前种子清洗','pause'],['撤销本轮已排除的种子批次','restore'],['开始种子清洗，告诉我进度','start'],['不要继续种子清洗了，暂停','pause']])assert.equal(discoveryCleanupIntent(text)?.action,action,text);
});

test('the real App continuation and coordinated negative cleanup clauses cannot become restore or start',()=>{
 const actual='已暂停的种子清洗继续执行。只恢复当前项目现有的全量清洗任务，不要新建任务或撤销任何排除。保留原来全部断点与模型用量，不调用 Hunter，不改 VPN。恢复后报告同一 runId 的实际状态，不要称作已全量完成。';
 for(const text of [actual,'继续当前种子清洗，不要 创建 新任务 或 还原任何排除。','恢复当前种子清洗，不要 新建任务 和 撤销任何批次。'])assert.equal(discoveryCleanupIntent(text)?.action,'resume',text);
 for(const text of ['种子清洗还有多少，先不要暂停或恢复任务','种子清洗多少条了，不要 新建任务 或 撤销任何排除','查看种子清洗进度，不要 创建新任务或还原排除','种子清洗进度，不要 停止任务或还原任何批次']){
  assert.equal(discoveryCleanupIntent(text)?.action,'status',text);assert.equal(isReadOnlyDataRequest(text),true,text);
 }
 for(const text of ['不要新建种子清洗任务或撤销任何排除','不要 创建种子清洗任务或恢复旧任务','不要 停止种子清洗或还原任何排除','不要 还原种子排除或创建新任务','不允许 恢复种子清洗或开始新任务'])assert.equal(discoveryCleanupIntent(text),null,text);
 for(const[text,action]of [['暂停当前种子清洗，不要恢复或撤销排除','pause'],['撤销本轮种子排除批次，不要新建或恢复任务','restore'],['开始种子清洗，不要暂停或还原排除','start']])assert.equal(discoveryCleanupIntent(text)?.action,action,text);
});
