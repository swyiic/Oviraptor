// Assets business adapter for the public generic page client. No Host imports.
const fail=code=>{throw Object.assign(Error(code),{code});};
const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const exact=(value,keys)=>value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).sort().join(',')===keys.slice().sort().join(',');
function url(value){if(typeof value!=='string'||value.length>2048||/[\u0000-\u0020\u007f]/u.test(value))fail('assets_external_url_invalid');let parsed;try{parsed=new URL(value);}catch{fail('assets_external_url_invalid');}if(!['http:','https:'].includes(parsed.protocol)||parsed.username||parsed.password)fail('assets_external_url_invalid');return value;}
export function validateScanSelection(value,assetIds,mode){
 if(!exact(value,['kind','targets','mode','source'])||value.kind!=='scan.targets.v1'||value.mode!==mode||!Array.isArray(value.targets)||!value.targets.length||value.targets.length>200||new Set(value.targets).size!==value.targets.length||!exact(value.source,['kind','applicationInstanceId','selectionId','selectionDigest','assetCount']))fail('assets_scan_selection_invalid');
 value.targets.forEach(url);const source=value.source;
 if(source.kind!=='asset-selection'||typeof source.applicationInstanceId!=='string'||!source.applicationInstanceId||source.applicationInstanceId.length>128||/[\u0000-\u0020\u007f]/u.test(source.applicationInstanceId)||!uuid.test(source.selectionId)||!/^[a-f0-9]{64}$/.test(source.selectionDigest)||source.assetCount!==assetIds.length||value.targets.length>source.assetCount)fail('assets_scan_selection_invalid');
 const payload=JSON.parse(JSON.stringify(value));if(new TextEncoder().encode(JSON.stringify(payload)).length>512*1024)fail('assets_scan_selection_too_large');return payload;
}
export function createAssetsNavigation({invoke,getContext,getRevision,getClient,beforeHandoff}){
 function capture(){const context=getContext(),client=getClient(),revision=getRevision();if(!context||!client)fail('assets_page_not_connected');return{context,client,revision};}
 function current(saved){if(getContext()!==saved.context||getClient()!==saved.client||getRevision()!==saved.revision)fail('assets_page_context_changed');}
 return Object.freeze({
  async openExternal(value){const saved=capture();url(value);if(typeof saved.client.openExternal!=='function')fail('assets_external_open_not_connected');const result=await saved.client.openExternal(value);current(saved);if(result?.cancelled===true)return{cancelled:true};if(result?.offered!==true)fail('assets_external_offer_invalid');return{offered:true};},
  async sendSelection(projectId,assetIds,mode){const saved=capture();if(projectId!==saved.context.nativeProjectId)fail('assets_project_scope_changed');if(!Array.isArray(assetIds)||!assetIds.length||assetIds.length>1000||assetIds.some(id=>!Number.isSafeInteger(id)||id<1)||new Set(assetIds).size!==assetIds.length||!['quick','standard','deep'].includes(mode))fail('assets_scan_selection_invalid');if(typeof saved.client.handoff!=='function')fail('assets_scanning_handoff_not_connected');const prepared=validateScanSelection(await invoke('prepare_scan_selection',{assetIds:[...assetIds],mode}),assetIds,mode);current(saved);if(typeof beforeHandoff!=='function')fail('assets_preferences_handoff_unconfirmed');await beforeHandoff(mode);current(saved);const result=await saved.client.handoff('org.oviraptor.scanning','home',prepared);current(saved);if(result?.cancelled===true)throw Object.assign(Error('assets_scanning_handoff_cancelled'),{code:'assets_scanning_handoff_cancelled'});if(result?.handoffAccepted!==true||result?.draftOnly!==true)fail('assets_scanning_handoff_receipt_invalid');return{handoffAccepted:true,draftOnly:true};}
 });
}
