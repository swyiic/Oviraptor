const short=name=>String(name).split(/::|__/).at(-1);
function body(result){try{const v=JSON.parse(result.content);return v.structuredContent??v;}catch{return null;}}
function receipt(result){return result.ok&&!result.advisory&&!result.truncated?body(result):null;}
const control=name=>/^discovery_cleanup_(start|pause|resume|restore)$/.test(short(name));
const id=value=>typeof value==='string'&&value.length>0&&value.length<=128&&!/[\u0000-\u001f\u007f]/u.test(value);
export function cleanupTurnState(intent,listed,results){
 const relevant=results.filter(r=>short(r.name).startsWith('discovery_cleanup_'));
 const mutations=relevant.filter(r=>control(r.name));
 const last=results.findLastIndex(r=>control(r.name));
 const uncertain=mutations.findLast(r=>body(r)?.outcomeUnknown===true);
 const operation=uncertain&&body(uncertain)?.operationId;
 const checked=last>=0&&results.some((r,i)=>i>last&&short(r.name)==='discovery_cleanup_status');
 let bound;
 for(const r of [...relevant].reverse()){
  const v=receipt(r);if(!v)continue;
  bound=[v.runId,v.run?.id,v.receipt?.runId,v.operation?.result?.runId].find(id);if(bound)break;
 }
 let allowed=[];
 if(mutations.length)allowed=checked?[]:['discovery_cleanup_status'];
 else if(intent.action==='status')allowed=relevant.length?[]:['discovery_cleanup_status'];
 else if(intent.action==='start')allowed=['discovery_cleanup_start'];
 else if(bound)allowed=['discovery_cleanup_'+intent.action];
 else allowed=relevant.some(r=>short(r.name)==='discovery_cleanup_status')?[]:['discovery_cleanup_status'];
 const tools=listed.filter(t=>allowed.includes(short(t._hostName))).map(t=>{
  if(id(operation)&&short(t._hostName)==='discovery_cleanup_status')return{...t,function:{...t.function,parameters:{...t.function.parameters,required:['operationId'],properties:{operationId:{type:'string',enum:[operation]}}}}};
  if(!bound||!/^discovery_cleanup_(pause|resume|restore)$/.test(short(t._hostName)))return t;
  const parameters=t.function.parameters;
  return{...t,function:{...t.function,parameters:{...parameters,properties:{...parameters.properties,runId:{type:'string',enum:[bound]}}}}};
 });
 return{tools,bound,operation:id(operation)?operation:undefined,completed:!tools.length,attempted:mutations.length>0};
}
export function unresolvedCleanupControlNote(results){
 const uncertain=results.findLast(r=>control(r.name)&&body(r)?.outcomeUnknown===true);
 if(!uncertain)return null;
 const operation=body(uncertain).operationId;
 const matched=results.slice(results.indexOf(uncertain)+1).some(r=>{
  const v=receipt(r);return short(r.name)==='discovery_cleanup_status'&&id(operation)&&v?.operationId===operation&&v.outcomeUnknown===false&&v.reconciliation?.operationId===operation;
 });
 return matched?null:`种子清洗控制的执行结果未知；已保留原操作回执，本轮没有自动重发。${id(operation)?`操作编号 ${operation}。`:''}当前最新任务不自动等于这次请求的结算结果，不能断言没有执行或已经完成。`;
}
