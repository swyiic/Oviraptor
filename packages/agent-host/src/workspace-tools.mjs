import { constants } from 'node:fs';
import {recordVersion,privateDirectory} from './workspace-versions.mjs';
import {executeWorkspaceScript} from './workspace-execution.mjs';
import { mkdir, readdir, lstat, realpath, open, rename, unlink, readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

const MAX_READ = 64 * 1024;
const MAX_WRITE = 256 * 1024;

export const WORKSPACE_TOOL_SPECS = [
  {
    name: 'workspace_list',
    description: 'List files in the conversation workspace. Read-only. Stays inside that directory.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, after: {type:'string'}, limit: {type:'integer',minimum:1,maximum:200} }, additionalProperties: false },
  },
  {
    name: 'workspace_read',
    description: 'Read a text file in the conversation workspace.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, offset:{type:'integer',minimum:0} }, required: ['path'], additionalProperties: false },
  },
  {
    name: 'workspace_write',
    description: 'Create or overwrite a text file in the conversation workspace. Use this to write scripts and tools the user asked for.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, content: { type: 'string' } }, required: ['path', 'content'], additionalProperties: false },
  },
  {name:'workspace_run',description:'Execute a Node.js or Python script from this workspace in an OS-isolated copy, with no network or user environment. Returns stdout, stderr, exit code and saved output files. Fix errors and run again explicitly; never claim tests passed on nonzero exit. Timeout/cancel does not publish output files.',inputSchema:{type:'object',properties:{path:{type:'string'},runtime:{enum:['node','python']},args:{type:'array',maxItems:32,items:{type:'string'}},timeoutMs:{type:'integer',minimum:100,maximum:60000}},required:['path','runtime'],additionalProperties:false}},
];

export function workspacePermission(toolName) {
  if (toolName === 'workspace_run') return 'workspace.run';
  if (toolName === 'workspace_write') return 'workspace.write';
  if (toolName === 'workspace_list' || toolName === 'workspace_read') return 'workspace.read';
  throw workspaceError('workspace_tool_unknown');
}

function workspaceError(code, message = code) { return Object.assign(Error(message), { code, retryable: false }); }

export function resolveInside(root, inputPath = '.') {
  const base = resolve(root);
  const target = resolve(base, String(inputPath || '.'));
  const rel = relative(base, target);
  if (rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel)) {
    throw workspaceError('workspace_path_denied');
  }
  return target;
}

async function workspaceRoot(root, anchor, assertAuthorized) {
  assertAuthorized();
  if (anchor) {
    const base = await realpath(anchor);
    const requested = relative(resolve(anchor), resolveInside(anchor, root));
    root = await checkedPath(base, requested, { createParents: true, assertAuthorized });
    assertAuthorized();
    try { await mkdir(root, { mode: 0o700 }); }
    catch (error) { if (error.code !== 'EEXIST') throw error; }
  } else await mkdir(root, { recursive: true, mode: 0o700 });
  const info = await lstat(root);
  if (info.isSymbolicLink() || !info.isDirectory()) throw workspaceError('workspace_path_denied');
  return realpath(root);
}

// Walk existing entries as well as canonical paths. Resolving a lexical path alone
// does not confine reads/writes when an old workspace contains symbolic links.
async function checkedPath(root, inputPath, { createParents = false, assertAuthorized = () => {} } = {}) {
  const target = resolveInside(root, inputPath);
  const components = relative(root, target).split(sep).filter(Boolean);
  let current = root;
  for (let index = 0; index < components.length; index++) {
    current = join(current, components[index]);
    const parent = index < components.length - 1;
    let info;
    try { info = await lstat(current); }
    catch (error) {
      if (error.code !== 'ENOENT') throw error;
      if (parent && createParents) {
        assertAuthorized();
        try { await mkdir(current, { mode: 0o700 }); }
        catch (mkdirError) { if (mkdirError.code !== 'EEXIST') throw mkdirError; }
        info = await lstat(current);
      } else if (!parent) return target;
      else throw error;
    }
    if (info.isSymbolicLink() || (parent && !info.isDirectory())) throw workspaceError('workspace_path_denied');
    resolveInside(root, await realpath(current));
  }
  return target;
}

const locks=new Map();
export async function runWorkspaceTool(root,name,args={},options={}){
 if(name!=='workspace_write'&&name!=='workspace_run')return runTool(root,name,args,options);
 const key=resolve(root),previous=locks.get(key)??Promise.resolve();let release;const wait=new Promise(r=>{release=r});const tail=previous.catch(()=>{}).then(()=>wait);locks.set(key,tail);await previous.catch(()=>{});
 try{return await runTool(root,name,args,options);}finally{release();if(locks.get(key)===tail)locks.delete(key);}
}
async function runTool(root, name, args = {}, { anchor, historyRoot, context={}, executionRoot, assertAuthorized = () => {} } = {}) {
  if (!WORKSPACE_TOOL_SPECS.some(tool => tool.name === name)) throw workspaceError('workspace_tool_unknown');
  root = await workspaceRoot(root, anchor, assertAuthorized);
  assertAuthorized();
  if(name==='workspace_run')return executeWorkspaceScript(root,args,{executionRoot,assertAuthorized,write:async(path,content)=>runTool(root,'workspace_write',{path,content},{anchor,historyRoot,context,assertAuthorized})});
  if (name === 'workspace_list') {
    const dir = await checkedPath(root, args.path || '.');
    assertAuthorized();
    const items = await readdir(dir, { withFileTypes: true });
    const limit=args.limit??200;if(!Number.isSafeInteger(limit)||limit<1||limit>200||args.after!==undefined&&typeof args.after!=='string')throw workspaceError('workspace_page_invalid');
    const sorted=items.sort((a,b)=>a.name<b.name?-1:a.name>b.name?1:0).filter(item=>!args.after||item.name>args.after),page=sorted.slice(0,limit);
    return { path: relative(root, dir) || '.', items: page.map(item => ({ name: item.name, kind: item.isSymbolicLink() ? 'symlink' : item.isDirectory() ? 'dir' : 'file' })),hasMore:sorted.length>limit,nextCursor:sorted.length>limit?page.at(-1).name:null,total:items.length };
  }
  if (name === 'workspace_read') {
    const file = await checkedPath(root, args.path);
    assertAuthorized();
    const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const info = await handle.stat();
      if (!info.isFile()) throw workspaceError('workspace_not_a_file');
      if (info.nlink > 1) throw workspaceError('workspace_path_denied');
      assertAuthorized();
      const offset=args.offset??0;if(!Number.isSafeInteger(offset)||offset<0||offset>info.size)throw workspaceError('workspace_offset_invalid');
      const buffer=Buffer.alloc(MAX_READ+4),{bytesRead}=await handle.read(buffer,0,buffer.length,offset);
      let length=Math.min(MAX_READ,bytesRead);
      // End before a UTF-8 continuation byte so nextOffset can resume losslessly.
      if(offset+length<info.size)while(length>0&&(buffer[length]&0xc0)===0x80)length--;
      const nextOffset=offset+length;
      return {path:relative(root,file),offset,nextOffset,totalBytes:info.size,truncated:nextOffset<info.size,content:buffer.subarray(0,length).toString('utf8')};
    } finally { await handle.close(); }
  }
  if (name === 'workspace_write') {
    if(historyRoot)await privateDirectory(historyRoot);
    const content = String(args.content ?? '');
    if (Buffer.byteLength(content) > MAX_WRITE) throw workspaceError('workspace_file_too_large');
    const file = await checkedPath(root, args.path, { createParents: true, assertAuthorized });
    if (file === root) throw workspaceError('workspace_not_a_file');
    try{const old=await lstat(file);if(old.isFile()&&old.nlink===1)await recordVersion(historyRoot,relative(root,file),await readFile(file),context,'prior');}catch(e){if(e.code!=='ENOENT')throw e;}
    const staging = join(dirname(file), `.workspace-write-${randomUUID()}`);
    let handle;
    try {
      assertAuthorized();
      handle = await open(staging, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
      await handle.writeFile(content, 'utf8');
      await handle.close(); handle = null;
      await checkedPath(root, args.path);
      // Replacing a file atomically also avoids modifying an outside inode if an
      // older workspace contains a hard link. Never follow an existing link.
      assertAuthorized();
      await rename(staging, file);
    } finally {
      await handle?.close();
      try { await unlink(staging); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    }
    const version=await recordVersion(historyRoot,relative(root,file),Buffer.from(content),context);
    return { path: relative(root, file), bytes: Buffer.byteLength(content),...(version?{version}:{} ) };
  }
}
