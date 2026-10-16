export const BRIDGE_PROTOCOL='oviraptor.app-ui.v1';
export const NATIVE_BRIDGE_PROTOCOL='oviraptor.app-ui.v2';
export function appPageUrl(raw,appId,origin){
  const url=new URL(raw,origin),prefix=`/apps/${encodeURIComponent(appId)}/`;
  if(url.origin!==origin||!url.pathname.startsWith(prefix)||!/^\/apps\/[^/]+\/[^/]+\/pages\/[^/]+$/.test(url.pathname)||url.search||url.hash||url.username||url.password)throw Error('The application page URL is not a registered local page.');
  return url.href;
}
export function bridgeMessage(event,{source,appId,nonce,protocol=BRIDGE_PROTOCOL}){
  const message=event.data;
  if(event.source!==source||event.origin!=='null'||!message||typeof message!=='object'||Array.isArray(message)||message.protocol!==protocol||message.appId!==appId||message.nonce!==nonce)return null;
  if(message.type==='ready'||message.type==='close')return {type:message.type};
  if(message.type==='resize'&&Number.isFinite(message.height))return {type:'resize',height:Math.max(240,Math.min(1200,message.height))};
  if(message.type==='notice'&&typeof message.text==='string'&&message.text.length<=500)return {type:'notice',text:message.text};
  if(protocol===NATIVE_BRIDGE_PROTOCOL&&['action.call','tool.call','artifact.download'].includes(message.type)){
    if(Object.keys(message).some(key=>!['protocol','appId','nonce','type','requestId','name','arguments','operationId'].includes(key))||!Number.isSafeInteger(message.requestId)||message.requestId<1||!(/^[a-z][a-z0-9_]{0,63}$/u.test(message.name))||!message.arguments||typeof message.arguments!=='object'||Array.isArray(message.arguments)||typeof message.operationId!=='string'||!message.operationId||message.operationId.length>128||/[\u0000-\u001f\u007f]/u.test(message.operationId))return null;
    try{const raw=JSON.stringify(message,(_key,value)=>{if(['undefined','function','symbol','bigint'].includes(typeof value)||typeof value==='number'&&!Number.isFinite(value))throw Error('invalid');return value;});if(new TextEncoder().encode(raw).length>1024*1024)return null;}catch{return null;}
    return {type:message.type,requestId:message.requestId,name:message.name,arguments:message.arguments,operationId:message.operationId};
  }
  return null; // No API, command, fetch, navigation or permission proxy.
}
export function sessionToken(document,window){return window.__OVIRAPTOR_HOST__?.token||document.querySelector('meta[name="oviraptor-session-token"]')?.content||'';}
