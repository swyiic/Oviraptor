// Production Host/manifest/grant routing with a local scripted desktop bridge.
// No real project, provider, network discovery, or paid request is used.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {mkdtemp,realpath,rm,readFile,mkdir,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {AgentHost} from '../src/service.mjs';
import {DiscoveryAssist,discoveryConversationAction,normalizeDiscoveryOrganization} from '../src/discovery-assist.mjs';
import {ensureBundledApps} from '../src/bundled-apps.mjs';
import {createDirectoryPackage} from '../../app-sdk/src/index.mjs';
const APP='org.oviraptor.assets',EXEC='inventory.discovery.run';
async function fixture(t,{paidOwner=false,collectionOwner=false,permissions=['inventory.read','inventory.write','certificate.collect'],trusted=false}={}){
 const root=await mkdtemp(join(await realpath(tmpdir()),'ovi-pipeline-grant-'));
 const host=new AgentHost({dataDir:root,runtimeSelection:'none'}),calls=[];
 const native=createServer(async(req,res)=>{let raw='';for await(const chunk of req)raw+=chunk;const body=JSON.parse(raw);calls.push(body);res.end(JSON.stringify({ok:true,result:{projectId:91,runId:'review-1',organization:'目标单位',includeIsolated:true,phase:'paused',enabled:false,review:{run:null},proposal:null,pipeline:{phase:'paused',round:1,collectionRunId:collectionOwner?44:null,allowHunter:paidOwner,counts:{},hunter:{queries:0,pages:0,spentPoints:0,costUnknown:false},reason:''}}}));});
 await new Promise(resolve=>native.listen(0,'127.0.0.1',resolve));
 host.apps.discoveryAssist=new DiscoveryAssist({origin:`http://127.0.0.1:${native.address().port}`,token:'e'.repeat(64)});
 const sourceDirectory=fileURLToPath(new URL('../../../apps/assets/',import.meta.url));
 const spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const pack=await createDirectoryPackage({...spec,sourceDirectory,outputDirectory:join(root,'package')});
 await host.apps.install(pack.directory);await host.apps.setEnabled(APP,true);
 const project=host.store.createProject('Permission boundary',{externalId:'91'}),other=host.store.createProject('Other',{externalId:'92'}),conversation=host.store.createConversation(project.id,'发现流程');
 host.apps.setGrants(APP,project.id,permissions,0,host.runtimeIdentity());
 if(trusted){const bundled=join(root,'bundled');await mkdir(join(bundled,'packages','agent-host'),{recursive:true});await writeFile(join(bundled,'packages','agent-host','bundled-apps.json'),JSON.stringify({schemaVersion:1,apps:[{id:APP,package:pack.directory,enable:true,builtin:true,uninstallable:false}]}));const loaded=await ensureBundledApps(host,{root:bundled});assert.equal(loaded.items[0].status,'enabled');}
 t.after(async()=>{await host.close();await new Promise(resolve=>native.close(resolve));await rm(root,{recursive:true,force:true});});
 return{root,host,project,other,conversation,calls,spec};
}
async function view(f){return discoveryConversationAction(f.host,f.conversation.id,f.project.id);}
async function start(f,options={}){const d=await view(f);return discoveryConversationAction(f.host,f.conversation.id,f.project.id,{...d.confirmationWitness,action:'start',organization:'目标单位',includeIsolated:true,...options});}
test('human paid or FOFA start requires execution permission before any native write',async t=>{
 const f=await fixture(t);
 for(const options of [{allowHunter:true},{collectFirst:true,collectionTargets:['example.test']}])await assert.rejects(start(f,options),{code:'host_tool_permission_denied'});
 assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('discovery execution permission cannot replace the certificate collection grant',async t=>{
 const f=await fixture(t,{permissions:['inventory.read','inventory.write',EXEC]});
 await assert.rejects(start(f,{allowHunter:true}),{code:'host_tool_permission_denied'});assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('a model cannot manufacture Hunter or FOFA consent through the old analysis schema',async t=>{
 const f=await fixture(t,{permissions:['inventory.read','inventory.write','certificate.collect',EXEC]}),grant=f.host.apps.effectiveGrant(f.host.store.application(APP),f.project.id,f.host.runtimeIdentity());
 for(const field of [{allowHunter:true},{collectFirst:true,collectionTargets:['example.test']}])await assert.rejects(f.host.apps.call(APP,'discovery_assist_start',f.project.id,{organization:'目标单位',includeIsolated:true,...field},grant.revision,'model-paid',undefined,f.host.runtimeIdentity()),{code:'app_arguments_invalid'});
 assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('continuing an already paid owner does not bypass revoked execution permission by omitting options',async t=>{
 const f=await fixture(t,{paidOwner:true});await assert.rejects(start(f),{code:'host_tool_permission_denied'});assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('trusted human action forwards exact first-collection options once without authorizing scope',async t=>{
 const f=await fixture(t,{permissions:['inventory.read','inventory.write','certificate.collect',EXEC]});
 await start(f,{collectFirst:true,collectionTargets:['example.test'],allowHunter:true});const writes=f.calls.filter(c=>c.action==='start');assert.equal(writes.length,1);assert.equal(writes[0].projectId,91);assert.deepEqual(writes[0].args,{organization:'目标单位',includeIsolated:true,collectFirst:true,collectionTargets:['example.test'],allowHunter:true});assert.equal(f.calls.some(c=>c.action==='confirm'),false);
});
test('existing full access includes this approved execution capability after asset manifest update without changing other projects',async t=>{
 const f=await fixture(t,{permissions:[],trusted:true}),app=f.host.store.application(APP),updated=structuredClone(app.manifest),old=structuredClone(updated);
 old.permissions=old.permissions.filter(p=>p.id!==EXEC);f.host.store.db.prepare('UPDATE applications SET manifest=? WHERE id=?').run(JSON.stringify(old),APP);
 f.host.setAccessPolicy({projectId:f.project.id,mode:'full',expectedRevision:0,expectedRuntimeIdentity:f.host.runtimeIdentity(),operatorConfirmed:true});
 f.host.store.db.prepare('UPDATE applications SET manifest=? WHERE id=?').run(JSON.stringify(updated),APP);
 await start(f,{allowHunter:true});assert.equal(f.calls.filter(c=>c.action==='start').length,1);assert.equal(f.host.accessPolicy.view(f.other.id).mode,'ask');assert.equal(f.host.status().authorizations.length,0);
});

function legacyFull(f,permissions=['inventory.write','certificate.collect']){
 const app=f.host.store.application(APP),current=structuredClone(app.manifest),old=structuredClone(current);old.permissions=old.permissions.filter(p=>permissions.includes(p.id));
 f.host.store.db.prepare('UPDATE applications SET manifest=? WHERE id=?').run(JSON.stringify(old),APP);
 f.host.setAccessPolicy({projectId:f.project.id,mode:'full',expectedRevision:0,expectedRuntimeIdentity:f.host.runtimeIdentity(),operatorConfirmed:true});
 f.host.store.db.prepare('UPDATE applications SET manifest=? WHERE id=?').run(JSON.stringify(current),APP);
}
function effective(f){return f.host.apps.effectiveGrant(f.host.store.application(APP),f.project.id,f.host.runtimeIdentity());}
test('compatibility is effective-only with a provenance marker and cannot mutate stored consent',async t=>{
 const f=await fixture(t,{permissions:[],trusted:true});legacyFull(f);
 const policy=f.host.store.db.prepare('SELECT * FROM project_access_policies WHERE project_id=?').get(f.project.id),grants=f.host.store.grants(APP);
 const grant=effective(f);assert.equal(grant.permissions.includes(EXEC),true);assert.equal(grant.permissionCompatibility.source,'compatibility');
 assert.deepEqual(f.host.store.db.prepare('SELECT * FROM project_access_policies WHERE project_id=?').get(f.project.id),policy);assert.deepEqual(f.host.store.grants(APP),grants);
});
test('builtin flags and an asset app ID without validated source provenance never receive compatibility',async t=>{
 const f=await fixture(t,{permissions:[]});f.host.store.db.prepare('UPDATE applications SET builtin=1,uninstallable=0 WHERE id=?').run(APP);legacyFull(f);
 assert.equal(effective(f).permissions.includes(EXEC),false);await assert.rejects(start(f,{allowHunter:true}),{code:'host_tool_permission_denied'});assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('old full consent with only inventory write does not inherit discovery execution',async t=>{
 const f=await fixture(t,{permissions:[],trusted:true});legacyFull(f,['inventory.write']);assert.equal(effective(f).permissions.includes(EXEC),false);
});
test('runtime changes, disabling the app, and replacing its package identity invalidate compatibility',async t=>{
 const f=await fixture(t,{permissions:[],trusted:true});legacyFull(f);assert.equal(effective(f).permissions.includes(EXEC),true);
 f.host.runtimeRevision++;assert.equal(effective(f).permissions.includes(EXEC),false);f.host.runtimeRevision--;
 f.host.store.db.prepare('UPDATE applications SET enabled=0 WHERE id=?').run(APP);assert.equal(effective(f).permissions.includes(EXEC),false);f.host.store.db.prepare('UPDATE applications SET enabled=1 WHERE id=?').run(APP);
 f.host.store.db.prepare('UPDATE applications SET digest=?,revision=revision+1 WHERE id=?').run('f'.repeat(64),APP);assert.equal(effective(f).permissions.includes(EXEC),false);
});

test('paused FOFA collection cannot be resumed without execution permission by sending collectFirst false',async t=>{
 const f=await fixture(t,{collectionOwner:true});await assert.rejects(start(f,{collectFirst:false,allowHunter:false}),{code:'host_tool_permission_denied'});
 const grant=effective(f);await assert.rejects(f.host.apps.call(APP,'discovery_assist_start',f.project.id,{organization:'目标单位',includeIsolated:true},grant.revision,'model-resume',undefined,f.host.runtimeIdentity()),{code:'host_tool_permission_denied'});
 assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('permission revocation during a status read is rechecked before the trusted native start',async t=>{
 const f=await fixture(t,{permissions:['inventory.read','inventory.write','certificate.collect',EXEC]}),d=await view(f);
 let release,entered;const gate=new Promise(resolve=>release=resolve),waiting=new Promise(resolve=>entered=resolve),original=f.host.apps.discoveryAssist.readFull.bind(f.host.apps.discoveryAssist);
 f.host.apps.discoveryAssist.readFull=async(...args)=>{entered();await gate;return original(...args);};
 const pending=discoveryConversationAction(f.host,f.conversation.id,f.project.id,{...d.confirmationWitness,action:'start',organization:'目标单位',includeIsolated:true,allowHunter:true});await waiting;
 const grant=f.host.store.grant(APP,f.project.id);f.host.apps.setGrants(APP,f.project.id,['inventory.read','inventory.write','certificate.collect'],grant.revision,f.host.runtimeIdentity());release();
 await assert.rejects(pending,e=>['host_tool_authority_changed','host_tool_permission_denied'].includes(e.code));assert.equal(f.calls.some(c=>c.action==='start'),false);
});

for(const owner of ['paidOwner','collectionOwner'])test(`native-equivalent whitespace cannot bypass the saved ${owner} execution grant in either channel`,async t=>{
 const f=await fixture(t,{[owner]:true}),grant=effective(f);
 for(const organization of ['目标单位 ',' 目标单位','\t目标单位\n','\u0085目标单位\u0085','\u3000目标单位\u3000']){
  await assert.rejects(f.host.apps.call(APP,'discovery_assist_start',f.project.id,{organization,includeIsolated:true},grant.revision,'normalized-model-resume',undefined,f.host.runtimeIdentity()),{code:'host_tool_permission_denied'});
  await assert.rejects(start(f,{organization,collectFirst:false}),{code:'host_tool_permission_denied'});
 }
 assert.equal(f.calls.some(c=>c.action==='start'),false);
});
test('authorized controls pass one canonical target to native in both channels',async t=>{
 const f=await fixture(t,{paidOwner:true,permissions:['inventory.read','inventory.write','certificate.collect',EXEC]}),grant=effective(f);
 await f.host.apps.call(APP,'discovery_assist_start',f.project.id,{organization:'\u0085目标单位\u3000',includeIsolated:true},grant.revision,'normalized-model-owner',undefined,f.host.runtimeIdentity());
 await start(f,{organization:' 目标单位 \n',collectFirst:false,allowHunter:true});
 const writes=f.calls.filter(c=>c.action==='start');assert.equal(writes.length,2);assert.deepEqual(writes.map(c=>c.args.organization),['目标单位','目标单位']);
});
test('missing, null, empty and native-whitespace-only targets never reach a native owner',async t=>{
 const f=await fixture(t,{permissions:['inventory.read','inventory.write','certificate.collect',EXEC]}),grant=effective(f);
 for(const organization of [undefined,null,'',' \t\n','\u0085\u3000']){
  await assert.rejects(f.host.apps.call(APP,'discovery_assist_start',f.project.id,{...(organization===undefined?{}:{organization}),includeIsolated:true},grant.revision,'invalid-normalized-target',undefined,f.host.runtimeIdentity()),{code:'app_arguments_invalid'});
  await assert.rejects(start(f,{organization,collectFirst:false}),{code:'host_tool_input_invalid'});
 }
 assert.equal(f.calls.some(c=>c.action==='start'),false);
 assert.equal(normalizeDiscoveryOrganization('\uFEFF目标单位\uFEFF'),'\uFEFF目标单位\uFEFF','BOM is not Rust Unicode White_Space and cannot alias a saved target');
});
