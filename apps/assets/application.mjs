import { createHash } from 'node:crypto';
import { context, exact, text, check, open, operation, result, scope } from './sdk/sqlite-data.mjs';
import { legacyRead } from './sdk/legacy-bridge-stub.mjs';
const ID='org.oviraptor.assets';

const SEVERITY_KEYS=new Set(['critical','high','medium','low','info']);
const STATUS_KEYS=new Set(['new','reviewing','confirmed','dismissed']);
function bump(map,key,known){const k=typeof key==='string'&&key.trim()?key.trim():'unknown';if(known&&!known.has(k))map.unknown=(map.unknown??0)+1;else map[k]=(map[k]??0)+1;}
function asFinite(v){const n=Number(v);return Number.isFinite(n)?n:null;}
function exposureAggregate(project,summaryRaw,runsRaw){
 const bySeverity={critical:0,high:0,medium:0,low:0,info:0,unknown:0};
 const byStatus={new:0,reviewing:0,confirmed:0,dismissed:0,unknown:0};
 const runByStatus={};
 let available=false,code=null,findingsTotal=0,eligibleAssets=0,highRisk=0,runsTotal=0;
 const unwrap=raw=>{
  if(!raw||typeof raw!=='object')return null;
  if(raw.available===false)return {unavailable:true,code:typeof raw.code==='string'?raw.code:'legacy_managed_bridge_unavailable'};
  if(raw.structuredContent&&typeof raw.structuredContent==='object'&&!Array.isArray(raw.structuredContent))return raw.structuredContent;
  return raw;
 };
 const summary=unwrap(summaryRaw);
 if(summary?.unavailable){code=summary.code;}
 else if(summary){
  available=true;
  const total=asFinite(summary.total);if(total!==null)findingsTotal=total;
  const eligible=asFinite(summary.eligibleAssets??summary.eligible_assets);if(eligible!==null)eligibleAssets=eligible;
  const hr=asFinite(summary.highRisk??summary.high_risk);if(hr!==null)highRisk=hr;
  const nc=asFinite(summary.newCount??summary.new_count);if(nc!==null)byStatus.new=nc;
  const rv=asFinite(summary.reviewing);if(rv!==null)byStatus.reviewing=rv;
  const cf=asFinite(summary.confirmed);if(cf!==null)byStatus.confirmed=cf;
  const ds=asFinite(summary.dismissed);if(ds!==null)byStatus.dismissed=ds;
  const sev=summary.bySeverity??summary.by_severity;
  if(sev&&typeof sev==='object'&&!Array.isArray(sev)){
   for(const key of Object.keys(bySeverity))bySeverity[key]=0;
   for(const [k,v] of Object.entries(sev)){const n=asFinite(v);if(n!==null){if(SEVERITY_KEYS.has(k))bySeverity[k]=n;else bySeverity.unknown+=n;}}
  }
  // If a list was mistakenly supplied, count severity/status only and drop every row field.
  const rows=Array.isArray(summary.items)?summary.items:Array.isArray(summary.findings)&&summary.findings.every(x=>x&&typeof x==='object'&&!Array.isArray(x))?summary.findings:null;
  if(rows){
   findingsTotal=rows.length;
   for(const key of Object.keys(bySeverity))bySeverity[key]=0;
   for(const key of Object.keys(byStatus))byStatus[key]=0;
   highRisk=0;
   for(const row of rows){
    bump(bySeverity,row.severity,SEVERITY_KEYS);
    bump(byStatus,row.status,STATUS_KEYS);
    if(SEVERITY_KEYS.has(row.severity)&&['critical','high'].includes(row.severity)&&row.status!=='dismissed')highRisk++;
   }
  }
 }
 const runs=unwrap(runsRaw);
 if(runs?.unavailable){if(!available)code=runs.code??code;}
 else if(runs){
  available=true;
  const list=Array.isArray(runs)?runs:Array.isArray(runs.items)?runs.items:Array.isArray(runs.runs)?runs.runs:null;
  if(list){
   runsTotal=list.length;
   for(const row of list){if(row&&typeof row==='object'&&!Array.isArray(row))bump(runByStatus,row.status);}
  }else{
   const n=asFinite(runs.total??runs.count);if(n!==null)runsTotal=n;
   const rbs=runs.byStatus??runs.by_status;
   if(rbs&&typeof rbs==='object'&&!Array.isArray(rbs)){for(const [k,v] of Object.entries(rbs)){const n=asFinite(v);if(n!==null)runByStatus[k]=n;}}
  }
 }
 return{
  ...scope(project),
  kind:'exposure_overview',
  available,
  code:available?null:(code??'legacy_managed_bridge_unavailable'),
  runs:{total:runsTotal,byStatus:runByStatus},
  findings:{total:findingsTotal,bySeverity,byStatus,eligibleAssets,highRisk},
  unit:'exposure_aggregate_counts',
  coverage:'host_bound_project_saved_exposure_only',
  ownership:'unreviewed',
  authorization:'not_granted',
  schema:{fields:['runs.total','runs.byStatus','findings.total','findings.bySeverity','findings.byStatus','findings.eligibleAssets','findings.highRisk'],forbidden:['payload','evidence','credentials','probeRules','rawResponse','requestContent','items']},
  note:'Aggregate exposure counts for the host-bound project only. Never includes payload, evidence excerpts, probe rules, credentials or raw responses. Unavailable bridge yields empty counts with code.'
 };
}
const fields=['externalId','host','link','ip','port','protocol','domain','title','statusCode','probeOutcome','observedAt'];
const bounds={externalId:128,host:256,link:1024,ip:80,port:8,protocol:32,domain:256,title:512,statusCode:16,probeOutcome:64,observedAt:120};
function record(raw){exact(raw,fields);const clean={};for(const key of fields)clean[key]=text(raw[key]??'',bounds[key],{empty:key!=='externalId'});check(clean.host||clean.link,'asset_locator_required');if(clean.link){let url;try{url=new URL(clean.link);}catch{throw Error('asset_link_invalid');}check(url.hostname&&!url.username&&!url.password,'asset_link_invalid');}return clean;}
const hash=v=>createHash('sha256').update(JSON.stringify(v)).digest('hex');
export function createApplication(manifest,{legacyBridge}={}){let db,dataDir;
 const requireDb=ctx=>{check(db,'app_not_started');return context(ctx);};
 const count=project=>db.prepare('SELECT COUNT(*) AS n FROM inventory WHERE project_id=?').get(project).n;
 const onStart=async input=>{dataDir=input.dataDir;db=open(dataDir,'inventory.sqlite3');db.exec(`CREATE TABLE IF NOT EXISTS inventory(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,source TEXT NOT NULL,external_id TEXT NOT NULL,observation_json TEXT NOT NULL,received_at TEXT NOT NULL,UNIQUE(project_id,source,external_id));`);return{dataSchemaVersion:1};};
 const onStop=async()=>{db?.close();db=undefined;return{busy:false};};
 const tools={
  inventory_import:async(args,ctx)=>{requireDb(ctx);exact(args,['source','records']);const source=text(args.source,128);check(Array.isArray(args.records)&&args.records.length>0&&args.records.length<=128);const records=args.records.map(record);check(new Set(records.map(r=>r.externalId)).size===records.length,'asset_duplicate_external_id');return result(operation(db,ctx,'inventory_import',args,project=>{const receivedAt=new Date().toISOString();const write=db.prepare('INSERT INTO inventory(project_id,source,external_id,observation_json,received_at) VALUES(?,?,?,?,?) ON CONFLICT(project_id,source,external_id) DO UPDATE SET observation_json=excluded.observation_json,received_at=excluded.received_at');for(const value of records)write.run(project,source,value.externalId,JSON.stringify(value),receivedAt);return{...scope(project),imported:records.length,total:count(project),unit:'app_inventory_records',identity:'explicit_source_and_external_id_no_endpoint_automerge',ownership:'unreviewed',authorization:'not_granted'};}));},
  inventory_stats:async(args,ctx)=>{const project=requireDb(ctx);exact(args,[]);const byProbe={unclassified:0,known:0,unknown:0},byHttp={informational:0,success:0,redirect:0,clientError:0,serverError:0,unclassified:0,unknown:0};const known=new Set(['alive_clean','web_alive','web_restricted','browser_render_required','virtual_host_required','web_abnormal','tcp_alive_non_http','blocked_content','unreachable','skipped']);for(const row of db.prepare('SELECT observation_json FROM inventory WHERE project_id=?').iterate(project)){const value=JSON.parse(row.observation_json);byProbe[value.probeOutcome?(known.has(value.probeOutcome)?'known':'unknown'):'unclassified']++;const code=value.statusCode;byHttp[!code?'unclassified':/^[1-5][0-9]{2}$/.test(code)?['informational','success','redirect','clientError','serverError'][Number(code[0])-1]:'unknown']++;}return result({...scope(project),total:count(project),unit:'app_inventory_records',byProbe,byHttp,ownership:'unreviewed',authorization:'not_granted',coverage:'explicitly_imported_records_not_provider_completeness'});},
  inventory_search:async(args,ctx)=>{const project=requireDb(ctx);exact(args,['query','limit','cursor']);const query=text(args.query??'',120,{empty:true}).trim(),limit=args.limit??20;check(Number.isSafeInteger(limit)&&limit>=1&&limit<=20);const filterHash=hash({project,query,order:'id_asc'});let after=0;if(args.cursor!==undefined){try{check(typeof args.cursor==='string'&&args.cursor.startsWith('inventory-page:')&&args.cursor.length<=2048);const cursor=JSON.parse(Buffer.from(args.cursor.slice(15),'base64url'));check(cursor.filterHash===filterHash&&Number.isSafeInteger(cursor.after)&&cursor.after>0,'asset_cursor_scope_changed');after=cursor.after;}catch{throw Error('asset_cursor_invalid');}}// Exact literal totals still require one project-scoped SQLite scan. Keep only
    // the requested page in JS, rather than retaining the whole matching inventory.
    const queryLower=query.toLowerCase(),items=[];let total=0,remainingCount=0;
    for(const row of db.prepare('SELECT * FROM inventory WHERE project_id=? ORDER BY id').iterate(project)){
      const value=JSON.parse(row.observation_json);
      if(query&&![row.source,...Object.values(value)].some(v=>String(v).toLowerCase().includes(queryLower)))continue;
      total++;
      if(row.id<=after)continue;
      remainingCount++;
      if(items.length<limit)items.push({id:row.id,source:row.source,...value,receivedAt:row.received_at});
    }
    let value;do{const hasMore=remainingCount>items.length;value={...scope(project),items,total,returned:items.length,hasMore,nextCursor:hasMore&&items.length?'inventory-page:'+Buffer.from(JSON.stringify({filterHash,after:items.at(-1).id})).toString('base64url'):null,order:'id_asc',unit:'app_inventory_records',searchSemantics:'literal_imported_metadata_substring'};if(Buffer.byteLength(JSON.stringify(value))<=24*1024)break;items.pop();}while(items.length);check(items.length>0||remainingCount===0,'app_result_too_large');return result(value);},
  inventory_schema:async(args,ctx)=>{const project=requireDb(ctx);exact(args,[]);return result({...scope(project),source:'static_whitelist',note:'Static column descriptions for this app inventory view. Does not query sqlite_master or any live schema.',tables:[{name:'inventory',purpose:'Project-scoped imported asset observations',columns:[{name:'id',type:'integer',role:'row_id'},{name:'project_id',type:'integer',role:'host_bound_project'},{name:'source',type:'string',role:'import_source',maxLength:128},{name:'external_id',type:'string',role:'replacement_key',maxLength:128},{name:'observation_json',type:'object',fields:[{name:'externalId',type:'string',maxLength:128},{name:'host',type:'string',maxLength:256},{name:'link',type:'string',maxLength:1024},{name:'ip',type:'string',maxLength:80},{name:'port',type:'string',maxLength:8},{name:'protocol',type:'string',maxLength:32},{name:'domain',type:'string',maxLength:256},{name:'title',type:'string',maxLength:512},{name:'statusCode',type:'string',maxLength:16},{name:'probeOutcome',type:'string',maxLength:64},{name:'observedAt',type:'string',maxLength:120}]},{name:'received_at',type:'string',role:'import_timestamp'}],filterFields:['query','source','host','domain','protocol','statusCode','probeOutcome'],queryTools:['inventory_search','inventory_query','inventory_stats','project_overview']}],ownership:'unreviewed',authorization:'not_granted'});},
  inventory_query:async(args,ctx)=>{const project=requireDb(ctx);exact(args,['filter','limit','cursor']);const allowed=['query','source','host','domain','protocol','statusCode','probeOutcome'];const filterRaw=args.filter??{};check(filterRaw===undefined||filterRaw===null||(typeof filterRaw==='object'&&!Array.isArray(filterRaw)),'asset_filter_invalid');const filter={};if(filterRaw&&typeof filterRaw==='object'){for(const key of Object.keys(filterRaw)){check(allowed.includes(key),'asset_filter_field_denied');filter[key]=text(filterRaw[key]??'',key==='query'?120:128,{empty:true}).trim();}}const limit=args.limit??20;check(Number.isSafeInteger(limit)&&limit>=1&&limit<=100);const filterHash=hash({project,filter,order:'id_asc',tool:'inventory_query'});let after=0;if(args.cursor!==undefined){try{check(typeof args.cursor==='string'&&args.cursor.startsWith('inventory-query:')&&args.cursor.length<=2048);const cursor=JSON.parse(Buffer.from(args.cursor.slice(16),'base64url'));check(cursor.filterHash===filterHash&&Number.isSafeInteger(cursor.after)&&cursor.after>0,'asset_cursor_scope_changed');after=cursor.after;}catch{throw Error('asset_cursor_invalid');}}const items=[];let total=0,remainingCount=0;const match=(row,value)=>{if(filter.source&&row.source.toLowerCase()!==filter.source.toLowerCase())return false;if(filter.query){const q=filter.query.toLowerCase();if(![row.source,...Object.values(value)].some(v=>String(v).toLowerCase().includes(q)))return false;}for(const key of['host','domain','protocol','statusCode','probeOutcome']){if(filter[key]&&String(value[key]??'').toLowerCase()!==filter[key].toLowerCase())return false;}return true;};for(const row of db.prepare('SELECT * FROM inventory WHERE project_id=? ORDER BY id').iterate(project)){const value=JSON.parse(row.observation_json);if(!match(row,value))continue;total++;if(row.id<=after)continue;remainingCount++;if(items.length<limit)items.push({id:row.id,source:row.source,...value,receivedAt:row.received_at});}let value;do{const hasMore=remainingCount>items.length;value={...scope(project),items,total,returned:items.length,hasMore,nextCursor:hasMore&&items.length?'inventory-query:'+Buffer.from(JSON.stringify({filterHash,after:items.at(-1).id})).toString('base64url'):null,order:'id_asc',unit:'app_inventory_records',filterFields:allowed,searchSemantics:'whitelist_filter_literal_match',limitMax:100};if(Buffer.byteLength(JSON.stringify(value))<=48*1024)break;items.pop();}while(items.length);check(items.length>0||remainingCount===0,'app_result_too_large');return result(value);},
  project_overview:async(args,ctx)=>{const project=requireDb(ctx);exact(args,[]);const byProbe={unclassified:0,known:0,unknown:0},byHttp={informational:0,success:0,redirect:0,clientError:0,serverError:0,unclassified:0,unknown:0};const known=new Set(['alive_clean','web_alive','web_restricted','browser_render_required','virtual_host_required','web_abnormal','tcp_alive_non_http','blocked_content','unreachable','skipped']);const sources=new Map();for(const row of db.prepare('SELECT source,observation_json FROM inventory WHERE project_id=?').iterate(project)){sources.set(row.source,(sources.get(row.source)??0)+1);const value=JSON.parse(row.observation_json);byProbe[value.probeOutcome?(known.has(value.probeOutcome)?'known':'unknown'):'unclassified']++;const code=value.statusCode;byHttp[!code?'unclassified':/^[1-5][0-9]{2}$/.test(code)?['informational','success','redirect','clientError','serverError'][Number(code[0])-1]:'unknown']++;}const legacy=await legacyRead(legacyBridge,ID,'saved_assets_stats',{},ctx);return result({...scope(project),kind:'project_overview',total:count(project),unit:'app_inventory_records',byProbe,byHttp,bySource:[...sources.entries()].map(([source,n])=>({source,count:n})).sort((a,b)=>a.source.localeCompare(b.source)),legacySnapshot:{available:legacy?.available===true,code:legacy?.code??null},readTools:['inventory_schema','inventory_query','inventory_stats','inventory_search','project_overview'],ownership:'unreviewed',authorization:'not_granted',coverage:'app_owned_imported_records_plus_optional_legacy_snapshot_flag',note:'Overview of the host-bound project inventory only. Not provider completeness or live reachability.'});},
  exposure_overview:async(args,ctx)=>{const project=requireDb(ctx);exact(args,[]);const summary=await legacyRead(legacyBridge,ID,'exposure_summary',{},ctx);const runs=await legacyRead(legacyBridge,ID,'list_exposure_runs',{},ctx);return result(exposureAggregate(project,summary,runs));},
 };
 // Not on the public manifest surface until a host-managed legacyBridge is injected.
 if(typeof legacyBridge==='function'){
  tools.legacy_snapshot_stats=async(args,ctx)=>{requireDb(ctx);exact(args,[]);return result(await legacyRead(legacyBridge,ID,'saved_assets_stats',{},ctx));};
 }
 return{manifest,onStart,onStop,tools};
}
