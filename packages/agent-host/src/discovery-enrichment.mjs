// Same authenticated desktop executor as the UI; never forwarded to a Worker.
import { CertificateTasks } from './certificate-tasks.mjs';
export const ENRICHMENT_TOOLS=new Map(['start','status','pause','resume'].map(action=>['discovery_enrichment_'+action,action]));
export class DiscoveryEnrichment extends CertificateTasks {
 constructor(options={}){super({...options,tools:ENRICHMENT_TOOLS,route:'/discovery-enrichment',kind:'discovery'});}
 async call(store,projectId,name,args,operationId){
  // These aliases always mean this Host-bound project's most recent queue.
  // Never normalize unknown UUIDs or resolve another project's task.
  if(name==='discovery_enrichment_status'&&['latest','current'].includes(args?.taskId)){
   const {taskId,...rest}=args;args=rest;
  }
  const result=await super.call(store,projectId,name,args,operationId);
  const body=result.structuredContent;
  // Keep control receipts parseable under the model's result budget. Complete
  // logs and conflict host sets stay in the native database and desktop UI.
  const view={...body,roots:(body.roots??[]).slice(0,12),rootCount:body.roots?.length??0,
    events:(body.events??[]).slice(0,5).map(e=>({...e,detail:String(e.detail??'').slice(0,200)})),
    conflicts:(body.conflicts??[]).slice(0,4).map(c=>({...c,hosts:(c.hosts??[]).slice(0,3),detail:String(c.detail??'').slice(0,150)})),
    fullDetailsInDesktopUI:true,previewTruncated:(body.events?.length??0)>5||(body.conflicts?.length??0)>4||(body.roots?.length??0)>12};
  if(Buffer.byteLength(JSON.stringify(view))>12000){view.events=view.events.slice(0,2);view.conflicts=view.conflicts.slice(0,2);view.roots=view.roots.slice(0,6);view.previewTruncated=true;}
  return {...result,structuredContent:view,content:[{type:'text',text:JSON.stringify(view)}]};
 }
}
