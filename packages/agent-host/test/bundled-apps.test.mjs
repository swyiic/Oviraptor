import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const repo = fileURLToPath(new URL('../../../', import.meta.url));

async function packageApp(sourceDirectory, outputDirectory) {
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  return createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
}

async function stageRoot(t) {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-bundled-apps-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'packages', 'agent-host'), { recursive: true });
  await mkdir(join(root, 'apps'), { recursive: true });
  await cp(join(repo, 'packages/agent-host/bundled-apps.json'), join(root, 'packages/agent-host/bundled-apps.json'));
  for (const name of ['assets', 'scanning']) {
    const sourceDirectory = join(repo, 'apps', name);
    const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
    await packageApp(sourceDirectory, join(root, 'apps', `${spec.manifest.id}-${spec.manifest.version}`));
  }
  return root;
}

test('ensureBundledApps installs and enables assets plus scanning once, and refuses builtin uninstall', async t => {
  const root = await stageRoot(t);
  const dataDir = join(root, 'data');
  const host = new AgentHost({ dataDir });
  t.after(() => host.close());
  const first = await host.ensureBundledApps({ root });
  assert.equal(first.missingManifest, false);
  assert.equal(first.items.length, 2);
  assert.deepEqual(first.items.map(item => item.id).sort(), ['org.oviraptor.assets', 'org.oviraptor.scanning']);
  for (const item of first.items) {
    assert.equal(item.installed, true);
    assert.equal(item.enabled, true);
    assert.equal(item.builtin, true);
    assert.equal(item.uninstallable, false);
    assert.equal(item.error, null);
  }
  const listed = host.apps.list();
  assert.equal(listed.length, 2);
  assert.ok(listed.every(app => app.enabled && app.builtin && app.uninstallable === false));
  assert.equal(listed.find(app => app.id === 'org.oviraptor.scanning').manifest.tools.some(tool => tool.name === 'task_status'), true);
  assert.equal(listed.find(app => app.id === 'org.oviraptor.scanning').manifest.tools.some(tool => tool.name === 'scan_start'), true);
  // Desktop declarations do not grant or enable execution in a standalone Host.
  assert.equal(host.apps.scanningTasks.available,false);

  const second = await host.ensureBundledApps({ root });
  assert.equal(second.items.length, 2);
  assert.equal(host.apps.list().length, 2);
  await assert.rejects(host.apps.uninstall('org.oviraptor.assets'), error => error.code === 'host_app_builtin');
  await assert.rejects(host.apps.uninstall('org.oviraptor.scanning'), error => error.code === 'host_app_builtin');

  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  const bundled = await (await fetch(`${web.origin}/api/apps/bundled`, { headers })).json();
  assert.equal(bundled.items.length, 2);
  assert.ok(bundled.items.every(item => item.enabled && item.status === 'enabled'));
  const apps = await (await fetch(`${web.origin}/api/apps`, { headers })).json();
  assert.equal(apps.items.length, 2);
});

test('ensureBundledApps records missing package errors without blocking an empty host', async t => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-bundled-missing-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'packages', 'agent-host'), { recursive: true });
  await writeFile(join(root, 'packages/agent-host/bundled-apps.json'), JSON.stringify({
    schemaVersion: 1,
    apps: [{ id: 'org.oviraptor.assets', package: 'apps/org.oviraptor.assets-1.0.0', enable: true, builtin: true, uninstallable: false }],
  }));
  const events = [];
  const host = new AgentHost({ dataDir: join(root, 'data'), onEvent: event => events.push(event) });
  t.after(() => host.close());
  const result = await host.ensureBundledApps({ root });
  assert.equal(result.items[0].installed, false);
  assert.equal(result.items[0].error, 'host_bundled_package_missing');
  assert.deepEqual(host.apps.list(), []);
  assert.ok(events.some(event => event.type === 'application.bundled_failed' && event.code === 'host_bundled_package_missing'));
  assert.equal(host.bundledApps().items[0].error, 'host_bundled_package_missing');
});


test('ensureBundledApps packages from apps/assets source when prebuilt package is missing', async t => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'oviraptor-bundled-source-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'packages', 'agent-host'), { recursive: true });
  await mkdir(join(root, 'apps'), { recursive: true });
  await cp(join(repo, 'packages/agent-host/bundled-apps.json'), join(root, 'packages/agent-host/bundled-apps.json'));
  for (const name of ['assets', 'scanning']) {
    await cp(join(repo, 'apps', name), join(root, 'apps', name), { recursive: true });
  }
  const host = new AgentHost({ dataDir: join(root, 'data') });
  t.after(() => host.close());
  const result = await host.ensureBundledApps({ root });
  assert.equal(result.missingManifest, false);
  assert.equal(result.items.length, 2);
  assert.ok(result.items.every(item => item.installed && item.enabled && item.error === null));
  assert.equal(host.apps.list().length, 2);
  const assetsVersion=JSON.parse(await readFile(join(repo,'apps/assets/app.package.json'),'utf8')).manifest.version;
  assert.ok(existsSync(join(root, '.agent-build', 'apps', `org.oviraptor.assets-${assetsVersion}`, 'manifest.json')));
  assert.ok(existsSync(join(root, '.agent-build', 'apps', 'org.oviraptor.scanning-1.4.0', 'manifest.json')));
});

test('POST /api/apps/bundled/ensure installs builtins and returns shell catalog', async t => {
  const root = await stageRoot(t);
  const host = new AgentHost({ dataDir: join(root, 'data') });
  t.after(() => host.close());
  const web = await serveHost(host); t.after(() => web.close());
  const headers = { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' };
  const response = await fetch(`${web.origin}/api/apps/bundled/ensure`, { method: 'POST', headers, body: JSON.stringify({ force: true }) });
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.missingManifest, false);
  assert.equal(body.items.length, 2);
  assert.ok(body.items.every(item => item.installed && item.enabled));
  assert.ok(Array.isArray(body.shell?.items));
  assert.equal(body.shell.items.find(item => item.id === 'asset').installed, true);
  assert.equal(body.shell.items.find(item => item.id === 'asset').enabled, true);
  assert.equal(body.shell.items.find(item => item.id === 'sentinel').installed, true);
});

test('only explicit bundled update upgrades an older Assets package and preserves its inventory in a separate generation', async t => {
  const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
  const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const old=await createDirectoryPackage({...spec,manifest:{...spec.manifest,version:'1.0.0'},sourceDirectory,outputDirectory:join(root,'older-assets')});
  await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);
  const project=host.store.createProject('Update test');
  const other=host.store.createProject('Other project');
  host.apps.setGrants(spec.manifest.id,project.id,['certificate.collect','inventory.read','inventory.write'],undefined,host.runtimeIdentity());
  host.apps.setGrants(spec.manifest.id,other.id,['inventory.exposure.read','inventory.read'],undefined,'previous-runtime');
  await host.apps.call(spec.manifest.id,'inventory_import',project.id,{source:'fixture',records:[{externalId:'kept',host:'kept.example.test'}]});
  const previous=host.store.application(spec.manifest.id);
  await host.ensureBundledApps({root});assert.equal(host.store.application(spec.manifest.id).version,'1.0.0');
  const previousGrants=host.store.grants(spec.manifest.id);
  const updated=await host.ensureBundledApps({root,force:true});
  assert.equal(updated.items.find(item=>item.id===spec.manifest.id).error,null);
  const current=host.store.application(spec.manifest.id);assert.equal(current.version,spec.manifest.version);assert.notEqual(current.data_path,previous.data_path);
  for (const saved of previousGrants) {
    const grant=host.store.grant(spec.manifest.id,saved.projectId);
    assert.deepEqual(grant.permissions,saved.permissions);
    assert.equal(grant.disclosureRuntime,saved.disclosureRuntime,'never rewrite an old runtime disclosure to the current runtime');
    assert.ok(grant.revision>saved.revision,'in-flight requests must still observe a revision fence');
  }
  assert.equal((await host.apps.call(spec.manifest.id,'inventory_stats',project.id,{})).structuredContent.total,1);
  assert.equal(host.store.db.prepare('SELECT COUNT(*) n FROM application_versions WHERE app_id=?').get(spec.manifest.id).n,1);
});

for (const change of ['added permission','changed permission contract','untrusted installed component']) test(`bundled update does not retain consent for ${change}`, async t => {
  const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
  const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const old=await createDirectoryPackage({...spec,manifest:{...spec.manifest,version:'1.0.0'},sourceDirectory,outputDirectory:join(root,'older-assets')});
  await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);
  const project=host.store.createProject('No implicit regrant');
  host.apps.setGrants(spec.manifest.id,project.id,['inventory.read'],undefined,host.runtimeIdentity());
  if(change!=='untrusted installed component')await host.ensureBundledApps({root});
  if(change!=='untrusted installed component'){
    const permissions=change==='added permission'?[...spec.manifest.permissions,{id:'inventory.future.read',description:'New, unapproved capability'}]
      :spec.manifest.permissions.map(permission=>permission.id==='inventory.read'?{...permission,description:'Changed disclosure contract'}:permission);
    const target=join(root,'apps',`${spec.manifest.id}-${spec.manifest.version}`);
    await rm(target,{recursive:true,force:true});
    await createDirectoryPackage({...spec,manifest:{...spec.manifest,permissions},sourceDirectory,outputDirectory:target});
  }
  const result=await host.ensureBundledApps({root,force:true});
  assert.equal(result.items.find(item=>item.id===spec.manifest.id).error,null);
  assert.deepEqual(host.store.grant(spec.manifest.id,project.id).permissions,[]);
  assert.equal(host.apps.capabilities(project.id,{runtimeIdentity:host.runtimeIdentity()}).length,0);
});

test('even a builtin-marked third-party bundle must renew its grants after an otherwise compatible update', async t => {
  const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
  const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const id='org.example.inventory',current={...spec.manifest,id,version:'2.0.0'};
  const old=await createDirectoryPackage({...spec,manifest:{...current,version:'1.0.0'},sourceDirectory,outputDirectory:join(root,'older-third-party')});
  await createDirectoryPackage({...spec,manifest:current,sourceDirectory,outputDirectory:join(root,'apps','third-party')});
  await writeFile(join(root,'packages/agent-host/bundled-apps.json'),JSON.stringify({schemaVersion:1,apps:[{id,package:'apps/third-party',enable:true,builtin:true,uninstallable:false}]}));
  await host.apps.install(old.directory);await host.apps.setEnabled(id,true);
  const project=host.store.createProject('Third-party');
  host.apps.setGrants(id,project.id,['inventory.read'],undefined,host.runtimeIdentity());
  await host.ensureBundledApps({root});assert.equal(host.store.application(id).builtin,true);
  const result=await host.ensureBundledApps({root,force:true});
  assert.equal(result.items[0].error,null);assert.equal(host.store.application(id).version,'2.0.0');
  assert.deepEqual(host.store.grant(id,project.id).permissions,[]);
});

test('an explicit grant revocation during bundled re-enable wins over a compatible retention snapshot', async t => {
  const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
  const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
  const old=await createDirectoryPackage({...spec,manifest:{...spec.manifest,version:'1.0.0'},sourceDirectory,outputDirectory:join(root,'older-assets')});
  await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);
  const project=host.store.createProject('Revoke wins');
  host.apps.setGrants(spec.manifest.id,project.id,['inventory.read'],undefined,host.runtimeIdentity());
  await host.ensureBundledApps({root});
  const enable=host.apps.setEnabled.bind(host.apps);
  host.apps.setEnabled=async(id,value)=>{
    const result=await enable(id,value);
    if(id===spec.manifest.id&&value&&host.store.application(id).version===spec.manifest.version){
      const current=host.store.grant(id,project.id);
      host.apps.setGrants(id,project.id,[],current.revision,host.runtimeIdentity());
    }
    return result;
  };
  const result=await host.ensureBundledApps({root,force:true});
  assert.equal(result.items.find(item=>item.id===spec.manifest.id).error,'host_grant_revision_conflict');
  assert.deepEqual(host.store.grant(spec.manifest.id,project.id).permissions,[],'never revive explicitly revoked permissions');
});

test('Assets skipping the known discovery release preserves original grants without adding paid permission in Ask mode',async t=>{
 const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
 const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 assert.notEqual(spec.manifest.version,'1.2.5');
 const oldManifest={...spec.manifest,version:'1.2.5',permissions:spec.manifest.permissions.filter(permission=>permission.id!=='inventory.discovery.run')};
 const old=await createDirectoryPackage({...spec,manifest:oldManifest,sourceDirectory,outputDirectory:join(root,'older-assets')});
 await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);
 const project=host.store.createProject('Ask permissions retained'),other=host.store.createProject('Prior runtime retained');
 host.apps.setGrants(spec.manifest.id,project.id,['certificate.collect','inventory.read','inventory.write'],undefined,host.runtimeIdentity());
 host.apps.setGrants(spec.manifest.id,other.id,['inventory.exposure.read','inventory.read'],undefined,'previous-runtime');
 await host.ensureBundledApps({root});const before=host.store.grants(spec.manifest.id);
 const updated=await host.ensureBundledApps({root,force:true});assert.equal(updated.items.find(item=>item.id===spec.manifest.id).error,null);
 assert.equal(host.store.application(spec.manifest.id).version,spec.manifest.version);assert.equal(host.accessPolicy.view(project.id).mode,'ask');
 for(const saved of before){const current=host.store.grant(spec.manifest.id,saved.projectId);assert.deepEqual(current.permissions,saved.permissions);assert.equal(current.disclosureRuntime,saved.disclosureRuntime);assert.equal(current.permissions.includes('inventory.discovery.run'),false);assert.ok(current.revision>saved.revision);}
 const app=host.store.application(spec.manifest.id),effective=host.apps.effectiveGrant(app,project.id,host.runtimeIdentity());assert.equal(effective.permissions.includes('inventory.discovery.run'),false);
 assert.equal((await host.apps.call(app.id,'inventory_stats',project.id,{})).structuredContent.total,0);
});

test('known discovery addition cannot revive an explicit revocation during re-enable',async t=>{
 const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
 const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const old=await createDirectoryPackage({...spec,manifest:{...spec.manifest,version:'1.2.5',permissions:spec.manifest.permissions.filter(permission=>permission.id!=='inventory.discovery.run')},sourceDirectory,outputDirectory:join(root,'older-assets')});
 await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);const project=host.store.createProject('Revoke known upgrade');host.apps.setGrants(spec.manifest.id,project.id,['inventory.read'],undefined,host.runtimeIdentity());await host.ensureBundledApps({root});
 const enable=host.apps.setEnabled.bind(host.apps);host.apps.setEnabled=async(id,on)=>{const result=await enable(id,on);if(id===spec.manifest.id&&on&&host.store.application(id).version===spec.manifest.version){const current=host.store.grant(id,project.id);host.apps.setGrants(id,project.id,[],current.revision,host.runtimeIdentity());}return result;};
 const updated=await host.ensureBundledApps({root,force:true});assert.equal(updated.items.find(item=>item.id===spec.manifest.id).error,'host_grant_revision_conflict');assert.deepEqual(host.store.grant(spec.manifest.id,project.id).permissions,[]);
});

test('audited 1.2.11 exposure upgrade preserves old grants and full snapshots without granting new detail or execution categories',async t=>{
 const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
 const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const added=new Set(['inventory.exposure.details.read','inventory.exposure.run']);
 const oldManifest={...spec.manifest,version:'1.2.11',permissions:spec.manifest.permissions.filter(p=>!added.has(p.id)),tools:spec.manifest.tools.filter(t=>!t.name.startsWith('exposure_')||t.name==='exposure_overview')};
 const old=await createDirectoryPackage({...spec,manifest:oldManifest,sourceDirectory,outputDirectory:join(root,'older-assets')});
 await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);const p=host.store.createProject('Existing permissions');
 host.apps.setGrants(spec.manifest.id,p.id,['inventory.read','inventory.exposure.read'],undefined,host.runtimeIdentity());await host.ensureBundledApps({root});
 const policy=host.accessPolicy.view(p.id);host.accessPolicy.set({projectId:p.id,mode:'full',expectedRevision:policy.revision,expectedRuntimeIdentity:policy.runtimeIdentity,operatorConfirmed:true});
 const before=host.store.grant(spec.manifest.id,p.id),snapshot=host.accessPolicy.view(p.id);
 const update=await host.ensureBundledApps({root,force:true});assert.equal(update.items.find(i=>i.id===spec.manifest.id).error,null);
 const after=host.store.grant(spec.manifest.id,p.id);assert.deepEqual(after.permissions,before.permissions);assert.equal(after.disclosureRuntime,before.disclosureRuntime);assert.deepEqual(host.accessPolicy.view(p.id),snapshot);
 const effective=host.apps.effectiveGrant(host.store.application(spec.manifest.id),p.id,host.runtimeIdentity());for(const permission of added)assert(!effective.permissions.includes(permission));
});

test('1.2.12 exposure patch upgrade keeps existing detail grants and full policy unchanged',async t=>{
 const root=await stageRoot(t),host=new AgentHost({dataDir:join(root,'data')});t.after(()=>host.close());
 const sourceDirectory=join(repo,'apps/assets'),spec=JSON.parse(await readFile(join(sourceDirectory,'app.package.json'),'utf8'));
 const old=await createDirectoryPackage({...spec,manifest:{...spec.manifest,version:'1.2.12'},sourceDirectory,outputDirectory:join(root,'prior-exposure-patch')});
 await host.apps.install(old.directory);await host.apps.setEnabled(spec.manifest.id,true);const p=host.store.createProject('Existing detail permission');
 host.apps.setGrants(spec.manifest.id,p.id,['inventory.read','inventory.exposure.read','inventory.exposure.details.read'],undefined,host.runtimeIdentity());await host.ensureBundledApps({root});
 const policy=host.accessPolicy.view(p.id);host.accessPolicy.set({projectId:p.id,mode:'full',expectedRevision:policy.revision,expectedRuntimeIdentity:policy.runtimeIdentity,operatorConfirmed:true});
 const before=host.store.grant(spec.manifest.id,p.id),snapshot=host.accessPolicy.view(p.id);
 const update=await host.ensureBundledApps({root,force:true});assert.equal(update.items.find(i=>i.id===spec.manifest.id).error,null);
 const after=host.store.grant(spec.manifest.id,p.id);assert.deepEqual(after.permissions,before.permissions);assert.equal(after.disclosureRuntime,before.disclosureRuntime);assert.deepEqual(host.accessPolicy.view(p.id),snapshot);
});
