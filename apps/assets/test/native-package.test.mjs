import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,mkdtemp,rm,writeFile,symlink} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {createNativeManifest} from '../native-manifest.mjs';
import {packageNativeAssets,verifyAssetsUiBuild} from '../package-native.mjs';
import {validateManifest,validateActionArguments,validateToolArguments,validatePackageDirectory,ARTIFACT_READ_SCHEMA} from '../../../packages/app-sdk/src/index.mjs';
const complete=()=>{const m=createNativeManifest();m.files=[m.entry,'page.html'].map(path=>({path,sha256:'0'.repeat(64)}));return m;};
test('native Assets preserves all 84 operator names in a closed 101-action SDK2 package',async()=>{
 const m=complete(),legacy=JSON.parse(await readFile(new URL('../contracts/legacy-ipc-contracts.json',import.meta.url),'utf8'));validateManifest(m);assert.equal(m.schemaVersion,2);assert.equal(m.actions.length,101);assert.equal(new Set(m.actions.map(a=>a.name)).size,101);for(const item of legacy.commands)assert.equal(m.actions.filter(a=>a.name===item.command).length,1,item.command);assert.deepEqual(m.tools.map(t=>t.name).sort(),['saved_assets_search','saved_assets_stats']);
});
test('global directory, rules and credential administration are operator application actions only',()=>{
 const m=complete();for(const name of ['save_project_ownership_directory','get_asset_rule_catalog','add_content_rule','list_config_profiles','save_config_profile','delete_config_profile','import_asset_collection_configuration']){const a=m.actions.find(a=>a.name===name);assert.equal(a.scope,'application',name);assert(!m.tools.some(t=>t.name===name));}
 assert.equal(m.actions.find(a=>a.name==='authorize_asset_ownership').permission,'inventory.authorization.manage');assert.equal(m.actions.find(a=>a.name==='start_exposure_scan').permission,'inventory.exposure.run');
});
test('original nullable observation/unit arguments stay typed and no JSON string bypass is accepted',()=>{
 const m=complete();const remediation=m.actions.find(a=>a.name==='save_exposure_remediation').inputSchema;validateActionArguments(remediation,{projectId:9001,findingId:1,observationId:null,runId:1,actor:'human',note:'synthetic',status:'open'});
 assert.throws(()=>validateActionArguments(remediation,JSON.stringify({projectId:9001})));const incident=m.actions.find(a=>a.name==='act_on_exposure_incident').inputSchema;assert.throws(()=>validateActionArguments(incident,{projectId:9001,action:{arbitraryCommand:'no'}}));
});
test('complete settings are closed, preserve explicit null and do not accept arbitrary paths or unknown policy',()=>{
 const schema=complete().actions.find(a=>a.name==='save_config_profile').inputSchema;validateActionArguments(schema,{input:{name:'Saved',settings:{fofaKey:'',runProbe:false,modelProfiles:null}}});assert.throws(()=>validateActionArguments(schema,{input:{name:'Saved',settings:{futureUnknown:true}}}));assert.throws(()=>validateActionArguments(schema,{input:{name:'Saved',settings:{configPath:'/private/arbitrary'}}}));
});
test('export delivery uses exactly the SDK fixed project-read schema and never becomes a model tool',()=>{
 const m=complete(),a=m.actions.find(a=>a.name==='read_asset_artifact');assert.equal(a.delivery,'artifact-read');assert.equal(a.scope,'project');assert.equal(a.permission,'inventory.export');assert.deepEqual(a.inputSchema,ARTIFACT_READ_SCHEMA);validateActionArguments(a.inputSchema,{artifactId:'synthetic-id',offset:0,maxBytes:196608});assert.throws(()=>validateActionArguments(a.inputSchema,{artifactId:'synthetic-id',offset:0,maxBytes:196609}));assert(!m.tools.some(t=>t.name===a.name));
});
test('model tools retain small bounded search inputs while operator bulk actions accept reviewed larger lists',()=>{
 const m=complete(),tool=m.tools.find(t=>t.name==='saved_assets_search');validateToolArguments(tool.inputSchema,{filter:{query:'literal',deletedView:'active'},limit:20});assert.throws(()=>validateToolArguments(tool.inputSchema,{limit:21}));const a=m.actions.find(a=>a.name==='update_asset_decisions');validateActionArguments(a.inputSchema,{input:{selections:Array.from({length:129},(_,i)=>({projectId:9001,assetId:i+1})),decision:'confirmed',note:''}});
});
test('native packaging contains the pinned inline page and binary without running or modifying the retained v1 application',async t=>{
 const root=await mkdtemp(join(tmpdir(),'assets-native-packaging-'));t.after(()=>rm(root,{recursive:true,force:true}));
 const previous=await readFile(new URL('../app.package.json',import.meta.url));const outputDirectory=join(root,'package');
 // A packaging-only fixture. This does not establish that the Assets bin runs.
 await packageNativeAssets({binary:process.execPath,outputDirectory});
 const verified=await validatePackageDirectory(outputDirectory);assert.equal(verified.manifest.schemaVersion,2);assert.equal(verified.manifest.runtime.kind,'native');assert.equal(verified.manifest.actions.length,101);assert.deepEqual(verified.manifest.files.map(f=>f.path).sort(),[verified.manifest.entry,'page.html'].sort());
 assert.deepEqual(await readFile(join(outputDirectory,'page.html')),await readFile(new URL('../ui/dist/page.html',import.meta.url)));assert.deepEqual(await readFile(new URL('../app.package.json',import.meta.url)),previous);
 await writeFile(join(outputDirectory,'page.html'),'tampered page');await assert.rejects(validatePackageDirectory(outputDirectory),{code:'app_package_hash_mismatch'});
});
test('native package rejects symlink binary and leaves its destination absent',async t=>{
 if(process.platform==='win32')return;const root=await mkdtemp(join(tmpdir(),'assets-native-alias-'));t.after(()=>rm(root,{recursive:true,force:true}));await symlink(process.execPath,join(root,'alias'));
 await assert.rejects(packageNativeAssets({binary:join(root,'alias'),outputDirectory:join(root,'package')}),/native_package_binary_invalid/);await assert.rejects(readFile(join(root,'package','manifest.json')),{code:'ENOENT'});
});
test('the actual Assets author build is self-contained and every recorded input still matches its SHA',async()=>{
 const build=await verifyAssetsUiBuild(),html=await readFile(new URL('../ui/dist/page.html',import.meta.url),'utf8');assert.equal(build.externalScripts,0);assert.equal(build.externalStyles,0);assert.equal(/<script[^>]+(?:src=|type="module")/iu.test(html),false);assert.equal(/<link[^>]+stylesheet/iu.test(html),false);assert.ok(html.includes('oviraptor.app-ui.v2'));
});
