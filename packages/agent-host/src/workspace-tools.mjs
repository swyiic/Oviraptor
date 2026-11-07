import { constants } from 'node:fs';
import {recordVersion,privateDirectory} from './workspace-versions.mjs';
import {executeWorkspaceScript} from './workspace-execution.mjs';
import { saveFetchedUrl, assertWsTarget } from './cited-pages.mjs';
import { readFileRange } from './cited-files.mjs';
import { browsePage, runHostShell, sourceList, sourceSearch } from './chat-tools.mjs';
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
  {name:'workspace_run',description:'Execute a Node.js or Python script from this workspace in an OS-isolated copy, with no network or user environment, but able to read any granted source directory (a large cited file stays at its original path and is read from there). For a big file (multi-GB burp/pcap), stream it in chunks and set timeoutMs high (up to 1800000 ms / 30 min). Returns stdout, stderr, exit code and saved output files (each up to 16MB). Fix errors and run again explicitly; never claim tests passed on nonzero exit. Timeout/cancel does not publish output files.',inputSchema:{type:'object',properties:{path:{type:'string'},runtime:{enum:['node','python']},args:{type:'array',maxItems:32,items:{type:'string'}},timeoutMs:{type:'integer',minimum:100,maximum:1800000}},required:['path','runtime'],additionalProperties:false}},
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
    name: 'workspace_read_path',
    description: 'Read a byte window of a local file left at its original path. offset is a byte offset; length is at most 1048576. Does not copy the file. For a large file, prefer writing a script and running it with workspace_run to read/parse the whole file directly from its directory (an allowed read root) instead of reading it in windows here.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, offset: { type: 'integer', minimum: 0 }, length: { type: 'integer', minimum: 1, maximum: 1048576 } }, required: ['path'], additionalProperties: false },
  },
  {
    name: 'workspace_fetch',
    description: 'After the user confirms material access, fetch an http(s) URL and save it. Small responses land in the workspace. Responses over 8MB are stored outside the workspace and returned as absolutePath; read them with workspace_read_path. Do not ask the user to download files.',
    inputSchema: { type: 'object', properties: { url: { type: 'string' } }, required: ['url'], additionalProperties: false },
  },
  {
    name: 'workspace_http',
    description: 'Send one http(s) request with method, headers and body after material access is confirmed. Saves the response like workspace_fetch. Use this for a request the user pasted, including POST. Does not scan ranges.',
    inputSchema: { type: 'object', properties: { url: { type: 'string' }, method: { type: 'string' }, headers: { type: 'object', additionalProperties: { type: 'string' } }, body: { type: 'string' } }, required: ['url'], additionalProperties: false },
  },
  {
    name: 'workspace_browser',
    description: 'Open one http(s) page in headless Chrome after material access is confirmed. Records the requests the page actually makes and saves same-page script bodies under cited-web. Use this when encryption is loaded only after the page runs.',
    inputSchema: { type: 'object', properties: { url: { type: 'string' }, timeoutMs: { type: 'integer', minimum: 1000, maximum: 30000 } }, required: ['url'], additionalProperties: false },
  },
  {
    name: 'workspace_ws',
    description: 'After material access is confirmed, open one WebSocket (ws/wss) to an authorized/cited host, optionally send text frames, and capture the frames exchanged as evidence. Scope-locked to cited hosts and blocked from private/loopback addresses. Bounded: at most 30s, 100 frames, 2MB total; each frame capped to 64KB. Returns the captured frames (direction, type, text/base64, timing) to attach to record_finding. Does not retry or fuzz.',
    inputSchema: { type: 'object', properties: { url: { type: 'string' }, send: { type: 'array', maxItems: 32, items: { type: 'string' } }, headers: { type: 'object', additionalProperties: { type: 'string' } }, timeoutMs: { type: 'integer', minimum: 100, maximum: 30000 } }, required: ['url'], additionalProperties: false },
  },
  {
    name: 'workspace_probe_sql',
    description: 'After access is allowed, compare one GET parameter. Sends the original URL, then up to 6 values supplied in `values`, and saves each response. Use this when the user judged a SQL comparison reasonable and told you to run it. Do not invent a range scan.',
    inputSchema: { type: 'object', properties: { url: { type: 'string' }, param: { type: 'string' }, values: { type: 'array', maxItems: 6, items: { type: 'string' } } }, required: ['url', 'param', 'values'], additionalProperties: false },
  },
  {
    name: 'workspace_probe_command',
    description: 'After access is allowed, send one request that places `command` in a query parameter or header on the user-authorized URL. Saves the response. Do not run the command on this machine; use workspace_shell for local commands.',
    inputSchema: { type: 'object', properties: { url: { type: 'string' }, method: { type: 'string' }, param: { type: 'string' }, header: { type: 'string' }, command: { type: 'string' } }, required: ['url', 'command'], additionalProperties: false },
  },
  {
    name: 'workspace_source_list',
    description: 'List one directory the user named, after access is allowed. Does not copy the tree.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'], additionalProperties: false },
  },
  {
    name: 'workspace_source_search',
    description: 'Search a literal string under a directory the user named, after access is allowed. Skips symlinks and files over 8MB. Returns a short snippet and byte offset.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, query: { type: 'string' } }, required: ['path', 'query'], additionalProperties: false },
  },
  {
    name: 'workspace_chain_read',
    description: 'Read this conversation analysis chain. The next action must use the latest fact whose opensNext is true. File identification and file excerpts are part of the same chain.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
  },
  {
    name: 'workspace_chain_add',
    description: 'Record one typed fact on the chain after you have evidence. factType is request_captured, crypto_function, plaintext_field, identity, endpoint, internal_address, file_identified, or file_excerpt. source is the file path with offset, or the saved request path. Set opensNext false when this fact does not unlock another step. Do this for file analysis as well as pages.',
    inputSchema: { type: 'object', properties: { factType: { type: 'string' }, factValue: { type: 'string' }, source: { type: 'string' }, opensNext: { type: 'boolean' } }, required: ['factType', 'factValue', 'source'], additionalProperties: false },
  },
  {
    name: 'workspace_shell',
    description: 'Run one shell command on this machine after material access is confirmed. Working directory is the conversation workspace. Timeout at most 60 seconds. Output is capped. Use it for curl, formatters and local inspection; do not claim a command succeeded without its exit code.',
    inputSchema: { type: 'object', properties: { command: { type: 'string' }, timeoutMs: { type: 'integer', minimum: 100, maximum: 60000 } }, required: ['command'], additionalProperties: false },
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
  // Outward/dangerous tools — reach the network, a browser, or the host shell. A
  // distinct, more-gated class so that granting sandboxed ANALYSIS (read + run +
  // write, all confined/no-net) never also grants these. The authorization wall.
  if (toolName === 'workspace_fetch' || toolName === 'workspace_http' || toolName === 'workspace_browser' || toolName === 'workspace_ws' || toolName === 'workspace_shell' || toolName === 'workspace_probe_sql' || toolName === 'workspace_probe_command') return 'workspace.network';
  if (toolName === 'workspace_list' || toolName === 'workspace_read' || toolName === 'workspace_grep' || toolName === 'workspace_glob' || toolName === 'workspace_read_path' || toolName === 'workspace_source_list' || toolName === 'workspace_source_search' || toolName === 'workspace_chain_read' || toolName === 'workspace_chain_add') return 'workspace.read';
  throw workspaceError('workspace_tool_unknown');
}

function workspaceError(code, message = code) { return Object.assign(Error(message), { code, retryable: false }); }

const WS_MAX_MS = 30000;
const WS_MAX_FRAMES = 100;
const WS_MAX_FRAME_BYTES = 64 * 1024;
const WS_MAX_TOTAL_BYTES = 2 * 1024 * 1024;

// Open one WebSocket, optionally send text frames, and capture the exchange,
// bounded by time / frame count / total bytes. Host scope + SSRF are already
// enforced by assertWsTarget before this is called; this only moves data and
// caps it. The captured frames are evidence, never an authorization.
async function captureWebSocket(url, { send = [], headers, timeoutMs } = {}) {
  if (typeof WebSocket !== 'function') throw workspaceError('workspace_ws_unsupported');
  const budget = Math.min(Math.max(Number(timeoutMs) || 10000, 100), WS_MAX_MS);
  const toSend = Array.isArray(send) ? send.filter(frame => typeof frame === 'string').slice(0, 32) : [];
  const frames = [];
  let totalBytes = 0;
  let truncated = false;
  let sentCount = 0;
  let opened = false;
  const started = Date.now();
  return await new Promise(resolve => {
    let ws;
    let timer;
    let settled = false;
    const finish = extra => {
      if (settled) return; settled = true;
      clearTimeout(timer);
      try { if (ws) ws.close(); } catch { /* ignore */ }
      resolve({ url: String(url), opened, frames, sent: sentCount, truncated, ...extra });
    };
    try {
      ws = headers && typeof headers === 'object' ? new WebSocket(url, { headers }) : new WebSocket(url);
    } catch { resolve({ url: String(url), opened: false, frames, sent: 0, truncated: false, error: 'ws_open_failed' }); return; }
    ws.binaryType = 'arraybuffer';
    timer = setTimeout(() => finish({ closed: 'timeout' }), budget);
    ws.addEventListener('open', () => {
      opened = true;
      for (const text of toSend) {
        try { ws.send(text); sentCount++; frames.push({ dir: 'send', type: 'text', at: Date.now() - started, text: text.slice(0, WS_MAX_FRAME_BYTES), bytes: Buffer.byteLength(text) }); }
        catch { /* a send that fails is not fatal */ }
      }
    });
    ws.addEventListener('message', ev => {
      if (frames.length >= WS_MAX_FRAMES) { truncated = true; finish({ closed: 'frame_cap' }); return; }
      let frame;
      if (typeof ev.data === 'string') {
        const bytes = Buffer.byteLength(ev.data);
        frame = { dir: 'recv', type: 'text', at: Date.now() - started, text: ev.data.slice(0, WS_MAX_FRAME_BYTES), bytes };
        totalBytes += bytes;
      } else {
        const buf = Buffer.from(ev.data);
        frame = { dir: 'recv', type: 'binary', at: Date.now() - started, base64: buf.subarray(0, WS_MAX_FRAME_BYTES).toString('base64'), bytes: buf.length };
        totalBytes += buf.length;
      }
      if (frame.bytes > WS_MAX_FRAME_BYTES) { frame.frameTruncated = true; truncated = true; }
      frames.push(frame);
      if (totalBytes >= WS_MAX_TOTAL_BYTES) { truncated = true; finish({ closed: 'byte_cap' }); }
    });
    ws.addEventListener('close', ev => finish({ closeCode: ev?.code, closeReason: String(ev?.reason || '').slice(0, 200), closed: 'remote' }));
    ws.addEventListener('error', () => { if (!opened) finish({ error: 'ws_open_failed' }); });
  });
}

function lineWindows(line) {
  const width = 8000;
  if (line.length <= width) return [line];
  const windows = [];
  for (let start = 0; start < line.length && windows.length < 64; start += width / 2) windows.push(line.slice(start, start + width));
  return windows;
}

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
async function runTool(root, name, args = {}, { anchor, historyRoot, context={}, executionRoot, assertAuthorized = () => {}, allowedHosts, fetchImpl, lookupImpl, materialOpen = false, materialRoot = '', readRoots = [], readChain = () => [], recordHop, onProgress } = {}) {
  if (!WORKSPACE_TOOL_SPECS.some(tool => tool.name === name)) throw workspaceError('workspace_tool_unknown');
  root = await workspaceRoot(root, anchor, assertAuthorized);
  assertAuthorized();
  if(name==='workspace_run')return executeWorkspaceScript(root,args,{executionRoot,assertAuthorized,readRoots,onProgress,write:async(path,content)=>runTool(root,'workspace_write',{path,content},{anchor,historyRoot,context,assertAuthorized})});
  if (name === 'workspace_read_path') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    try { return await readFileRange(args.path, args.offset, args.length); }
    catch (error) {
      const code = error.code === 'range_invalid' || error.code === 'range_not_file' ? 'workspace_read_path_invalid' : 'workspace_read_path_failed';
      throw workspaceError(code);
    }
  }
  if (name === 'workspace_fetch') {
    const target = args.url;
    if (typeof target !== 'string' || !target.trim()) throw workspaceError('workspace_fetch_invalid');
    try {
      return await saveFetchedUrl(root, target, { allowedHosts, openNetwork: materialOpen, materialRoot, fetchImpl, lookupImpl });
    } catch (error) {
      const code = error.code === 'blocked_address' ? 'workspace_fetch_blocked'
        : error.code === 'fetch_host_not_allowed' || error.code === 'redirect_left_host' ? 'workspace_fetch_host_denied'
        : error.code === 'fetch_url_invalid' ? 'workspace_fetch_invalid'
        : 'workspace_fetch_failed';
      throw workspaceError(code);
    }
  }
  if (name === 'workspace_http') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    if (typeof args.url !== 'string' || !args.url.trim()) throw workspaceError('workspace_fetch_invalid');
    try {
      return await saveFetchedUrl(root, args.url, { allowedHosts, openNetwork: true, materialRoot, fetchImpl, lookupImpl, method: args.method, headers: args.headers, body: args.body });
    } catch (error) {
      const code = error.code === 'blocked_address' ? 'workspace_fetch_blocked' : 'workspace_fetch_failed';
      throw workspaceError(code);
    }
  }
  if (name === 'workspace_ws') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    if (typeof args.url !== 'string' || !args.url.trim()) throw workspaceError('workspace_ws_invalid');
    let url;
    try {
      // openNetwork:false → scope-locked to cited/authorized hosts + SSRF wall.
      url = await assertWsTarget(args.url, { allowedHosts, openNetwork: false, lookupImpl });
    } catch (error) {
      const code = error.code === 'blocked_address' ? 'workspace_ws_blocked'
        : error.code === 'ws_host_not_allowed' ? 'workspace_ws_host_denied'
        : error.code === 'dns' ? 'workspace_ws_unreachable'
        : 'workspace_ws_invalid';
      throw workspaceError(code);
    }
    try { return await captureWebSocket(url.href, { send: args.send, headers: args.headers, timeoutMs: args.timeoutMs }); }
    catch (error) { throw workspaceError(/^workspace_[a-z_]+$/u.test(error?.code ?? '') ? error.code : 'workspace_ws_failed'); }
  }
  if (name === 'workspace_browser') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    try { return await browsePage(args.url, { workspaceRoot: root, timeoutMs: args.timeoutMs }); }
    catch (error) { throw workspaceError(error.code === 'browser_missing' ? 'workspace_browser_missing' : error.code === 'browser_blocked' ? 'workspace_fetch_blocked' : 'workspace_browser_failed'); }
  }
  if (name === 'workspace_shell') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    try { return await runHostShell(args.command, { cwd: root, timeoutMs: args.timeoutMs }); }
    catch { throw workspaceError('workspace_shell_failed'); }
  }
  if (name === 'workspace_source_list') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    try { return await sourceList(args.path); }
    catch { throw workspaceError('workspace_source_failed'); }
  }
  if (name === 'workspace_source_search') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    try { return await sourceSearch(args.path, args.query); }
    catch { throw workspaceError('workspace_source_failed'); }
  }
  if (name === 'workspace_probe_sql') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    const param = String(args.param || '');
    const values = Array.isArray(args.values) ? args.values.filter(item => typeof item === 'string').slice(0, 6) : [];
    let page;
    try { page = new URL(String(args.url || '')); }
    catch { throw workspaceError('workspace_fetch_invalid'); }
    if (!param) throw workspaceError('workspace_fetch_invalid');
    const shots = [];
    const send = async (target, value) => {
      const saved = await saveFetchedUrl(root, target, { openNetwork: true, materialRoot, fetchImpl, lookupImpl });
      shots.push({ value: String(value).slice(0, 180), status: saved.status, path: saved.path || saved.absolutePath || '', bytes: saved.bytes });
    };
    await send(page.href, page.searchParams.get(param) ?? '');
    for (const value of values) {
      const next = new URL(page.href);
      next.searchParams.set(param, value);
      await send(next.href, value);
    }
    return { url: page.href, param, shots };
  }
  if (name === 'workspace_probe_command') {
    if (!materialOpen) throw workspaceError('workspace_material_closed');
    const command = String(args.command || '').slice(0, 512);
    if (!command) throw workspaceError('workspace_fetch_invalid');
    let page;
    try { page = new URL(String(args.url || '')); }
    catch { throw workspaceError('workspace_fetch_invalid'); }
    if (args.param) page.searchParams.set(String(args.param), command);
    const headers = args.header ? { [String(args.header)]: command } : undefined;
    try {
      return await saveFetchedUrl(root, page.href, { openNetwork: true, materialRoot, fetchImpl, lookupImpl, method: args.method, headers });
    } catch { throw workspaceError('workspace_fetch_failed'); }
  }
  if (name === 'workspace_chain_read') return { hops: readChain() };
  if (name === 'workspace_chain_add') {
    if (typeof recordHop !== 'function') throw workspaceError('workspace_chain_failed');
    try { return recordHop({ factType: args.factType, factValue: args.factValue, source: args.source, opensNext: args.opensNext, action: 'judge' }); }
    catch (error) { throw workspaceError(error.code === 'chain_fact_invalid' ? 'workspace_chain_invalid' : 'workspace_chain_failed'); }
  }
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
      if (size > 8 * 1024 * 1024) { truncated = true; continue; }
      assertAuthorized();
      const handle = await open(join(root, rel), constants.O_RDONLY | constants.O_NOFOLLOW);
      let content;
      try {
        const info = await handle.stat();
        if (!info.isFile() || info.nlink > 1) continue;
        const buffer = await handle.readFile();
        if (buffer.includes(0)) continue;
        content = buffer.toString('utf8');
      } finally { await handle.close(); }
      filesScanned++;
      const lines = content.split('\n');
      for (let i = 0; i < lines.length; i++) {
        if (matches.length >= maxResults) { truncated = true; break; }
        const windows = lineWindows(lines[i]);
        for (const probe of windows) {
          if (matches.length >= maxResults) { truncated = true; break; }
          re.lastIndex = 0;
          const found = re.exec(probe);
          if (!found) continue;
          const at = found.index;
          matches.push({ file: rel, line: i + 1, text: probe.slice(Math.max(0, at - 80), at + 160) });
        }
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
