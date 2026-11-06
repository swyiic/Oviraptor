import {spawn} from 'node:child_process';
import {constants,existsSync,realpathSync} from 'node:fs';
import {mkdir,mkdtemp,readdir,lstat,open,writeFile,realpath,rm} from 'node:fs/promises';
import {join,resolve,relative,dirname,isAbsolute,extname} from 'node:path';
import {createHash} from 'node:crypto';
import {privateDirectory} from './workspace-versions.mjs';
import {scrubbedModelText} from './quality-prompt.mjs';
const error=code=>Object.assign(Error(code),{code});
const OUTPUT_LIMIT=65536;
function boundedScrubbedOutput(raw){
 const scrubbed=scrubbedModelText(raw);
 const bytes=Buffer.from(scrubbed);
 if(bytes.length<=OUTPUT_LIMIT)return {text:scrubbed,truncated:false};
 let end=OUTPUT_LIMIT;while(end>0&&(bytes[end]&0xc0)===0x80)end--;
 return {text:bytes.subarray(0,end).toString('utf8'),truncated:true};
}
const digest=b=>createHash('sha256').update(b).digest('hex');
const MAX_BYTES=40*1024*1024,MAX_FILE=8*1024*1024;
async function tree(root){const out=new Map();let total=0;async function walk(dir,depth=0){if(depth>16)throw error('workspace_execution_capacity');for(const entry of await readdir(dir,{withFileTypes:true})){const path=join(dir,entry.name),info=await lstat(path);if(info.isSymbolicLink()||info.nlink>1&&!info.isDirectory())throw error('workspace_path_denied');if(info.isDirectory()){await walk(path,depth+1);continue;}if(!info.isFile()||info.size>MAX_FILE||out.size>=1000||(total+=info.size)>MAX_BYTES)throw error('workspace_execution_capacity');const handle=await open(path,constants.O_RDONLY|constants.O_NOFOLLOW);let bytes;try{const stat=await handle.stat();if(!stat.isFile()||stat.nlink>1)throw error('workspace_path_denied');bytes=await handle.readFile();}finally{await handle.close()}out.set(relative(root,path),{bytes,sha256:digest(bytes)});}}await walk(root);return out;}
function sandbox(work,runtime,script,args,readRoots=[]){
 const reads=readRoots.filter(path=>typeof path==='string'&&path.startsWith('/')&&!path.includes('\0')&&!path.includes('"'));
 if(process.platform==='darwin'&&existsSync('/usr/bin/sandbox-exec')){
  const quoted=x=>JSON.stringify(x);
  const excluded=['/Users','/Volumes','/private/var','/private/tmp','/tmp','/private/etc','/etc','/Library/Keychains','/opt/homebrew/etc'];
  const extra=reads.map(path=>`(subpath ${quoted(path)})`).join(' ');
  const profile=`(version 1)(deny default)(allow process-exec (literal ${quoted(runtime)}))(allow process-info* (target self))(allow sysctl-read)(allow file-read-metadata)(allow file-map-executable)(allow file-read* (require-all ${excluded.map(p=>`(require-not (subpath ${quoted(p)}))`).join(' ')}))(allow file-read* (subpath ${quoted(work)}) (literal ${quoted(runtime)}) ${extra})(allow file-write* (subpath ${quoted(work)}))`;
  return ['/usr/bin/sandbox-exec',['-p',profile,runtime,script,...args]];
 }
 if(process.platform==='linux'&&existsSync('/usr/bin/bwrap')){
  const flags=['--unshare-all','--die-with-parent','--new-session','--proc','/proc','--dev','/dev','--tmpfs','/tmp'];
  for(const path of ['/usr','/bin','/lib','/lib64',dirname(runtime)])if(existsSync(path))flags.push('--ro-bind',path,path);
  for(const path of reads)if(existsSync(path))flags.push('--ro-bind',path,path);
  flags.push('--bind',work,work,'--chdir',work,runtime,script,...args);return['/usr/bin/bwrap',flags];
 }
 throw error('workspace_execution_unsupported');
}
export async function executeWorkspaceScript(root,input,{executionRoot,assertAuthorized,write,readRoots=[]}){
 if(!input||!['node','python'].includes(input.runtime)||typeof input.path!=='string'||!input.path||isAbsolute(input.path)||input.path.split('/').some(p=>p==='..'||p==='.'||!p)||/[\\\u0000-\u001f]/u.test(input.path)||!Array.isArray(input.args??[])||(input.args??[]).length>32||(input.args??[]).some(a=>typeof a!=='string'||a.length>4096)||Object.keys(input).some(k=>!['path','runtime','args','timeoutMs'].includes(k)))throw error('workspace_execution_invalid');
 const timeoutMs=input.timeoutMs??30000;if(!Number.isSafeInteger(timeoutMs)||timeoutMs<100||timeoutMs>1800000)throw error('workspace_execution_invalid');
 if(!executionRoot)throw error('workspace_execution_unsupported');
 assertAuthorized();const before=await tree(root);if(!before.has(input.path))throw error('workspace_script_unavailable');
 const valid=input.runtime==='node'?['.js','.mjs','.cjs']:['.py'];if(!valid.includes(extname(input.path).toLowerCase()))throw error('workspace_execution_invalid');
 let runtime;try{let python='/usr/bin/python3';if(process.platform==='darwin'){const base='/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/Current/Resources/Python.app/Contents/MacOS/Python';if(existsSync(base))python=base;else if(existsSync('/Library/Developer/CommandLineTools/usr/bin/python3'))python='/Library/Developer/CommandLineTools/usr/bin/python3';}runtime=realpathSync(input.runtime==='node'?process.execPath:python);}catch{throw error('workspace_runtime_unavailable')}
 await privateDirectory(executionRoot);
 const work=await mkdtemp(join(await realpath(executionRoot),'run-'));let command,argv;
 try{
  for(const[path,entry]of before){const dest=join(work,path);await mkdir(dirname(dest),{recursive:true,mode:0o700});await writeFile(dest,entry.bytes,{mode:0o600,flag:'wx'});}
  await mkdir(join(work,'.tmp'),{mode:0o700,recursive:true});[command,argv]=sandbox(work,runtime,join(work,input.path),input.args??[],readRoots);assertAuthorized();
  const result=await new Promise((resolveResult,reject)=>{
   let stdoutChunks=[],stderrChunks=[],timedOut=false,cancelled=false,settled=false,child;
   const append=(kind,chunk)=>{(kind==='out'?stdoutChunks:stderrChunks).push(Buffer.isBuffer(chunk)?chunk:Buffer.from(chunk));};
   const stop=()=>{try{process.kill(-child.pid,'SIGKILL')}catch{child.kill('SIGKILL')}};
   child=spawn(command,argv,{cwd:work,detached:true,stdio:['ignore','pipe','pipe'],env:{PATH:'/usr/bin:/bin',HOME:work,TMPDIR:join(work,'.tmp'),LANG:'en_US.UTF-8',OPENSSL_CONF:'/dev/null',PYTHONDONTWRITEBYTECODE:'1'}});
   child.stdout.on('data',b=>append('out',b));child.stderr.on('data',b=>append('err',b));
   const timer=setTimeout(()=>{timedOut=true;stop()},timeoutMs),check=setInterval(()=>{try{assertAuthorized()}catch{cancelled=true;stop()}},100);
   const finish=()=>{clearTimeout(timer);clearInterval(check)};
   child.on('error',e=>{finish();if(!settled){settled=true;reject(error('workspace_execution_launch_failed'))}});
   child.on('close',(exitCode,signal)=>{finish();if(settled)return;settled=true;const out=boundedScrubbedOutput(Buffer.concat(stdoutChunks).toString('utf8')),err=boundedScrubbedOutput(Buffer.concat(stderrChunks).toString('utf8'));resolveResult({exitCode,signal,stdout:out.text,stderr:err.text,truncated:out.truncated||err.truncated,timedOut,cancelled,success:exitCode===0&&!timedOut&&!cancelled});});
  });
  const files=[],conflicts=[],deleted=[],outputs=[];
  if(result.success){assertAuthorized();const after=await tree(work),current=await tree(root);for(const[path,entry]of after){if(path.startsWith('.tmp/'))continue;if(before.get(path)?.sha256===entry.sha256)continue;if(current.get(path)?.sha256!==before.get(path)?.sha256){conflicts.push(path);continue;}if(entry.bytes.includes(0)){conflicts.push(path);continue;}if(entry.bytes.length>16*1024*1024){conflicts.push(path);continue;}const content=entry.bytes.toString('utf8');if(!Buffer.from(content).equals(entry.bytes)){conflicts.push(path);continue;}outputs.push([path,content]);}for(const path of before.keys())if(!after.has(path))deleted.push(path);}
  if(result.success&&!conflicts.length){assertAuthorized();for(const[path,content]of outputs)files.push(await write(path,content));}
  return {...result,files,conflicts,deletedNotApplied:deleted,success:result.success&&conflicts.length===0,isolated:true,networkAllowed:false,originalFilesChanged:files.length>0};
 }finally{await rm(work,{recursive:true,force:true});}
}
