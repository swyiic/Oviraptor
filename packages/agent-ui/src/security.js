export const BRIDGE_PROTOCOL='oviraptor.app-ui.v1';
export function appPageUrl(raw,appId,origin){
  const url=new URL(raw,origin),prefix=`/apps/${encodeURIComponent(appId)}/`;
  if(url.origin!==origin||!url.pathname.startsWith(prefix)||!/^\/apps\/[^/]+\/[^/]+\/pages\/[^/]+$/.test(url.pathname)||url.search||url.hash||url.username||url.password)throw Error('The application page URL is not a registered local page.');
  return url.href;
}
export function bridgeMessage(event,{source,appId,nonce}){
  const message=event.data;
  if(event.source!==source||event.origin!=='null'||!message||typeof message!=='object'||Array.isArray(message)||message.protocol!==BRIDGE_PROTOCOL||message.appId!==appId||message.nonce!==nonce)return null;
  if(message.type==='ready'||message.type==='close')return {type:message.type};
  if(message.type==='resize'&&Number.isFinite(message.height))return {type:'resize',height:Math.max(240,Math.min(1200,message.height))};
  if(message.type==='notice'&&typeof message.text==='string'&&message.text.length<=500)return {type:'notice',text:message.text};
  return null; // No API, command, fetch, navigation or permission proxy.
}
export function sessionToken(document,window){return window.__OVIRAPTOR_HOST__?.token||document.querySelector('meta[name="oviraptor-session-token"]')?.content||'';}
