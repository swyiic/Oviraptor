const fs=require('node:fs'),path=require('node:path'),ts=require('typescript'),vue=require('vue');
const {liveLogHarness}=require('./live_runner_log_harness.cjs');
const flush=async()=>{for(let i=0;i<12;i++)await vue.nextTick();};
function load(relative,imports={}){
  const exports={};const source=fs.readFileSync(path.join(__dirname,'../',relative),'utf8');
  new Function('require','exports','document',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText)(name=>{
    if(name==='vue')return vue;if(name in imports)return imports[name];throw Error(`Unexpected import ${name}`);
  },exports,imports.document);return exports;
}
const contract=()=>load('src/features/sentinel/execution/nativeProcessLogContract.ts');
const row=(sequence=1,overrides={})=>({scanId:'A',attempt:1,branch:'source',dispatchClaimId:'11111111-1111-4111-8111-111111111111',invocationKey:'a'.repeat(64),executionId:'22222222-2222-4222-8222-222222222222',stage:'semgrep:step-0',sequence,stream:'stderr',streamSequence:sequence,message:'same',gap:false,time:'committed',...overrides});
const page=(rows=[],overrides={})=>({schemaVersion:1,scanId:'A',attempt:1,requestedAfterSequence:0,resetCursor:false,rows,more:false,afterSequence:rows.at(-1)?.sequence||0,available:true,executionState:null,latestSequence:rows.at(-1)?.sequence||0,...overrides});
const deferred=()=>{let resolve;const promise=new Promise(r=>{resolve=r;});return{promise,resolve};};
async function mount(read,options={}){
  options.channel='nest-native-process-log';const live=liveLogHarness(options),calls=[];
  // Production composable and contract execute unchanged; only transport/time
  // are fixtures. This is frontend state evidence, never backend/UI acceptance.
  const document={get hidden(){return hidden;}};let hidden=false;
  const module=load('src/features/sentinel/execution/useNativeProcessLog.ts',{
    document,'../api':{sentinelApi:{readNativeProcessLog:async(...args)=>{calls.push(args);return read(...args);}}},
    '../../../composables/useCommittedRefresh':live.committed,'./nativeProcessLogContract':contract(),
  });
  const selection=vue.reactive({scanId:'A',attempt:1,active:true});let state;
  const renderer=vue.createRenderer({createComment:()=>({}),insert(){},remove(){},parentNode:()=>null,nextSibling:()=>null});
  const app=renderer.createApp({setup(){state=module.useNativeProcessLog(()=>selection.active&&selection.scanId?{scanId:selection.scanId,attempt:selection.attempt}:undefined);return()=>null;}});
  app.mount({});await flush();
  const setVisibility=value=>{hidden=value;live.visibility(value);};
  return {state,live,calls,selection,visibility:setVisibility,unmount:()=>app.unmount()};
}
module.exports={load,contract,row,page,flush,deferred,mount};
