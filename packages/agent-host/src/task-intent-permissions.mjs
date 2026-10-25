// Names to OFFER, never authority to execute. Persisted grants and runtime
// identity remain checked by permissionOffer and the actual tool owner.
import { exposureTaskIntent, discoveryAssistIntent, looksLikeDataQuestion, discoveryEnrichmentIntent, certificateTaskIntent, scanningTaskIntent, isReadOnlyDataRequest, isDataReadTool, scanningHandoffIntent, discoverySeedReadIntent, discoveryCleanupIntent } from './quality-prompt.mjs';

const ASSETS='org.oviraptor.assets', SCANNING='org.oviraptor.scanning';
export function requestedBusinessTools(text,{applications=[],certificateAvailable=false,scanningAvailable=false,exposureAvailable=false}={}) {
  const requested=new Set(), readOnly=isReadOnlyDataRequest(text);
  const find=id=>applications.find(app=>app.id===id&&app.enabled);
  const want=(id,name)=>{
    if(readOnly&&!isDataReadTool(name))return;
    if(find(id)?.manifest.tools.some(tool=>tool.name===name))requested.add(`${id}::${name}`);
  };
  const cleanup=discoveryCleanupIntent(text);
  if(cleanup&&certificateAvailable){want(ASSETS,'discovery_cleanup_status');if(cleanup.action!=='status')want(ASSETS,'discovery_cleanup_'+cleanup.action);}
  const seeds=discoverySeedReadIntent(text);
  if(seeds&&certificateAvailable){want(ASSETS,'discovery_seed_list');want(ASSETS,'discovery_seed_detail');}
  const exposure=exposureTaskIntent(text);
  if(exposure&&exposureAvailable){want(ASSETS,'exposure_task_status');if(exposure.action==='review'){want(ASSETS,'exposure_findings');want(ASSETS,'exposure_finding');}else if(exposure.action!=='status')want(ASSETS,'exposure_task_'+exposure.action);}
  const assist=discoveryAssistIntent(text);
  if(assist&&certificateAvailable){want(ASSETS,'discovery_assist_status');if(assist.action!=='status')want(ASSETS,'discovery_assist_'+assist.action);want(ASSETS,'discovery_scope_propose');}
  const scan=scanningTaskIntent(text),cert=certificateTaskIntent(text),discovery=discoveryEnrichmentIntent(text);
  if(discovery&&certificateAvailable){want(ASSETS,'discovery_enrichment_status');if(discovery.action!=='status')want(ASSETS,'discovery_enrichment_'+discovery.action);}
  const scanner=find(SCANNING),scanAvailable=scanningAvailable||scanner?.manifest.schemaVersion===2;
  if(!scan&&!exposure&&looksLikeDataQuestion(text)&&/资产|证书|域名|当前项目|asset|certificate|inventory|project overview/i.test(text)) {
    const tool=find(ASSETS)?.manifest.tools.find(tool=>tool.permission==='inventory.read');
    if(tool)want(ASSETS,tool.name);
  }
  if(cert&&certificateAvailable){want(ASSETS,'certificate_task_status');if(cert.action!=='status')want(ASSETS,'certificate_task_'+cert.action);}
  if(scan&&scanAvailable){want(SCANNING,'scan_status');want(SCANNING,'scan_list');if(scan.action==='resume')want(SCANNING,'scan_recovery');if(scan.action!=='status')want(SCANNING,'scan_'+scan.action);}
  const handoff=scanningHandoffIntent(text);
  if(!readOnly&&handoff&&scanAvailable&&find(ASSETS)) {
    const read=find(ASSETS).manifest.tools.find(tool=>tool.permission==='inventory.read');
    if(read)want(ASSETS,read.name);
    for(const name of ['scan_from_assets','scan_status','scan_list',...(handoff.preflight?['scan_preflight']:[])])want(SCANNING,name);
    if(handoff.start)want(SCANNING,'scan_start');
  }
  // A combined read intentionally has no single-owner control intent.
  if(readOnly){
    if(certificateAvailable&&/证书|补采|certificate/i.test(text))want(ASSETS,'certificate_task_status');
    if(scanAvailable&&/扫描|scan/i.test(text))for(const name of ['scan_list','scan_status','scan_results'])want(SCANNING,name);
  }
  return [...requested];
}
