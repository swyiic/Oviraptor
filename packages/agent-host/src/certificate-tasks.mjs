// Trusted Host -> desktop certificate executor. Endpoint and token never enter model/worker context.
import { request } from 'node:http';
import { hostError } from './store.mjs';
export const CERTIFICATE_TOOLS = new Map([
 ['certificate_task_start','start'],['certificate_task_status','status'],['certificate_task_pause','pause'],['certificate_task_resume','resume'],
]);
export class CertificateTasks {
 constructor({origin=process.env.OVIRAPTOR_CERTIFICATE_BRIDGE_ORIGIN,token=process.env.OVIRAPTOR_CERTIFICATE_BRIDGE_TOKEN}={}){
  try{const url=new URL(origin);if(url.protocol==='http:'&&url.hostname==='127.0.0.1'&&url.port&&url.pathname==='/'&&!url.username&&!url.password&&!url.search&&!url.hash&&/^[a-f0-9]{64}$/.test(token)){this.origin=url.origin;this.token=token;}}catch{}
 }
 get available(){return Boolean(this.origin&&this.token);}
 async call(store,projectId,name,args,operationId){
  if(!this.available||!CERTIFICATE_TOOLS.has(name))throw hostError('host_certificate_executor_unavailable',503);
  const project=store.project(projectId),desktopId=Number(project.externalId);
  if(!Number.isSafeInteger(desktopId)||desktopId<=0)throw hostError('host_certificate_project_unmapped',409);
  const body=JSON.stringify({projectId:desktopId,action:CERTIFICATE_TOOLS.get(name),args,operationId});
  if(Buffer.byteLength(body)>32*1024)throw hostError('host_certificate_input_too_large',400);
  const result=await new Promise((resolve,reject)=>{
   const fail=()=>reject(hostError(name==='certificate_task_status'?'host_certificate_executor_unavailable':'host_certificate_task_outcome_unknown',503,{outcomeUnknown:name!=='certificate_task_status',retryable:false}));
   const req=request(this.origin+'/certificate-task',{method:'POST',headers:{Authorization:`Bearer ${this.token}`,'Content-Type':'application/json','Content-Length':Buffer.byteLength(body)},timeout:15000},response=>{
    let chunks=[],size=0;response.on('data',chunk=>{size+=chunk.length;if(size>128*1024){response.destroy();fail();}else chunks.push(chunk)});
    response.on('error',fail);response.on('end',()=>{try{const value=JSON.parse(Buffer.concat(chunks).toString('utf8'));if(!value.ok){reject(hostError('host_certificate_task_rejected',409,{reason:String(value.error||'certificate_task_rejected').slice(0,500),retryable:false}));return}resolve(value.result);}catch{fail();}});
   });req.on('timeout',()=>{req.destroy();fail()});req.on('error',fail);req.end(body);
  });
  if(result?.projectId!==desktopId)throw hostError('host_certificate_project_binding_changed',409,{retryable:false});
  const scoped={...result,projectId,hostBoundProjectId:projectId,desktopProjectId:desktopId,
    projectBinding:{hostProjectId:projectId,desktopProjectId:desktopId,source:'host_saved_external_id'}};
  return {content:[{type:'text',text:JSON.stringify(scoped)}],structuredContent:scoped};
 }
}
