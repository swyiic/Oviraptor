// Trusted Host -> desktop certificate executor. Endpoint and token never enter model/worker context.
import { request } from 'node:http';
import { hostError } from './store.mjs';
export const CERTIFICATE_TOOLS = new Map([
 ['certificate_task_start','start'],['certificate_task_status','status'],['certificate_task_pause','pause'],['certificate_task_resume','resume'],
]);
// Compatibility with older desktop bridges. Only exact, audited pre-admission
// validations are safe to retry; transport and arbitrary database errors stay unknown.
const KNOWN_NOT_STARTED = new Set([
 'scan_project_unavailable','scan_arguments_invalid','scan_asset_unavailable','scan_asset_url_invalid','scan_task_invalid','scan_task_unavailable','scan_channel_web_only','scan_action_unavailable','scan_operation_invalid','scan_operation_conflict',
 'certificate_arguments_invalid','certificate_action_invalid','certificate_operation_invalid','certificate_limits_invalid','certificate_selection_invalid','certificate_selection_outside_project',
 '请先在新建查询中确认允许补采的域名范围',
]);
export class CertificateTasks {
 constructor({origin=process.env.OVIRAPTOR_CERTIFICATE_BRIDGE_ORIGIN,token=process.env.OVIRAPTOR_CERTIFICATE_BRIDGE_TOKEN,tools=CERTIFICATE_TOOLS,route='/certificate-task',kind='certificate',maxInputBytes=32*1024,maxResponseBytes=128*1024,requestTimeoutMs=15000,readOnlyTools=['discovery_enrichment_status','certificate_task_status','scan_list','scan_status','scan_preflight','scan_recovery','scan_results']}={}){
  this.tools=tools;this.route=route;this.kind=kind;
  this.maxInputBytes=Math.min(256*1024,Math.max(1,maxInputBytes));this.maxResponseBytes=Math.min(1024*1024,Math.max(1,maxResponseBytes));
  this.requestTimeoutMs=Number.isFinite(requestTimeoutMs)?Math.min(120000,Math.max(1,requestTimeoutMs)):15000;
  this.readOnlyTools=new Set(readOnlyTools);
  try{const url=new URL(origin);if(url.protocol==='http:'&&url.hostname==='127.0.0.1'&&url.port&&url.pathname==='/'&&!url.username&&!url.password&&!url.search&&!url.hash&&/^[a-f0-9]{64}$/.test(token)){this.origin=url.origin;this.token=token;}}catch{}
 }
 get available(){return Boolean(this.origin&&this.token);}
 async call(store,projectId,name,args,operationId){
  const prefix='host_'+this.kind;
  if(!this.available||!this.tools.has(name))throw hostError(prefix+'_executor_unavailable',503,{outcomeUnknown:false,phase:'not_started',operationId,retryable:false});
  const project=store.project(projectId),desktopId=Number(project.externalId);
  if(!Number.isSafeInteger(desktopId)||desktopId<=0)throw hostError(prefix+'_project_unmapped',409,{outcomeUnknown:false,phase:'not_started',operationId,retryable:false});
  const body=JSON.stringify({projectId:desktopId,action:this.tools.get(name),args,operationId});
  if(Buffer.byteLength(body)>this.maxInputBytes)throw hostError(prefix+'_input_too_large',400,{outcomeUnknown:false,phase:'not_started'});
  const result=await new Promise((resolve,reject)=>{
   const readOnly=this.readOnlyTools.has(name);
   const fail=error=>{const notStarted=readOnly||error?.code==='ECONNREFUSED';reject(hostError(prefix+(notStarted?'_executor_unavailable':'_task_outcome_unknown'),503,{outcomeUnknown:!notStarted,phase:notStarted?'not_started':'unknown',operationId,retryable:false}));};
   const req=request(this.origin+this.route,{method:'POST',headers:{Authorization:`Bearer ${this.token}`,'Content-Type':'application/json','Content-Length':Buffer.byteLength(body)},timeout:this.requestTimeoutMs},response=>{
    let chunks=[],size=0;response.on('data',chunk=>{size+=chunk.length;if(size>this.maxResponseBytes){response.destroy();fail();}else chunks.push(chunk)});
    response.on('error',fail);response.on('end',()=>{try{const value=JSON.parse(Buffer.concat(chunks).toString('utf8'));if(!value.ok){const reason=String(value.error||'task_rejected').slice(0,500);if(name==='scan_results'&&reason==='scan_record_unavailable'){reject(hostError('host_scan_record_unavailable',404,{reason,outcomeUnknown:false,phase:'not_started',operationId,retryable:false}));return}if(this.kind==='scan'&&['scan_financial_continuation_required','scan_financial_reconciliation_required','scan_financial_history_unavailable'].includes(reason)){reject(hostError('host_'+reason,409,{reason,outcomeUnknown:false,phase:'not_started',operationId,retryable:false}));return}const notStarted=KNOWN_NOT_STARTED.has(reason)||(value.phase==='not_started'&&value.outcomeUnknown===false);reject(hostError(prefix+'_task_rejected',409,{reason,operationId,retryable:false,phase:notStarted?'not_started':'unknown',outcomeUnknown:!readOnly&&!notStarted}));return}resolve(value.result);}catch{fail();}});
   });req.on('timeout',()=>{req.destroy();fail()});req.on('error',fail);req.end(body);
  });
  if(result?.projectId!==desktopId)throw hostError(prefix+'_project_binding_changed',409,{retryable:false});
  const scoped={...result,projectId,hostBoundProjectId:projectId,desktopProjectId:desktopId,
    projectBinding:{hostProjectId:projectId,desktopProjectId:desktopId,source:'host_saved_external_id'}};
  return {content:[{type:'text',text:JSON.stringify(scoped)}],structuredContent:scoped};
 }
}
