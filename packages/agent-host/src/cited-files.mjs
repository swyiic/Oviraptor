import { constants } from 'node:fs';
import { lstat, mkdir, open, writeFile } from 'node:fs/promises';
import { basename, isAbsolute, join, relative, resolve, sep } from 'node:path';

// Files up to this size are copied into the workspace; larger ones stay at
// their original path and are read directly by a workspace_run script (their
// directory is an allowed read root in full / material-open mode), never copied.
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_FILES = 8;

function pathChar(char) {
  return char !== ' ' && char !== '\n' && char !== '\r' && char !== '\t'
    && char !== '"' && char !== "'" && char !== '<' && char !== '>'
    && char !== '|' && char !== '*' && char !== '?' && char !== '\\' && char !== '\0';
}

// Absolute paths written in the user message, including one glued to Chinese
// with no space. A trailing sentence is dropped only when a shorter prefix is
// the actual file.
export function citedPathTokens(text) {
  const value = String(text ?? '');
  const tokens = [];
  for (let index = 0; index < value.length; index++) {
    if (value[index] !== '/') continue;
    // Chinese may be glued to the path, as in "吗/Users/...". ASCII path
    // characters mean this slash is already inside a token.
    if (index > 0 && /[A-Za-z0-9._+-]/u.test(value[index - 1])) continue;
    let end = index + 1;
    while (end < value.length && pathChar(value[end])) end++;
    const token = value.slice(index, end).replace(/[。，、；;,.!?）)】\]]+$/u, '');
    if (token.length > 1 && token.length <= 1024 && !tokens.includes(token)) tokens.push(token);
  }
  return tokens;
}

async function longestFile(token) {
  let best = '';
  for (let end = token.length; end > 1; end--) {
    const candidate = token.slice(0, end);
    if (!isAbsolute(candidate) || candidate.split('/').includes('..')) continue;
    try {
      const info = await lstat(candidate);
      if (info.isSymbolicLink() || !info.isFile()) continue;
      best = candidate;
      break;
    } catch {
      // Keep shrinking. The token may include words typed after the path.
    }
  }
  return best;
}

function safeBase(file) {
  const base = basename(file);
  if (!base || base === '.' || base === '..' || base.includes('/') || base.includes('\\')) return '';
  return base;
}

async function unusedCopy(dir, base) {
  for (let index = 0; index < 20; index++) {
    const name = index === 0 ? base : `${index + 1}-${base}`;
    const target = join(dir, name);
    try { await lstat(target); }
    catch (error) {
      if (error.code === 'ENOENT') return { target, name };
      throw error;
    }
  }
  return null;
}

export async function readFileRange(file, offset, length) {
  if (typeof file !== 'string' || !isAbsolute(file) || file.includes('\0')) {
    throw Object.assign(Error('range_invalid'), { code: 'range_invalid' });
  }
  const start = offset ?? 0;
  const want = length ?? 65536;
  if (!Number.isSafeInteger(start) || start < 0 || !Number.isSafeInteger(want) || want < 1 || want > 1048576) {
    throw Object.assign(Error('range_invalid'), { code: 'range_invalid' });
  }
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (!info.isFile()) throw Object.assign(Error('range_not_file'), { code: 'range_not_file' });
    if (start > info.size) throw Object.assign(Error('range_invalid'), { code: 'range_invalid' });
    const count = Math.min(want, info.size - start);
    const buffer = Buffer.alloc(count);
    await handle.read(buffer, 0, count, start);
    let encoding = 'utf8';
    let content = buffer.toString('utf8');
    try { new TextDecoder('utf-8', { fatal: true }).decode(buffer); }
    catch { encoding = 'hex'; content = buffer.toString('hex'); }
    return { path: file, size: info.size, offset: start, bytes: count, encoding, content };
  } finally { await handle.close(); }
}

export async function importCitedFiles(text, workspaceRoot) {
  const root = resolve(workspaceRoot);
  const imported = [];
  const external = [];
  const skipped = [];
  const seen = new Set();
  for (const token of citedPathTokens(text)) {
    if (imported.length + external.length + skipped.length >= MAX_FILES) break;
    const file = await longestFile(token);
    if (!file) {
      if (token.startsWith('/Users/') || token.startsWith('/private/') || token.startsWith('/Volumes/') || token.startsWith('/tmp/')) {
        skipped.push({ source: token, reason: 'missing' });
      }
      continue;
    }
    if (seen.has(file)) continue;
    seen.add(file);
    const rel = relative(root, file);
    if (rel === '' || (!rel.startsWith(`..${sep}`) && rel !== '..' && !isAbsolute(rel))) {
      skipped.push({ source: file, reason: 'already_in_workspace' });
      continue;
    }
    const base = safeBase(file);
    if (!base) {
      skipped.push({ source: file, reason: 'name_rejected' });
      continue;
    }
    let handle;
    try {
      handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
      const info = await handle.stat();
      if (!info.isFile() || info.nlink < 1) {
        skipped.push({ source: file, reason: 'not_a_file' });
        continue;
      }
      if (info.size > MAX_BYTES) {
        external.push({ source: file, bytes: info.size });
        continue;
      }
      const bytes = await handle.readFile();
      if (bytes.length !== info.size) {
        skipped.push({ source: file, reason: 'read_changed' });
        continue;
      }
      await mkdir(join(root, 'cited'), { recursive: true, mode: 0o700 });
      const dest = await unusedCopy(join(root, 'cited'), base);
      if (!dest) {
        skipped.push({ source: file, reason: 'name_exhausted' });
        continue;
      }
      await writeFile(dest.target, bytes, { mode: 0o600, flag: 'wx' });
      imported.push({ source: file, path: `cited/${dest.name}`, bytes: bytes.length });
    } catch {
      skipped.push({ source: file, reason: 'unreadable' });
    } finally {
      await handle?.close();
    }
  }
  return { imported, external, skipped, note: citedNote(imported, skipped, external) };
}

function citedNote(imported, skipped, external = []) {
  if (!imported.length && !skipped.length && !external.length) return '';
  const lines = ['Host: 用户点名的本机文件按下述方式打开。小文件已复制进工作区。大文件留在原路径——它所在目录已是允许读取的根，直接写脚本用 workspace_run 读取并解析原文件，不要整份复制，也不要用 workspace_read_path 一段段手读整份大文件。'];
  for (const file of imported) {
    lines.push(`- 已复制 ${file.path}（${file.bytes} 字节）。文本用 workspace_read；二进制或未知后缀用 workspace_run 跑 Python，看文件头、大小和可读字符串。`);
  }
  for (const file of external) {
    lines.push(`- 大文件留在原路径 ${file.source}（${file.bytes} 字节）。它所在目录已是允许读取的根：直接写一个脚本(Node/Python)用 workspace_run 读取并解析原文件、一次跑完（如解析 .burp/.har、批量解密响应），不要用 workspace_read_path 一段段手读整份大文件——那会迅速耗尽本轮步数。只需看小窗口时 workspace_read_path 的 length 最大 1048576。`);
  }
  for (const file of skipped) {
    if (file.reason === 'already_in_workspace') lines.push('- 该路径已经在工作区内，直接用相对路径读取。');
    else lines.push('- 有一个点名路径没有复制（不是普通文件、是符号链接，或读不到）。说明没打开，不要编造内容。');
  }
  return lines.join('\n');
}
