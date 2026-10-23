const short=name=>String(name).split(/::|__/).at(-1);
function receipt(result){if(!result.ok||result.truncated||result.advisory)return null;try{const v=JSON.parse(result.content);return v.structuredContent??v;}catch{return null;}}
/** Narrow a single requested control to control -> one status -> answer.
 * Tool filtering is never permission; AppManager and the native owner recheck it.
 */
export function exposureTurnTools(intent,goal,listed,results){
 const relevant=results.filter(r=>/^exposure_(?:task_|findings$|finding$|runs$|overview$)/.test(short(r.name)));
 const mutations=relevant.filter(r=>/^exposure_task_(start|resume|cancel)$/.test(short(r.name)));
 const explicit=String(goal).match(/(?:#|任务\s*[#：:]?\s*)([1-9][0-9]*)/u)?.[1];
 const stated=explicit?Number(explicit):undefined;
 let bound;
 for(const r of [...relevant].reverse()){
  const v=receipt(r);if(!v)continue;
  const ids=[v.runId,v.id,v.latest?.id,v.run?.id].filter(Number.isSafeInteger);
  const id=stated?ids.find(id=>id===stated):ids[0];if(id){bound=id;break;}
 }
 const last=results.findLastIndex(r=>mutations.includes(r));
 const checked=mutations.length&&results.some((r,i)=>i>last&&short(r.name)==='exposure_task_status'&&receipt(r));
 let allowed=[];
 if(intent.action==='review')allowed=relevant.length<6?['exposure_findings','exposure_finding','exposure_runs','exposure_task_status','exposure_overview','inventory_stats','inventory_query']:[];
 else if(!checked&&relevant.filter(r=>!r.ok).length<2){
  if(mutations.length)allowed=relevant.filter(r=>short(r.name)==='exposure_task_status'&&results.indexOf(r)>last).length?[]:['exposure_task_status'];
  else if(intent.action==='status')allowed=relevant.length?[]:['exposure_task_status'];
  else if(intent.action==='start')allowed=['exposure_task_start'];
  else if(bound)allowed=['exposure_task_'+intent.action];
  else allowed=relevant.length<2?['exposure_task_status']:[];
 }
 const target=bound??stated;
 return listed.filter(t=>allowed.includes(short(t._hostName))).map(t=>{
  if(!target||!/^exposure_task_(resume|cancel|status)$/.test(short(t._hostName)))return t;
  const params=t.function.parameters;
  return{...t,function:{...t.function,parameters:{...params,required:['runId'],properties:{runId:{type:'integer',enum:[target]}}}}};
 });
}
