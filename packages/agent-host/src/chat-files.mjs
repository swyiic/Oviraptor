import { constants } from 'node:fs';
import { lstat, realpath, open } from 'node:fs/promises';
import { basename, extname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { hostError, positiveId } from './store.mjs';

const MAX_PREVIEW = 64 * 1024;
const MAX_RECEIPTS = 1000;
const TEXT = new Set(['.txt', '.md', '.markdown', '.csv', '.tsv', '.json', '.jsonl', '.log', '.yaml', '.yml', '.toml', '.ini', '.conf', '.xml', '.html', '.htm', '.svg', '.css', '.scss', '.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx', '.vue', '.rs', '.py', '.sh', '.bash', '.sql', '.go', '.java', '.c', '.h', '.cpp', '.rb', '.php', '.ps1']);
const OPEN = new Set(['.txt', '.md', '.markdown', '.csv', '.tsv', '.json', '.log', '.yaml', '.yml', '.toml', '.pdf', '.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp']);
const language = path => ({ '.md': 'markdown', '.json': 'json', '.jsonl': 'json', '.js': 'javascript', '.mjs': 'javascript', '.ts': 'typescript', '.rs': 'rust', '.py': 'python', '.sh': 'bash', '.html': 'html', '.htm': 'html', '.yml': 'yaml' }[extname(path).toLowerCase()] ?? extname(path).slice(1));
const fail = (code, status = 400) => { throw hostError(`host_chat_file_${code}`, status, { retryable: false }); };

function localPath(value) {
  if (typeof value !== 'string' || !value || value.length > 2048 || isAbsolute(value) || /[\\\u0000-\u001f\u007f]/u.test(value)) fail('path_denied');
  const parts = value.split('/');
  if (parts.some(part => !part || part === '.' || part === '..')) fail('path_denied');
  return parts.join('/');
}

async function checked(root, parts, { missing = false } = {}) {
  let current = root;
  for (const [index, part] of parts.entries()) {
    current = join(current, part);
    let info;
    try { info = await lstat(current); }
    catch (error) {
      if (missing && error.code === 'ENOENT') return { path: join(root, ...parts), exists: false };
      if (error.code === 'ENOENT') fail('unavailable', 404);
      throw error;
    }
    if (info.isSymbolicLink() || index < parts.length - 1 && !info.isDirectory()) fail('path_denied');
    const canonical = await realpath(current), rel = relative(root, canonical);
    if (rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel)) fail('path_denied');
  }
  return { path: current, exists: true, info: await lstat(current) };
}

/** User-facing file discovery uses durable successful tool receipts, never model links. */
export class ChatFiles {
  constructor(host) { this.host = host; }
  #scope(conversationId, projectId) {
    const id = positiveId(projectId), conversation = this.host.store.conversation(conversationId, id);
    return { conversationId: conversation.id, projectId: id, root: resolve(this.host.dataDir, 'workspaces', String(id), 'agent') };
  }
  #receipts(scope) {
    const rows = this.host.store.db.prepare(`SELECT r.id,r.body,r.created_at AS generatedAt,
      json_extract(e.value,'$.runId') AS runId FROM conversation_tasks t,
      json_each(t.payload,'$.evidence') e JOIN tool_records r ON r.id=json_extract(e.value,'$.rawRecordId')
      WHERE t.conversation_id=? AND r.conversation_id=? AND r.project_id=? AND r.visibility='available'
      AND json_extract(e.value,'$.toolName')='workspace_write' AND json_extract(e.value,'$.status')='completed'
      AND COALESCE(json_extract(e.value,'$.outcomeUnknown'),0)=0 ORDER BY r.rowid DESC LIMIT ?`)
      .all(scope.conversationId, scope.conversationId, scope.projectId, MAX_RECEIPTS + 1);
    const files = new Map();
    for (const row of rows.slice(0, MAX_RECEIPTS)) {
      try {
        const result = JSON.parse(row.body);
        if (result?.isError === true) continue;
        const receipt = result?.structuredContent ?? JSON.parse(result?.content?.find(item => item.type === 'text')?.text ?? 'null');
        if (!Number.isSafeInteger(receipt?.bytes) || receipt.bytes < 0 || typeof receipt.workspace !== 'string' || resolve(receipt.workspace) !== scope.root) continue;
        const path = localPath(receipt.path);
        if (!files.has(path)) files.set(path, { path, generatedAt: row.generatedAt, runId: row.runId, recordId: row.id });
      } catch { /* A truncated or unbound legacy record is not a verified file. */ }
    }
    return { files, truncated: rows.length > MAX_RECEIPTS };
  }
  async #root(scope) {
    const anchor = await realpath(this.host.dataDir);
    return checked(anchor, ['workspaces', String(scope.projectId), 'agent'], { missing: true });
  }
  async workspace(conversationId, projectId) {
    const scope = this.#scope(conversationId, projectId), root = await this.#root(scope);
    if (root.exists && !root.info.isDirectory()) fail('path_denied');
    return { projectId: scope.projectId, conversationId: scope.conversationId, path: scope.root, exists: root.exists, shared: true,
      description: '同一项目的会话共用此工作区；文件列表仅包含本会话成功写入的文件。' };
  }
  async list(conversationId, projectId) {
    const scope = this.#scope(conversationId, projectId), { files, truncated } = this.#receipts(scope), root = await this.#root(scope);
    const items = []; let unavailable = 0;
    if (root.exists && !root.info.isDirectory()) fail('path_denied');
    for (const receipt of files.values()) {
      if (!root.exists) { unavailable++; continue; }
      try {
        const file = await checked(root.path, receipt.path.split('/'));
        if (!file.info.isFile() || file.info.nlink > 1) { unavailable++; continue; }
        const extension = extname(receipt.path).toLowerCase();
        items.push({ ...receipt, name: basename(receipt.path), absolutePath: file.path, size: file.info.size,
          modifiedAt: file.info.mtime.toISOString(), previewable: TEXT.has(extension), openable: OPEN.has(extension) && (file.info.mode & 0o111) === 0 });
      } catch (error) {
        if (error.code?.startsWith('host_chat_file_') || error.code === 'ENOENT') unavailable++;
        else throw error;
      }
    }
    return { projectId: scope.projectId, conversationId: scope.conversationId, workspacePath: scope.root, shared: true, items, truncated, unavailable };
  }
  async preview(conversationId, projectId, inputPath) {
    const scope = this.#scope(conversationId, projectId), path = localPath(inputPath), { files } = this.#receipts(scope);
    if (!files.has(path)) fail('unavailable', 404);
    if (!TEXT.has(extname(path).toLowerCase())) fail('preview_unsupported', 415);
    const root = await this.#root(scope);
    if (!root.exists || !root.info.isDirectory()) fail('unavailable', 404);
    const target = await checked(root.path, path.split('/'));
    const handle = await open(target.path, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const info = await handle.stat();
      if (!info.isFile() || info.nlink > 1) fail('path_denied');
      const bytes = Buffer.alloc(MAX_PREVIEW + 1), { bytesRead } = await handle.read(bytes, 0, bytes.length, 0);
      if (bytes.subarray(0, bytesRead).includes(0)) fail('preview_unsupported', 415);
      return { path, name: basename(path), size: info.size, modifiedAt: info.mtime.toISOString(),
        content: bytes.subarray(0, Math.min(bytesRead, MAX_PREVIEW)).toString('utf8'), truncated: bytesRead > MAX_PREVIEW, language: language(path) };
    } finally { await handle.close(); }
  }
}
