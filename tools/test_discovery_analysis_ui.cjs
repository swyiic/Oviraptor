// Production Vue setup + synthetic IPC. No live database or provider requests.
const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const {loadPure}=require('./shell_module_harness.cjs');
const file='src/features/assets/components/DiscoveryWorkbench.vue';
const candidate=host=>({host,assetCount:2,assetIds:[3,7],sources:['certificate_name','cname'],companies:['中国移动'],fingerprints:['a'.repeat(64)],inConfirmedScope:false,ownershipConfirmed:false,hunterAuthorized:false});
const report=(p=1)=>({projectId:p,projectName:'移动资产',organization:'中国移动',roots:[],scannedAssets:70013,matchedAssets:2,certificateAssets:1,candidateTotal:1,candidates:[candidate('one.example.com')],previewTruncated:false,note:'Local observations only'});
async function mount(t,commands={},props={projectId:1}){const h=await mountAssetAccuracy(file,props,{events:['analyze','confirmed','changed'],commands:{get_asset_discovery_page:async()=>({roots:[],seeds:[],settings:{keyConfigured:true,contractVerified:true}}),...commands}});t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;}
test('AI refinement reads all local observations and emits a bounded draft without saving scope, sending or querying Hunter',async t=>{
 const h=await mount(t,{analyze_local_discovery:async args=>{assert.deepEqual(args,{projectId:1,organization:'中国移动'});return report();}});
 h.state().organization.value='中国移动';await h.state().analyzeSeeds();await flush();
 assert.deepEqual(h.calls.map(c=>c[0]),['get_asset_discovery_page','analyze_local_discovery']);
 const event=h.events.find(e=>e[0]==='analyze');assert.equal(event[1].analysis.scannedAssets,70013);assert.match(event[1].prompt,/不要联网、调用 Hunter/);assert.match(event[1].prompt,/资产 ID/);
 assert.equal(h.state().confirmedRelationship.value,false);assert.equal(h.state().selectedCandidates.value.length,0);
 h.state().selectedCandidates.value=['one.example.com'];await h.state().confirmCandidates();assert.equal(h.calls.some(c=>c[0]==='confirm_local_discovery'),false);
});
test('review confirmation passes exact candidates, original roots and explicit consent, then clears review',async t=>{
 let confirmed;const h=await mount(t,{confirm_local_discovery:async args=>{confirmed=args.input;return{roots:['one.example.com']};}},{projectId:1,review:report()});
 h.state().selectedCandidates.value=['one.example.com'];h.state().confirmedRelationship.value=true;await h.state().confirmCandidates();await flush();
 assert.deepEqual(confirmed,{projectId:1,organization:'中国移动',hosts:['one.example.com'],expectedRoots:[],userConfirmed:true});assert.equal(h.state().analysis.value,undefined);
 assert.match(h.text(),/尚未启动采集、探测或 Hunter/);assert.equal(h.events.filter(e=>e[0]==='confirmed').length,1);
 assert.equal(h.calls.some(c=>/hunter|send|probe/.test(c[0])),false);
});
test('late analysis cannot move another project to chat and editing the company invalidates old suggestions',async t=>{
 const waiting=deferred();const h=await mount(t,{analyze_local_discovery:()=>waiting.promise});h.state().organization.value='中国移动';const analyzing=h.state().analyzeSeeds();h.props.projectId=2;await flush();waiting.resolve(report());await analyzing;await flush();
 assert.equal(h.events.some(e=>e[0]==='analyze'),false);assert.equal(h.state().analysis.value,undefined);
 h.props.review=report(2);await flush();assert.equal(h.state().analysis.value.projectId,2);h.state().organization.value='另一单位';await flush();assert.equal(h.state().analysis.value,undefined);assert.equal(h.state().confirmedRelationship.value,false);
});
test('prompt caps the actual UTF-8 payload and explicitly reports its limited AI preview',()=>{
 const value=report();value.candidates=Array.from({length:80},(_,i)=>({...candidate(`host-${i}.example.com`),companies:['移动'.repeat(160)],sources:['certificate_name','cname','observed_host'],assetIds:[i*2+1,i*2+2]}));value.candidateTotal=30000;value.previewTruncated=true;
 const prompt=loadPure('src/features/assets/discoveryAnalysis.ts').discoveryAnalysisPrompt(value);
 assert.ok(Buffer.byteLength(prompt)<16384);const json=JSON.parse(prompt.slice(prompt.lastIndexOf('\n')+1));assert.ok(json.candidates.length>0&&json.candidates.length<80);assert.equal(json.aiPreviewCount,json.candidates.length);assert.match(prompt,new RegExp(`包含 ${json.aiPreviewCount} 个候选，共发现 30000 个`));assert.equal(json.candidates.every(c=>c.ownershipConfirmed===false&&c.hunterAuthorized===false),true);
});
