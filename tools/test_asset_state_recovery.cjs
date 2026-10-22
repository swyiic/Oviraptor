const test=require('node:test');
const {assert,flush,deferred,mountAssetAccuracy}=require('./asset_accuracy_ui_harness.cjs');
const table='src/features/assets/components/AssetWorkspace.vue';
const workspace='src/features/assets/workspace/AssetsPluginWorkspace.vue';
const certificates='src/features/assets/components/QuarantineCertificateReview.vue';
const projects=[{id:91,name:'Fixture project',status:'active'}];
const page=()=>({items:[],total:0,memberTotal:0,summary:{all:0,pending:0,uncertain:0,confirmed:0,rejected:0,notApplicable:0,sentToAgent:0}});
const stats=()=>({assetCount:1,aliveCount:0,pendingCount:0,blockedCount:1,runningJobs:0,projectCount:1});
const batch=()=>({items:[],scanned:3,nextCursor:3,hasMore:false,roots:['fixture.example.test']});
const storage=entries=>({getItem:key=>entries[key]??null,setItem(key,value){entries[key]=String(value);}});
async function mount(t,file,props,options={}) {
 const h=await mountAssetAccuracy(file,props,options);
 t.after(()=>{h.unmount();assert.deepEqual(h.warnings,[]);});return h;
}

for(const saved of ['{','{}','"not an array"','null','42','{"schemaVersion":999,"columns":["host"]}'])test(`invalid saved column shape ${saved} falls back without a blank asset page`,async t=>{
 const h=await mount(t,table,{projects,selectedProjectId:91},{host:{localStorage:storage({'asset-columns':saved,'asset-column-widths':'{'})},commands:{list_assets:async()=>page()}});
 assert(h.calls.some(([name])=>name==='list_assets'));
 assert(h.state().visibleColumns.value.includes('host'));
 assert.equal(h.state().columnWidths.value.host,230);
 assert.equal(h.state().readFailed.value,false);
});

test('legacy columns migrate without losing valid choices, and widths accept only bounded known numeric fields',async t=>{
 const h=await mount(t,table,{projects,selectedProjectId:91},{host:{localStorage:storage({'asset-columns':'["projectName","title","host","host",123,"unknown"]','asset-column-widths':'{"host":190,"title":99999,"port":-1,"ip":"180","unknown":200,"__proto__":120}'})},commands:{list_assets:async()=>page()}});
 const columns=h.state().visibleColumns.value;
 assert(columns.includes('assetSources'));assert(columns.includes('title'));assert(columns.includes('host'));
 assert.equal(columns.filter(key=>key==='host').length,1);
 assert.equal(columns.some(key=>typeof key!=='string'||key==='unknown'||key==='projectName'),false);
 assert.equal(h.state().columnWidths.value.host,190);
 assert.equal(h.state().columnWidths.value.title,280);
 assert.equal(h.state().columnWidths.value.port,76);
 assert.equal(h.state().columnWidths.value.ip,140);
 assert.equal(Object.hasOwn(h.state().columnWidths.value,'unknown'),false);
});

test('browser storage read/write denial cannot block mounting, filtering or changing columns',async t=>{
 const blocked={getItem(){throw Error('fixture storage denied');},setItem(){throw Error('fixture quota exceeded');}};
 const h=await mount(t,table,{projects,selectedProjectId:91},{host:{localStorage:blocked,sessionStorage:blocked},commands:{list_assets:async()=>page()}});
 assert.equal(h.state().readFailed.value,false);
 h.state().toggleColumn('note');h.state().saveColumnWidths();await flush();
 assert.equal(await h.state().refresh(),true);
 assert.match(h.text(),/preferences could not be saved/);
 assert.equal(h.calls.filter(([name])=>name==='list_assets').length,2);
});

test('the real quarantine toolbar waits for both visible table and prior certificate-review refresh',async t=>{
 const tableHold=deferred(),certificateHold=deferred();let tableReads=0,certReads=0;
 const h=await mount(t,workspace,{projects,profiles:[],projectId:91,view:'quarantine',refreshToken:0,scanningEnabled:false},{commands:{
  dashboard_stats:async()=>stats(),list_assets:()=>++tableReads===1?page():tableHold.promise,
  review_quarantined_certificates:()=>++certReads===1?batch():certificateHold.promise,
 }});
 await h.state(certificates).scan();await flush();
 assert.equal(h.state(certificates).complete.value,true);
 let done=false;const pending=h.matching('data-refresh-quarantine')[0].props.onClick().then(value=>{done=true;return value;});
 await flush();assert.equal(done,false);
 assert.equal(tableReads,2);assert.equal(certReads,2);
 tableHold.resolve(page());await flush();assert.equal(done,false);
 certificateHold.resolve(batch());assert.equal(await pending,true);
 assert.equal(h.state().readError.value,'');
 assert.equal(h.state(certificates).scanned.value,3,'refresh replaces the previous review instead of appending stale candidates');
});

test('a failed visible asset refresh cannot be reported as successful merely because dashboard stats loaded',async t=>{
 let fail=false;
 const h=await mount(t,workspace,{projects,profiles:[],projectId:91,view:'quarantine',refreshToken:0,scanningEnabled:false},{commands:{dashboard_stats:async()=>stats(),list_assets:async()=>{if(fail)throw Error('fixture asset read failed');return page();}}});
 fail=true;assert.equal(await h.state().refreshQuarantine(),false);
 assert.match(h.state().readError.value,/Quarantine asset list/);
 assert.equal(h.state(table).readFailed.value,true);
 fail=false;assert.equal(await h.state().refreshQuarantine(),true);
 assert.equal(h.state().readError.value,'');
});

test('an untouched certificate workbench stays manual when the surrounding page refreshes',async t=>{
 const h=await mount(t,workspace,{projects,profiles:[],projectId:91,view:'quarantine',refreshToken:0,scanningEnabled:false},{commands:{dashboard_stats:async()=>stats(),list_assets:async()=>page()}});
 assert.equal(await h.state().refreshQuarantine(),true);
 assert.equal(h.calls.some(([name])=>name==='review_quarantined_certificates'),false);
 assert.equal(h.state(certificates).scanned.value,0);
});
