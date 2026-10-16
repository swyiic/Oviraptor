import test,{afterEach} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {ASSET_ACTIONS} from '../ui/operations.mjs';
import {bindAssetsClient,clearAssetsClient,invoke,openAssetUrl,subscribeAssetsArtifacts,downloadAssetsArtifact} from '../ui/ports/client.mjs';
import {assetLocalStorage,assetSessionStorage} from '../ui/ports/storage.mjs';
const context={hostProjectId:5,nativeProjectId:9001,name:'Bound native project',status:'active'};
const bind=(handler=async()=>null,hostProjectId=5)=>bindAssetsClient({context:{projectId:hostProjectId},callAction:handler},context);
afterEach(clearAssetsClient);
test('the finite page action surface preserves all 84 reviewed old Assets IPC contracts',async()=>{
 const ledger=JSON.parse(await readFile(new URL('../contracts/legacy-ipc-contracts.json',import.meta.url),'utf8'));
 const commands=ledger.commands.map(item=>item.command);
 assert.equal(commands.length,84);assert.equal(new Set(ASSET_ACTIONS).size,ASSET_ACTIONS.length);
 assert.deepEqual([...ASSET_ACTIONS].sort(),[...commands,'assets_context','list_asset_job_profiles','list_config_profiles','save_config_profile','delete_config_profile','import_asset_collection_configuration','read_asset_artifact','saved_assets_stats','saved_assets_search'].sort());
});
test('the disconnected page rejects reads and never invents inventory',async()=>{
 await assert.rejects(invoke('list_assets',{query:{projectId:9001}}),{code:'assets_page_not_connected'});
});
test('native and Host project identities must be explicitly bound',()=>{
 assert.throws(()=>bind(undefined,9001),{code:'assets_context_invalid'});bind();
});
test('unknown native commands are refused before the client is called',async()=>{
 let calls=0;bind(async()=>calls++);await assert.rejects(invoke('read_arbitrary_file',{path:'/tmp/any'}),{code:'assets_action_not_declared'});assert.equal(calls,0);
});
test('optional argument omission follows desktop JSON without changing raw result DTOs',async()=>{
 const wire=[];let answer=null;bind(async(name,args)=>{wire.push([name,args]);return answer;});
 assert.equal(await invoke('list_logs',{runId:undefined,limit:20,projectId:9001}),null);
 answer=7;assert.equal(await invoke('update_decision',{assetId:4,decision:'confirmed'}),7);
 answer={items:[{id:4,projectId:9001,extra:{proof:'raw'}}],total:1,memberTotal:2};assert.strictEqual(await invoke('list_assets',{query:{projectId:9001,responseRepresentativeAssetId:4}}),answer);
 assert.deepEqual(wire[0],['list_logs',{limit:20,projectId:9001}]);
 await assert.rejects(invoke('import_targets',{projectId:9001,lines:[undefined]}),{code:'assets_arguments_invalid'});
});
test('every known query and mutation scope location refuses another native project',async()=>{
 let calls=0;bind(async()=>calls++);
 for(const args of [{projectId:5},{input:{projectId:5}},{query:{projectId:5}},{request:{query:{projectId:5}}},{input:{rule:{projectId:5}}},{expectedProjectId:5},{selections:[{projectId:5,assetId:1}]},{input:{selections:[{projectId:5,assetId:1}]}}])await assert.rejects(invoke('list_assets',args),{code:'assets_project_scope_changed'});
 assert.equal(calls,0);
});
test('evidence data remains opaque while its explicit project envelope is checked',async()=>{
 let parameters;bind(async(_name,args)=>{parameters=args;return null;});
 await invoke('save_exposure_finding',{input:{projectId:9001,evidence:{projectId:'provider-owned text'}}});
 assert.equal(parameters.input.evidence.projectId,'provider-owned text');
});
test('late native replies cannot publish into a rebound or closed page',async()=>{
 let resolve;bind(()=>new Promise(done=>{resolve=done;}));const pending=invoke('list_assets',{query:{projectId:9001}});bind();resolve({items:[{id:1}],total:1});await assert.rejects(pending,{code:'assets_page_context_changed'});
});
test('external URLs remain explicitly unavailable until the controlled host channel is connected',async()=>{
 await assert.rejects(openAssetUrl('https://example.invalid'),{code:'assets_external_open_not_connected'});
});
test('opaque page preferences do not access browser storage and stay in separate areas',()=>{
 const previous=Object.getOwnPropertyDescriptor(globalThis,'localStorage');Object.defineProperty(globalThis,'localStorage',{configurable:true,get(){assert.fail('opaque browser storage accessed');}});
 try{assetLocalStorage.setItem('width',320);assert.equal(assetLocalStorage.getItem('width'),'320');assert.equal(assetSessionStorage.getItem('width'),null);assetLocalStorage.removeItem('width');assert.equal(assetLocalStorage.getItem('width'),null);}finally{if(previous)Object.defineProperty(globalThis,'localStorage',previous);else delete globalThis.localStorage;}
});
test('an export issues one opaque receipt and download retry never repeats the export mutation',async()=>{
 const id='9ce58d7e-11a2-4dc9-8fe0-4b143fc37749';let exports=0,downloads=0,items;
 bindAssetsClient({context:{projectId:5},callAction:async()=>{exports++;return {path:`oviraptor-asset-export:${id}`,rows:2};},downloadArtifact:async(name,artifactId,options)=>{downloads++;assert.equal(name,'read_asset_artifact');assert.equal(artifactId,id);assert.deepEqual(options,{filename:'assets.csv'});if(downloads===1)throw Error('synthetic transport failure');return {downloadStarted:true,totalBytes:42,sha256:'a'.repeat(64)};}},context);
 const off=subscribeAssetsArtifacts(value=>{items=value;});try{const raw=await invoke('export_assets',{request:{query:{projectId:9001},fields:['host'],chineseHeaders:true}});assert.equal(raw.rows,2);assert.equal(items[0].state,'ready');await assert.rejects(downloadAssetsArtifact(id),/synthetic transport failure/);assert.equal(items[0].state,'failed');const receipt=await downloadAssetsArtifact(id);assert.equal(receipt.downloadStarted,true);assert.equal(items[0].state,'started');assert.equal(exports,1);assert.equal(downloads,2);}finally{off();}
});
test('raw paths and a different artifact ID never become browser download authority',async()=>{
 bind(async()=>({path:'/private/arbitrary.csv',rows:1}));await assert.rejects(invoke('export_assets'),{code:'assets_export_receipt_invalid'});
 bind(async()=>({path:'oviraptor-asset-export:9ce58d7e-11a2-4dc9-8fe0-4b143fc37749',artifactId:'different',rows:1}));await assert.rejects(invoke('export_assets'),{code:'assets_export_receipt_invalid'});
});
test('project rebinding drops export UI authority and rejects a late download receipt',async()=>{
 const id='9ce58d7e-11a2-4dc9-8fe0-4b143fc37749';let done;
 bindAssetsClient({context:{projectId:5},callAction:async()=>({path:`oviraptor-asset-export:${id}`,rows:1}),downloadArtifact:()=>new Promise(resolve=>{done=resolve;})},context);
 await invoke('export_assets');const pending=downloadAssetsArtifact(id);bind();done({downloadStarted:true,totalBytes:1,sha256:'a'.repeat(64)});await assert.rejects(pending,{code:'assets_page_context_changed'});await assert.rejects(downloadAssetsArtifact(id),{code:'assets_export_receipt_invalid'});
});
