export class HostError extends Error {
  constructor(message,status=0,code='request_failed'){super(message);this.name='HostError';this.status=status;this.code=code;}
}
export class HostApi {
  constructor({token,fetchImpl=globalThis.fetch,origin=globalThis.location?.origin}={}){this.token=token;this.fetch=fetchImpl.bind(globalThis);this.origin=origin;}
  async request(path,{method='GET',body,signal}={}){
    if(!this.token)throw new HostError('Host session is unavailable. Open this page from the local Agent host.',401,'session_missing');
    if(!path.startsWith('/api/')||path.startsWith('//'))throw new HostError('Invalid host API path.',0,'path_invalid');
    const response=await this.fetch(path,{method,signal,credentials:'same-origin',headers:{'X-Oviraptor-Session':this.token,...(body===undefined?{}:{'Content-Type':'application/json'})},...(body===undefined?{}:{body:JSON.stringify(body)})});
    const text=await response.text();let value;
    try{value=text?JSON.parse(text):null;}catch{throw new HostError('The host returned an invalid response.',response.status,'invalid_response');}
    if(!response.ok){const detail=value?.error;throw new HostError(typeof detail==='string'?detail:detail?.message||value?.message||`Host request failed (${response.status}).`,response.status,(typeof detail==='string'?detail:detail?.code)||value?.code||'request_failed');}
    return value;
  }
  list(path,options){return this.request(path,options).then(value=>{const items=Array.isArray(value)?value:value?.items;if(!Array.isArray(items))throw new HostError('The host returned an invalid list.',0,'invalid_list');return items;});}
  post(path,body){return this.request(path,{method:'POST',body});}
  delete(path){return this.request(path,{method:'DELETE'});}
  async events(onEvent,{signal,onState=()=>{}}={}){
    if(!this.token)throw new HostError('Host session is unavailable.',401,'session_missing');
    onState('connecting');
    const response=await this.fetch('/api/events',{signal,credentials:'same-origin',headers:{Accept:'text/event-stream','X-Oviraptor-Session':this.token}});
    if(!response.ok||!response.body)throw new HostError(`Event stream unavailable (${response.status}).`,response.status,'event_stream_failed');
    onState('connected');const reader=response.body.getReader(),decoder=new TextDecoder(),parser=new SseParser(onEvent);
    try{while(true){const {done,value}=await reader.read();if(done)break;parser.push(decoder.decode(value,{stream:true}));}parser.push(decoder.decode());}
    finally{reader.releaseLock();}
  }
}
export class SseParser {
  constructor(emit){this.emit=emit;this.buffer='';this.data=[];this.event='';this.bytes=0;}
  push(chunk){
    this.buffer+=chunk;if(this.buffer.length>1024*1024)throw new HostError('Event frame exceeds the local UI limit.',0,'event_too_large');
    let end;while((end=this.buffer.indexOf('\n'))>=0){let line=this.buffer.slice(0,end);this.buffer=this.buffer.slice(end+1);if(line.endsWith('\r'))line=line.slice(0,-1);
      if(!line){if(this.data.length){const raw=this.data.join('\n');let payload;try{payload=JSON.parse(raw);}catch{throw new HostError('Invalid event data.',0,'invalid_event');}this.emit({...payload,...(!payload.type&&this.event?{type:this.event}:{})});}this.data=[];this.event='';this.bytes=0;continue;}
      if(line.startsWith(':'))continue;const split=line.indexOf(':');const field=split<0?line:line.slice(0,split);let value=split<0?'':line.slice(split+1);if(value.startsWith(' '))value=value.slice(1);
      if(field==='data'){this.bytes+=value.length;if(this.bytes>1024*1024)throw new HostError('Event frame exceeds the local UI limit.',0,'event_too_large');this.data.push(value);}else if(field==='event')this.event=value;
    }
  }
}
export const entityId=value=>typeof value==='string'||typeof value==='number'?String(value):'';
export const segment=value=>encodeURIComponent(entityId(value));
