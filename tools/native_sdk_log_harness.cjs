const fs=require('node:fs'),path=require('node:path'),ts=require('typescript'),vue=require('vue');
const {liveLogHarness}=require('./live_runner_log_harness.cjs');
const flush=async()=>{for(let i=0;i<12;i++)await vue.nextTick();};
function load(relative,imports={}){const exports={};const source=fs.readFileSync(path.join(__dirname,'../',relative),'utf8');
 new Function('require','exports','document',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText)(name=>{
  if(name==='vue')return vue;if(name in imports)return imports[name];throw Error(`Unexpected SDK log import ${name}`);
 },exports,imports.document);return exports;}
const contract=()=>load('src/features/sentinel/execution/nativeSdkLogContract.ts');
const owner=(changes={})=>({ownerId:'a'.repeat(64),domain:'root',dispatchKey:'b'.repeat(64),scanId:'A',attempt:1,rootRunId:'root-A',runId:'root-A',assignmentId:null,leaseAttemptId:'11111111-1111-4111-8111-111111111111',workerId:null,round:1,requestHash:'c'.repeat(64),...changes});
const row=(sequence=1,changes={})=>({...owner(),sequence,ordinal:1,stage:'prepared',costPhase:'',terminalState:'',time:'committed',...changes});
const page=(rows=[],changes={})=>({schemaVersion:2,channel:'native_sdk',scanId:'A',attempt:1,ownerId:null,requestedAfterSequence:0,resetCursor:false,afterSequence:rows.at(-1)?.sequence??0,latestSequence:rows.at(-1)?.sequence??0,more:false,available:true,rows,owners:[owner()],ownersTruncated:false,gaps:[],gapsTruncated:false,incompleteOwners:['a'.repeat(64)],incompleteOwnersTruncated:false,...changes});
const deferred=()=>{let resolve;const promise=new Promise(r=>{resolve=r;});return {promise,resolve};};
async function mount(read,options={}){
 options.channel='nest-native-sdk-log';const live=liveLogHarness(options),calls=[];let hidden=false;
 const module=load('src/features/sentinel/execution/useNativeSdkLog.ts',{document:{get hidden(){return hidden;}},
  '../api':{sentinelApi:{readNativeSdkLog:async(...args)=>{calls.push(args);return read(...args);}}},
  '../../../composables/useCommittedRefresh':live.committed,'./nativeSdkLogContract':contract()});
 const selection=vue.reactive({scanId:'A',attempt:1,active:true,ownerId:undefined});let state;
 const renderer=vue.createRenderer({createComment:()=>({}),insert(){},remove(){},parentNode:()=>null,nextSibling:()=>null});
 let tree,app;
 if(options.view){
  const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc'),relative='src/features/sentinel/components/NativeSdkLogView.vue';
  const filename=path.join(__dirname,'../',relative),{descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
  const script=compileScript(descriptor,{id:'sdk-view'}),template=compileTemplate({source:descriptor.template.content,filename,id:'sdk-view',compilerOptions:{bindingMetadata:script.bindings}});
  if(template.errors.length)throw Error(JSON.stringify(template.errors));
  const compile=(source,imports={})=>{const exports={};new Function('require','exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText)(name=>name==='vue'?vue:imports[name],exports);return exports;};
  const view=compile(script.content,{'../execution/useNativeSdkLog':{useNativeSdkLog:(...args)=>{state=module.useNativeSdkLog(...args);return state;}}});
  const rendered=compile(template.code),node=(kind,text='')=>({kind,text,children:[]});
  const viewRenderer=vue.createRenderer({createElement:kind=>node(kind),createText:text=>node('text',text),createComment:text=>node('comment',text),
   insert(child,parent,anchor){if(child.parent){const prior=child.parent.children.indexOf(child);if(prior>=0)child.parent.children.splice(prior,1);}child.parent=parent;const n=anchor?parent.children.indexOf(anchor):-1;parent.children.splice(n<0?parent.children.length:n,0,child);},
   remove(child){const n=child.parent?.children.indexOf(child);if(n>=0)child.parent.children.splice(n,1);},setElementText(element,text){element.text=text;element.children=[];},setText(element,text){element.text=text;},
   parentNode:child=>child.parent,nextSibling:child=>child.parent?.children[child.parent.children.indexOf(child)+1],patchProp(){}});
  tree=node('root');app=viewRenderer.createApp({...view.default,render:rendered.render},{scanId:'A',attempt:1,active:true});app.mount(tree);
 }else{
  app=renderer.createApp({setup(){state=module.useNativeSdkLog(()=>selection.active&&selection.scanId?{scanId:selection.scanId,attempt:selection.attempt,ownerId:selection.ownerId}:undefined);return()=>null;}});app.mount({});
 }
 await flush();
 const textTree=n=>n?`${n.kind==='comment'?'':n.text} ${n.children.map(textTree).join(' ')}`:'';
 return {state,live,calls,selection,text:()=>textTree(tree),visibility(value){hidden=value;live.visibility(value);},unmount:()=>app.unmount()};
}
module.exports={load,contract,owner,row,page,deferred,mount,flush};
