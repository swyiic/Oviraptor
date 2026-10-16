// Isolated HTTP catch regression; synthetic Host, no app/native work or provider.
import http from 'node:http';import {pathToFileURL} from 'node:url';
const {serveHost}=await import(pathToFileURL(process.argv[2]));const outcomes=[];
for(const [name,thrown]of[['null',null],['undefined',undefined],['string-status',{status:'409',code:'asset_preferences_revision_conflict'}],['out-of-range-status',{status:999,code:'asset_preferences_revision_conflict'}]]){
 const h={apps:{callAction:async()=>{throw thrown}},onEvent:()=>{},status:()=>({}),setOrigin:()=>{}},web=await serveHost(h);let request;
 const observed=await new Promise(resolve=>{
  let settled=false;const finish=value=>{if(settled)return;settled=true;clearTimeout(timer);process.off('unhandledRejection',unhandled);resolve(value)};
  const unhandled=error=>finish({kind:'unhandledRejection',name:error?.name??typeof error});process.once('unhandledRejection',unhandled);
  const timer=setTimeout(()=>finish({kind:'deadline'}),1000);
  request=http.request(web.origin+'/api/apps/example.synthetic/actions/read',{method:'POST',agent:false,headers:{'X-Oviraptor-Session':web.token,'Content-Type':'application/json'}},response=>{let text='';response.on('data',b=>text+=b);response.on('end',()=>finish({kind:'response',status:response.statusCode,body:JSON.parse(text)}));response.on('error',e=>finish({kind:'responseError',code:e.code}));});
  request.on('error',e=>finish({kind:'requestError',code:e.code}));request.end('{}');
 });
 request.destroy();web.server.closeAllConnections();await web.close();
 outcomes.push({case:name,observed,passed:observed.kind==='response'&&observed.status===400&&observed.body.error==='host_request_failed'});
}
console.log(JSON.stringify({scope:'Real isolated private HTTP server with ordinary synthetic throws only; no native/app/business work accepted.',outcomes,accepted:outcomes.every(x=>x.passed)},null,2));process.exitCode=outcomes.every(x=>x.passed)?0:1;
