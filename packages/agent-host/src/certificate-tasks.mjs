// Trusted Host -> desktop certificate executor. Endpoint and token never enter model/worker context.
import { request } from 'node:http';
import { hostError } from './store.mjs';
export const CERTIFICATE_TOOLS = new Map([
 ['certificate_task_start','start'],['certificate_task_status','status'],['certificate_task_pause','pause'],['certificate_task_resume','resume'],
]);
export class CertificateTasks {
 constructor({origin=process.env.OVIRAPTOR_CERTIFICATE_BRIDGE_ORIGIN,token=process.env.OVIRAPTOR_CERTIFICATE_BRIDGE_TOKEN,tools=CERTIFICATE_TOOLS,route='/certificate-task',kind='certificate'}={}){
  this.tools=tools;this.route=route;this.kind=kind;
  try{const url=new URL(origin);if(url.protocol==='http:'&&url.hostname==='127.0.0.1'&&url.port&&url.pathname==='/'&&!url.username&&!url.password&&!url.search&&!url.hash&&/^[a-f0-9]{64}$/.test(token)){this.origin=url.origin;this.token=token;}}catch{}
 }
 get available(){return Boolean(this.origin&&this.token);}
 async call(store,projectId,name,args,operationId){
  const prefix='host_'+this.kind;
  if(!this.available||!this.tools.has(name))throw hostError(prefix+'_executor_unavailable',503);
  const project=store.project(projectId),desktopId=Number(project.externalId);
  if(!Number.isSafeInteger(desktopId)||desktopId<=0)throw hostError(prefix+'_project_unmapped',409);
  const body=JSON.stringify({projectId:desktopId,action:this.tools.get(name),args,operationId});
  if(Buffer.byteLength(body)>32*1024)throw hostError(prefix+'_input_too_large',400);
  const result=await new Promise((resolve,reject)=>{
   const readOnly=['certificate_task_status','scan_list','scan_status','scan_preflight','scan_results'].includes(name);
   const fail=()=>reject(hostError(prefix+(readOnly?'_executor_unavailable':'_task_outcome_unknown'),503,{outcomeUnknown:!readOnly,retryable:false}));
   const req=request(this.origin+this.route,{method:'POST',headers:{Authorization:`Bearer ${this.token}`,'Content-Type':'application/json','Content-Length':Buffer.byteLength(body)},timeout:15000},response=>{
    let chunks=[],size=0;response.on('data',chunk=>{size+=chunk.length;if(size>128*1024){response.destroy();fail();}else chunks.push(chunk)});
    response.on('error',fail);response.on('end',()=>{try{const value=JSON.parse(Buffer.concat(chunks).toString('utf8'));if(!value.ok){const reason=String(value.error||'task_rejected').slice(0,500);if(name==='scan_results'&&reason==='scan_record_unavailable'){reject(hostError('host_scan_record_unavailable',404,{outcomeUnknown:false,retryable:false}));return}if(this.kind==='scan'&&['scan_financial_continuation_required','scan_financial_reconciliation_required','scan_financial_history_unavailable'].includes(reason)){reject(hostError('host_'+reason,409,{outcomeUnknown:false,retryable:false}));return}const knownBeforeExecution=new Set(['scan_project_unavailable','scan_arguments_invalid','scan_asset_unavailable','scan_asset_url_invalid','scan_task_invalid','scan_task_unavailable','scan_channel_web_only','scan_action_unavailable','scan_operation_invalid','scan_operation_conflict']);reject(hostError(prefix+'_task_rejected',409,{reason,retryable:false,outcomeUnknown:!knownBeforeExecution.has(reason)}));return}resolve(value.result);}catch{fail();}});
   });req.on('timeout',()=>{req.destroy();fail()});req.on('error',fail);req.end(body);
  });
  if(result?.projectId!==desktopId)throw hostError(prefix+'_project_binding_changed',409,{retryable:false});
  const scoped={...result,projectId,hostBoundProjectId:projectId,desktopProjectId:desktopId,
    projectBinding:{hostProjectId:projectId,desktopProjectId:desktopId,source:'host_saved_external_id'}};
  return {content:[{type:'text',text:JSON.stringify(scoped)}],structuredContent:scoped};
 }
}
