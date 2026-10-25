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
