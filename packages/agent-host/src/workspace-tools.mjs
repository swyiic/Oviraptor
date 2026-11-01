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
  {
    name: 'workspace_edit',
    description: 'Replace an exact text span in an existing file in the conversation workspace. oldString must occur exactly once (set replaceAll to replace every occurrence). Prefer this over workspace_write for surgical changes — it rewrites only the matched span and keeps the rest byte-for-byte.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, oldString: { type: 'string' }, newString: { type: 'string' }, replaceAll: { type: 'boolean' } }, required: ['path', 'oldString', 'newString'], additionalProperties: false },
  },
  {
    name: 'workspace_grep',
    description: 'Search text files in the conversation workspace for a regular expression. Read-only. Returns matching file/line/text, bounded. Set literal:true to match a plain string, caseInsensitive:true to ignore case, include to a glob that filters file paths (e.g. **/*.mjs).',
    inputSchema: { type: 'object', properties: { pattern: { type: 'string' }, path: { type: 'string' }, include: { type: 'string' }, caseInsensitive: { type: 'boolean' }, literal: { type: 'boolean' }, maxResults: { type: 'integer', minimum: 1, maximum: 1000 } }, required: ['pattern'], additionalProperties: false },
  },
  {
    name: 'workspace_glob',
    description: 'List files in the conversation workspace whose path matches a glob (e.g. **/*.mjs, src/*.js, test/**). Read-only. Returns matching relative paths, sorted and bounded. Use this to find files before reading or editing them.',
    inputSchema: { type: 'object', properties: { pattern: { type: 'string' }, path: { type: 'string' }, maxResults: { type: 'integer', minimum: 1, maximum: 2000 } }, required: ['pattern'], additionalProperties: false },
  },
];

export function workspacePermission(toolName) {
  if (toolName === 'workspace_run') return 'workspace.run';
  if (toolName === 'workspace_write' || toolName === 'workspace_edit') return 'workspace.write';
  if (toolName === 'workspace_list' || toolName === 'workspace_read' || toolName === 'workspace_grep' || toolName === 'workspace_glob') return 'workspace.read';
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

// Anchored glob → RegExp. '**' spans directories, '*'/'?' stay within a path
// segment, and every other character is matched literally (regex metachars
// escaped). Used by workspace_glob and workspace_grep's include filter.
export function globToRegExp(glob) {
  let re = '';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        i++;
        if (glob[i + 1] === '/') { i++; re += '(?:[^/]+/)*'; }
        else re += '.*';
      } else re += '[^/]*';
    } else if (c === '?') re += '[^/]';
    else if (/[.+^${}()|[\]\\]/.test(c)) re += '\\' + c;
    else re += c;
  }
  return new RegExp('^' + re + '$');
}

// Collect regular files under baseDir (relative to root), skipping symlinks and
// hard-linked files — the same confinement the execution tree walk enforces, so
// a stale workspace's links can never reach outside. Bounded by `cap`.
async function collectFiles(root, baseDir, cap) {
  const files = [];
  let truncated = false;
  async function walk(dir, depth) {
    if (truncated || depth > 16) return;
    let entries;
    try { entries = await readdir(dir, { withFileTypes: true }); }
    catch (error) { if (error.code === 'ENOENT' || error.code === 'ENOTDIR') return; throw error; }
    entries.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
    for (const entry of entries) {
      if (truncated) return;
      const path = join(dir, entry.name);
      let info;
      try { info = await lstat(path); } catch (error) { if (error.code === 'ENOENT') continue; throw error; }
      if (info.isSymbolicLink()) continue;
      if (info.isDirectory()) { await walk(path, depth + 1); continue; }
      if (!info.isFile() || info.nlink > 1) continue;
      if (files.length >= cap) { truncated = true; return; }
      files.push({ rel: relative(root, path), size: info.size });
    }
  }
  await walk(baseDir, 0);
  return { files, truncated };
}

// The single atomic-write path, shared by workspace_write and workspace_edit:
// stage under a random name, never follow an existing link, re-check the target
// path before the rename, and record prior + new versions.
async function writeFileContent(root, inputPath, content, { historyRoot, context = {}, assertAuthorized = () => {} } = {}) {
  if (historyRoot) await privateDirectory(historyRoot);
  if (Buffer.byteLength(content) > MAX_WRITE) throw workspaceError('workspace_file_too_large');
  const file = await checkedPath(root, inputPath, { createParents: true, assertAuthorized });
  if (file === root) throw workspaceError('workspace_not_a_file');
  try { const old = await lstat(file); if (old.isFile() && old.nlink === 1) await recordVersion(historyRoot, relative(root, file), await readFile(file), context, 'prior'); } catch (e) { if (e.code !== 'ENOENT') throw e; }
  const staging = join(dirname(file), `.workspace-write-${randomUUID()}`);
  let handle;
  try {
    assertAuthorized();
    handle = await open(staging, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
    await handle.writeFile(content, 'utf8');
    await handle.close(); handle = null;
    await checkedPath(root, inputPath);
    assertAuthorized();
    await rename(staging, file);
  } finally {
    await handle?.close();
    try { await unlink(staging); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  const version = await recordVersion(historyRoot, relative(root, file), Buffer.from(content), context);
  return { path: relative(root, file), bytes: Buffer.byteLength(content), ...(version ? { version } : {}) };
}

const locks=new Map();
export async function runWorkspaceTool(root,name,args={},options={}){
 if(name!=='workspace_write'&&name!=='workspace_run'&&name!=='workspace_edit')return runTool(root,name,args,options);
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
    return writeFileContent(root, args.path, String(args.content ?? ''), { historyRoot, context, assertAuthorized });
  }
  if (name === 'workspace_edit') {
    const oldString = String(args.oldString ?? ''), newString = String(args.newString ?? '');
    if (!oldString) throw workspaceError('workspace_edit_invalid');
    if (oldString === newString) throw workspaceError('workspace_edit_no_change');
    const replaceAll = args.replaceAll === true;
    const file = await checkedPath(root, args.path);
    assertAuthorized();
    const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
    let current;
    try {
      const info = await handle.stat();
      if (!info.isFile()) throw workspaceError('workspace_not_a_file');
      if (info.nlink > 1) throw workspaceError('workspace_path_denied');
      if (info.size > MAX_WRITE) throw workspaceError('workspace_file_too_large');
      current = (await handle.readFile()).toString('utf8');
    } finally { await handle.close(); }
    let count = 0;
    for (let idx = current.indexOf(oldString); idx !== -1; idx = current.indexOf(oldString, idx + oldString.length)) count++;
    if (count === 0) throw workspaceError('workspace_edit_no_match');
    if (count > 1 && !replaceAll) throw workspaceError('workspace_edit_not_unique');
    // Splice literally (slice/split+join), so '$'-sequences in newString are not
    // reinterpreted as String.replace replacement patterns.
    const first = current.indexOf(oldString);
    const updated = replaceAll ? current.split(oldString).join(newString)
      : current.slice(0, first) + newString + current.slice(first + oldString.length);
    const result = await writeFileContent(root, args.path, updated, { historyRoot, context, assertAuthorized });
    return { ...result, replaced: replaceAll ? count : 1 };
  }
  if (name === 'workspace_grep') {
    const pattern = String(args.pattern ?? '');
    if (!pattern || pattern.length > 4096) throw workspaceError('workspace_grep_invalid');
    const maxResults = args.maxResults ?? 200;
    if (!Number.isSafeInteger(maxResults) || maxResults < 1 || maxResults > 1000) throw workspaceError('workspace_grep_invalid');
    let includeRe = null;
    if (args.include !== undefined) {
      if (typeof args.include !== 'string' || !args.include || args.include.length > 1024) throw workspaceError('workspace_grep_invalid');
      includeRe = globToRegExp(args.include);
    }
    let re;
    try { re = new RegExp(args.literal === true ? pattern.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') : pattern, args.caseInsensitive === true ? 'i' : ''); }
    catch { throw workspaceError('workspace_grep_pattern_invalid'); }
    const base = await checkedPath(root, args.path || '.');
    assertAuthorized();
    const { files, truncated: walkTruncated } = await collectFiles(root, base, 5000);
    const matches = [];
    let filesScanned = 0, truncated = walkTruncated;
    for (const { rel, size } of files) {
      if (matches.length >= maxResults) { truncated = true; break; }
      if (includeRe && !includeRe.test(rel)) continue;
      if (size > 1024 * 1024) continue;
      assertAuthorized();
      const handle = await open(join(root, rel), constants.O_RDONLY | constants.O_NOFOLLOW);
      let content;
      try {
        const info = await handle.stat();
        if (!info.isFile() || info.nlink > 1) continue;
        const buffer = await handle.readFile();
        if (buffer.includes(0)) continue; // skip binary files
        content = buffer.toString('utf8');
      } finally { await handle.close(); }
      filesScanned++;
      const lines = content.split('\n');
      for (let i = 0; i < lines.length; i++) {
        if (matches.length >= maxResults) { truncated = true; break; }
        // Bound the tested string so a pathological pattern cannot run away on a
        // minified line; report a likewise-bounded snippet.
        const probe = lines[i].length > 2000 ? lines[i].slice(0, 2000) : lines[i];
        if (re.test(probe)) matches.push({ file: rel, line: i + 1, text: probe.length > 1000 ? probe.slice(0, 1000) : probe });
      }
    }
    return { path: relative(root, base) || '.', matches, filesScanned, truncated };
  }
  if (name === 'workspace_glob') {
    const pattern = String(args.pattern ?? '');
    if (!pattern || pattern.length > 1024) throw workspaceError('workspace_glob_invalid');
    const maxResults = args.maxResults ?? 500;
    if (!Number.isSafeInteger(maxResults) || maxResults < 1 || maxResults > 2000) throw workspaceError('workspace_glob_invalid');
    const base = await checkedPath(root, args.path || '.');
    assertAuthorized();
    const re = globToRegExp(pattern);
    const { files, truncated: walkTruncated } = await collectFiles(root, base, 5000);
    const matched = [];
    for (const { rel } of files) {
      // Match the glob as a shell would from the search base directory.
      const candidate = base === root ? rel : relative(base, join(root, rel));
      if (re.test(candidate)) matched.push(rel);
    }
    matched.sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
    return { path: relative(root, base) || '.', matches: matched.slice(0, maxResults), truncated: walkTruncated || matched.length > maxResults };
  }
}
