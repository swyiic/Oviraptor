import { constants } from 'node:fs';
import { mkdir, readdir, lstat, realpath, open, rename, unlink } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

const MAX_READ = 64 * 1024;
const MAX_WRITE = 256 * 1024;

export const WORKSPACE_TOOL_SPECS = [
  {
    name: 'workspace_list',
    description: 'List files in the conversation workspace. Read-only. Stays inside that directory.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, additionalProperties: false },
  },
  {
    name: 'workspace_read',
    description: 'Read a text file in the conversation workspace.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'], additionalProperties: false },
  },
  {
    name: 'workspace_write',
    description: 'Create or overwrite a text file in the conversation workspace. Use this to write scripts and tools the user asked for.',
    inputSchema: { type: 'object', properties: { path: { type: 'string' }, content: { type: 'string' } }, required: ['path', 'content'], additionalProperties: false },
  },
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

export async function runWorkspaceTool(root, name, args = {}, { anchor, assertAuthorized = () => {} } = {}) {
  // cwd and an interpreter allowlist are not a sandbox. Keep this legacy name
  // fail-closed until a separately verified execution backend is available.
  if (name === 'workspace_run') throw workspaceError('workspace_execution_unsupported', 'workspace_execution_unsupported: Script execution is unavailable because this host has no verified isolated execution backend. Files can still be read and written.');
  if (!WORKSPACE_TOOL_SPECS.some(tool => tool.name === name)) throw workspaceError('workspace_tool_unknown');
  root = await workspaceRoot(root, anchor, assertAuthorized);
  assertAuthorized();
  if (name === 'workspace_list') {
    const dir = await checkedPath(root, args.path || '.');
    assertAuthorized();
    const items = await readdir(dir, { withFileTypes: true });
    return { path: relative(root, dir) || '.', items: items.slice(0, 200).map(item => ({ name: item.name, kind: item.isSymbolicLink() ? 'symlink' : item.isDirectory() ? 'dir' : 'file' })) };
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
      const buffer = Buffer.alloc(MAX_READ + 1);
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
      return { path: relative(root, file), truncated: bytesRead > MAX_READ, content: buffer.subarray(0, Math.min(bytesRead, MAX_READ)).toString('utf8') };
    } finally { await handle.close(); }
  }
  if (name === 'workspace_write') {
    const content = String(args.content ?? '');
    if (Buffer.byteLength(content) > MAX_WRITE) throw workspaceError('workspace_file_too_large');
    const file = await checkedPath(root, args.path, { createParents: true, assertAuthorized });
    if (file === root) throw workspaceError('workspace_not_a_file');
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
    return { path: relative(root, file), bytes: Buffer.byteLength(content) };
  }
}
