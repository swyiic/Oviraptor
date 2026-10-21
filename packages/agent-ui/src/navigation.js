export const pageScope = page => ({projectId:page.projectId,pageId:page.pageId,version:page.version,expectedAppRevision:page.appRevision,expectedGrantRevision:page.grantRevision});
const stale = () => { throw Object.assign(Error('host_app_page_stale'),{code:'host_app_page_stale'}); };
/** Parent-only confirmation tickets never enter an application frame. */
export async function reviewNavigation({api,page,type,input,isCurrent,review}) {
  const kind=type==='navigation.external'?'external':type==='navigation.handoff'?'handoff':null;
  if(!kind)throw Error('Invalid navigation request');
  const check=()=>{if(!isCurrent())stale();};check();
  const scope=pageScope(page);let prepared,confirmed=false;
  try{
    prepared=await api.post(`/api/apps/${encodeURIComponent(page.appId)}/navigation/${kind}`,{scope,input});check();
    if(prepared.kind!==kind||prepared.projectId!==page.projectId||typeof prepared.token!=='string'||!/^[a-f0-9]{64}$/u.test(prepared.token)||!Number.isSafeInteger(prepared.expiresAt)||prepared.expiresAt<=Date.now())throw Error('Invalid navigation confirmation');
    const accepted=await review(prepared);check();
    if(!accepted)return {cancelled:true};
    const result=await api.post('/api/navigation/confirm',{token:prepared.token,scope});confirmed=true;check();
    if(result.kind!==kind)throw Error('Invalid navigation confirmation');
    if(kind==='external'){
      const url=new URL(result.url);
      if(!['http:','https:'].includes(url.protocol)||url.username||url.password||url.href.length>2048||/[\u0000-\u0020\u007f]/u.test(result.url))throw Error('Invalid external destination');
      return {kind,url:url.href};
    }
    if(result.projectId!==page.projectId||result.targetAppId!==input.targetAppId||result.targetPageId!==input.targetPageId||typeof result.deliveryToken!=='string'||!/^[a-f0-9]{64}$/u.test(result.deliveryToken))throw Error('Invalid application destination');
    return result;
  }finally{
    if(prepared&&!confirmed)try{await api.post('/api/navigation/cancel',{token:prepared.token,scope});}catch{/* Expired or revoked tickets cannot authorize anything. */}
  }
}
