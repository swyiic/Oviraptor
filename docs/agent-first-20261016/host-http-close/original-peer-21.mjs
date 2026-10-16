// Synthetic real subprocess JSON-RPC peer. Not a Rust/business-engine fixture.
import {readFile} from 'node:fs/promises';import {createHash} from 'node:crypto';import {createInterface} from 'node:readline';
const [directory,version]=process.argv.slice(2),bytes=await readFile(directory+'/manifest.json'),m=JSON.parse(bytes);
const send=v=>process.stdout.write(JSON.stringify(v)+'\n');let held;
process.on('message',message=>{if(message==='release'&&held){held();held=undefined;}});
createInterface({input:process.stdin}).on('line',line=>{
 const q=JSON.parse(line),reply=result=>send({jsonrpc:'2.0',id:q.id,result});
 if(q.method==='describe')return reply(version==='v1'?{protocolVersion:1,appId:m.id,version:m.version,tools:m.tools,pages:m.pages}:{protocolVersion:2,appId:m.id,version:m.version,manifestSha256:createHash('sha256').update(bytes).digest('hex'),toolNames:m.tools.map(x=>x.name),actionNames:m.actions.map(x=>x.name),pageIds:m.pages.map(x=>x.id)});
 if(q.method==='lifecycle/start')return reply({dataSchemaVersion:1});
 if(q.method==='lifecycle/status')return reply({busy:false});
 if(q.method==='lifecycle/stop'){reply({busy:false});process.disconnect?.();process.stdin.destroy();return;}
 if(['action/call','tool/call'].includes(q.method)){
  const mode=q.params.arguments.mode??'conflict';const codes={conflict:'asset_preferences_revision_conflict',upper:'ASSET_CAS_STALE',invalid:'token: /private/userdata/key',long:'x'.repeat(97),numeric:123,empty:''};
  const error=()=>send({jsonrpc:'2.0',id:q.id,error:{code:-32000,message:'PRIVATE_API_KEY /private/userdata/db.sqlite internal stack',data:{code:codes[mode==='hold'?'conflict':mode],privateKey:'must-not-leak'}}});
  if(mode==='hold'){held=error;process.send({entered:true});}else error();
 }
});
