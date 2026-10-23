import {mkdir,lstat,readdir,readFile,writeFile} from 'node:fs/promises';
import {createHash,randomUUID} from 'node:crypto';
import {join,resolve,parse} from 'node:path';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const fail=()=>{throw Object.assign(Error('workspace_history_unavailable'),{code:'workspace_history_unavailable'})};
export async function privateDirectory(path,create=true){const full=resolve(path);let current=parse(full).root;for(const part of full.slice(current.length).split('/').filter(Boolean)){current=join(current,part);let info;try{info=await lstat(current)}catch(e){if(e.code!=='ENOENT'||!create)throw e;await mkdir(current,{mode:0o700});info=await lstat(current)}if(info.isSymbolicLink()&&!['/tmp','/var'].includes(current)||!info.isDirectory()&&!info.isSymbolicLink())fail();}}
const directory=privateDirectory;
export async function recordVersion(historyRoot,path,bytes,context={},kind='write'){
 if(!historyRoot)return null;
 await directory(historyRoot);const dir=join(historyRoot,hash(path));await directory(dir);
 const versionId=Date.now().toString().padStart(16,'0')+'-'+randomUUID();const value={versionId,path,sha256:hash(bytes),size:bytes.length,createdAt:new Date().toISOString(),conversationId:context.conversationId??null,runId:context.runId??null,kind,contentBase64:Buffer.from(bytes).toString('base64')};
 await writeFile(join(dir,versionId+'.json'),JSON.stringify(value),{flag:'wx',mode:0o600});const{contentBase64,...metadata}=value;return metadata;
}
export async function fileVersions(historyRoot,path,{after='',limit=50}={}){
 if(!Number.isSafeInteger(limit)||limit<1||limit>200)fail();const dir=join(historyRoot,hash(path));let names;try{await privateDirectory(dir,false);const s=await lstat(dir);if(s.isSymbolicLink()||!s.isDirectory())fail();names=(await readdir(dir)).filter(n=>/^\d{16}-[a-f0-9-]{36}\.json$/.test(n)).sort().reverse().filter(n=>!after||n<after);}catch(e){if(e.code==='ENOENT')return{items:[],hasMore:false,nextCursor:null};throw e;}
 const selected=names.slice(0,limit),items=[];for(const name of selected){const p=join(dir,name),s=await lstat(p);if(s.isSymbolicLink()||!s.isFile()||s.nlink>1)fail();const v=JSON.parse(await readFile(p,'utf8'));if(v.path!==path)fail();const{contentBase64,...metadata}=v;items.push(metadata);}
 return{items,hasMore:names.length>limit,nextCursor:names.length>limit?selected.at(-1):null};
}
export async function readVersion(historyRoot,path,versionId){
 if(!/^\d{16}-[a-f0-9-]{36}$/.test(versionId))fail();const dir=join(historyRoot,hash(path));await privateDirectory(dir,false);const s=await lstat(dir);if(s.isSymbolicLink()||!s.isDirectory())fail();const p=join(dir,versionId+'.json'),info=await lstat(p);if(info.isSymbolicLink()||!info.isFile()||info.nlink>1)fail();const v=JSON.parse(await readFile(p,'utf8')),bytes=Buffer.from(v.contentBase64,'base64');if(v.path!==path||v.versionId!==versionId||hash(bytes)!==v.sha256)fail();return{...v,bytes};
}
