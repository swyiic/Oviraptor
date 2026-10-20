import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile,mkdtemp,rm,realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { requestedBusinessTools } from '../src/task-intent-permissions.mjs';
import { AgentHost } from '../src/service.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const repo=fileURLToPath(new URL('../../../',import.meta.url));
async function contracts(){
 return Promise.all(['assets','scanning'].map(async name=>{
  const {manifest}=JSON.parse(await readFile(join(repo,'apps',name,'app.package.json'),'utf8'));
  return{id:manifest.id,enabled:true,manifest};
 }));
}

test('combined first read offers both owners with exact qualified read scopes, reuses grants and never asks for execution',async t=>{
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-first-read-'));const host=new AgentHost({dataDir:join(root,'data'),runtimeSelection:'none'});t.after(async()=>{await host.close();await rm(root,{recursive:true,force:true});});
 for(const name of ['assets','scanning']){
  const sourceDirectory=join(repo,'apps',name),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,name)});await host.apps.install(pack.directory);await host.apps.setEnabled(spec.manifest.id,true);
 }
 const project=host.store.createProject('one',{externalId:'3'}),conversation=host.store.createConversation(project.id,'read');
 const request=()=>requestedBusinessTools('核对当前项目证书任务和扫描结果。',{applications:host.store.applications(),certificateAvailable:true,scanningAvailable:true});
 const tools=request();assert(tools.includes('org.oviraptor.assets::certificate_task_status'));assert(tools.includes('org.oviraptor.scanning::scan_results'));
 let offer=host.permissionOffer(conversation.id,tools);assert.deepEqual(offer.offers.map(x=>x.permissions.map(p=>p.permission)),[['inventory.read'],['investigation.read']]);
 host.apps.setGrants('org.oviraptor.assets',project.id,['inventory.read'],0,host.runtimeIdentity());
 offer=host.permissionOffer(conversation.id,request());assert.deepEqual(offer.offers.map(x=>x.appId),['org.oviraptor.scanning']);
 host.apps.setGrants('org.oviraptor.scanning',project.id,['investigation.read'],0,host.runtimeIdentity());
 assert.deepEqual(host.permissionOffer(conversation.id,request()).offers,[]);
 const other=host.store.createProject('other',{externalId:'4'}),otherChat=host.store.createConversation(other.id,'other');assert.equal(host.permissionOffer(otherChat.id,request()).offers.length,2);
});

test('first asset handoff distinguishes draft-only, explicit execution, prohibited handoff and unavailable owners',async()=>{
 const applications=await contracts(),options={applications,certificateAvailable:true,scanningAvailable:true};
 const draft=requestedBusinessTools('找到当前项目库存资产，交给扫描，新建草稿，先不要启动；启动前检查模型。',options);
 assert(draft.includes('org.oviraptor.scanning::scan_from_assets'));assert(!draft.includes('org.oviraptor.scanning::scan_start'));
 const execute=requestedBusinessTools('把库存资产交给扫描，创建草稿并启动。',options);assert(execute.includes('org.oviraptor.scanning::scan_start'));
 const denied=requestedBusinessTools('查看当前项目资产，不要交接扫描。',options);assert(!denied.some(n=>n.endsWith('scan_from_assets')));
 const read=requestedBusinessTools('核对当前项目扫描任务。只读取数据，不启动、恢复或新建。',options);assert(!read.some(n=>/scan_(start|resume|pause|from_assets)$/.test(n)));
 assert(!requestedBusinessTools('核对扫描结果。只读取数据。',{...options,scanningAvailable:false}).some(n=>n.includes('scanning')));
});
