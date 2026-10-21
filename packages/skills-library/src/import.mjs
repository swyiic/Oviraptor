import { constants } from 'node:fs';
import { lstat, open, readdir, realpath } from 'node:fs/promises';
import { basename, dirname, join, resolve, sep } from 'node:path';
import { createHash } from 'node:crypto';
import { gunzipSync } from 'node:zlib';

export const LIMITS = Object.freeze({ files: 512, fileBytes: 1_048_576, totalBytes: 16_777_216, manifestBytes: 25_165_824, skills: 128, depth: 16, resourcePageBytes: 8192 });
export const notices = () => ['Packages provide instructions and reference files only. Imported scripts are inert; shell execution, browser automation, external integrations, and other tools mentioned by a skill are not installed or authorized by this package.'];
export const error = (code, status = 400) => Object.assign(new Error(code), { code: code.split(':')[0], status });
export const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const control = /[\u0000-\u001f\u007f-\u009f]/u;
export function checkedPath(path) {
  if (typeof path !== 'string' || !path || Buffer.byteLength(path) > 512 || path.includes('\\') || path.includes(':') || control.test(path)
    || path.startsWith('/') || path.split('/').some(part => !part || part === '.' || part === '..' || /[ .]$/u.test(part))) throw error('skill_package_unsafe_path');
  return path;
}
export function utf8(bytes) {
  try { return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes); }
  catch { return null; }
}
function scalar(value) {
  const text = value.trim();
  if (text.startsWith('"') && text.endsWith('"')) { try { const parsed = JSON.parse(text); return typeof parsed === 'string' ? parsed : ''; } catch { return ''; } }
  if (text.startsWith("'") && text.endsWith("'") && text.length >= 2) return text.slice(1, -1).replaceAll("''", "'");
  return /^[!&*{[]/u.test(text) ? '' : text;
}
function metadata(instructions, entry) {
  if (!instructions.trim() || instructions.includes('\0') || [...instructions].length > 30_000) throw error(`skill_package_invalid_instructions:${entry}`);
  let name = '', description = '';
  const lines = instructions.replace(/^\uFEFF/u, '').split(/\r?\n/u);
  if (lines[0]?.trim() === '---') {
    let closed = false;
    for (let index = 1; index < lines.length; index++) {
      const line = lines[index]; if (line.trim() === '---') { closed = true; break; }
      const colon = line.indexOf(':'); if (colon < 0) continue;
      const key = line.slice(0, colon); if (!['name', 'description'].includes(key)) continue;
      const value = line.slice(colon + 1).trim(); let text;
      if (['>', '>-', '|', '|-'].includes(value)) {
        const pieces = [];
        while (index + 1 < lines.length && (/^[ \t]/u.test(lines[index + 1]) || !lines[index + 1])) pieces.push(lines[++index].trim());
        text = pieces.join(value.startsWith('>') ? ' ' : '\n').trim();
      } else text = scalar(value);
      if (key === 'name') name = text; else description = text;
    }
    if (!closed) throw error(`skill_package_invalid_frontmatter:${entry}`);
  }
  if (!name) name = dirname(entry) === '.' ? 'custom-skill' : basename(dirname(entry));
  if (!name.trim() || [...name].length > 80 || control.test(name) || Buffer.byteLength(description) > 8192) throw error(`skill_package_invalid_metadata:${entry}`);
  return { name: name.trim(), description };
}
function validate(pkg) {
  if (!pkg.files.length || pkg.files.length > LIMITS.files) throw error('skill_package_file_limit');
  pkg.files.sort((a, b) => Buffer.compare(Buffer.from(a.path), Buffer.from(b.path)));
  const digest = createHash('sha256'), paths = new Set(), names = new Set(); let total = 0;
  for (const value of [pkg.packageKey, pkg.name, pkg.version, pkg.source, pkg.license]) {
    if (typeof value !== 'string' || Buffer.byteLength(value) > 2048 || control.test(value)) throw error('skill_package_invalid_manifest');
    digest.update(value).update('\0');
  }
  pkg.skills = [];
  for (const file of pkg.files) {
    checkedPath(file.path);
    if (paths.has(file.path.toLowerCase())) throw error('skill_package_duplicate_path'); paths.add(file.path.toLowerCase());
    if (file.bytes.length > LIMITS.fileBytes) throw error(`skill_package_file_too_large:${file.path}`);
    total += file.bytes.length; if (total > LIMITS.totalBytes) throw error('skill_package_byte_limit');
    if (hash(file.bytes) !== file.sha256) throw error(`skill_package_hash_mismatch:${file.path}`);
    digest.update(file.path).update('\0').update(file.sha256).update('\0');
    if (file.path === 'SKILL.md' || file.path.endsWith('/SKILL.md')) {
      const instructions = utf8(file.bytes); if (instructions === null) throw error(`skill_package_non_utf8_instructions:${file.path}`);
      const parsed = metadata(instructions, file.path);
      if (names.has(parsed.name.toLowerCase())) throw error('skill_package_duplicate_skill_name'); names.add(parsed.name.toLowerCase());
      pkg.skills.push({ ...parsed, entryPath: file.path, instructions });
    }
  }
  if (!pkg.skills.length || pkg.skills.length > LIMITS.skills) throw error('skill_package_no_skills_or_limit');
  pkg.totalBytes = total; pkg.digest = digest.digest('hex'); pkg.id = pkg.builtin ? pkg.packageKey : `local-${pkg.digest.slice(0, 32)}`;
  return pkg;
}
async function noLink(path) {
  let info; try { info = await lstat(path, { bigint: true }); } catch { throw error('skill_package_path_unavailable'); }
  if (info.isSymbolicLink()) throw error('skill_package_symlink_forbidden');
  return info;
}
const identity = info => [info.dev, info.ino, info.size, info.mtimeNs, info.ctimeNs].join(':');
async function readBounded(path, maximum, root) {
  const parents = [];
  if (root) {
    let current = root;
    parents.push([current, await noLink(current)]);
    for (const part of dirname(path).slice(root.length).split(sep).filter(Boolean)) { current = join(current, part); parents.push([current, await noLink(current)]); }
  }
  const before = await noLink(path);
  if (!before.isFile()) throw error('skill_package_special_file_forbidden');
  if (before.size > BigInt(maximum)) throw error('skill_package_file_too_large');
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0)).catch(() => { throw error('skill_package_safe_open_failed'); });
  try {
    const opened = await handle.stat({ bigint: true }); if (identity(before) !== identity(opened)) throw error('skill_package_source_changed');
    if (root) {
      const actual = await realpath(process.platform === 'linux' ? `/proc/self/fd/${handle.fd}` : path);
      if (!actual.startsWith(root + sep)) throw error('skill_package_safe_open_failed');
    }
    const chunks = []; let length = 0;
    for (;;) { const buffer = Buffer.alloc(Math.min(65_536, maximum + 1 - length)); const { bytesRead } = await handle.read(buffer, 0, buffer.length, null); if (!bytesRead) break; chunks.push(buffer.subarray(0, bytesRead)); length += bytesRead; if (length > maximum) throw error('skill_package_file_too_large'); }
    if (identity(opened) !== identity(await handle.stat({ bigint: true }))) throw error('skill_package_source_changed');
    for (const [parent, info] of parents) if (identity(info) !== identity(await noLink(parent))) throw error('skill_package_source_changed');
    return Buffer.concat(chunks);
  } finally { await handle.close(); }
}
const exists = async path => { try { await lstat(path); return true; } catch (cause) { if (cause.code === 'ENOENT') return false; throw cause; } };
export async function localPackage(path) {
  if (typeof path !== 'string' || !path || path.includes('\0')) throw error('skill_package_path_unavailable');
  const absolute = resolve(path), info = await noLink(absolute);
  if (!info.isDirectory()) throw error('skill_package_select_directory_zip_unsupported');
  const root = await realpath(absolute), paths = new Set(), seen = new Map();
  seen.set(root, identity(await noLink(root)));
  async function collect(relative, depth = 0) {
    if (depth > LIMITS.depth) throw error('skill_package_depth_limit');
    const full = relative ? join(root, relative) : root, stat = await noLink(full); seen.set(full, identity(stat));
    if (stat.isDirectory()) { for (const name of await readdir(full)) await collect(relative ? `${relative}/${name}` : name, depth + 1); }
    else if (stat.isFile()) {
      checkedPath(relative); if (stat.size > BigInt(LIMITS.fileBytes)) throw error('skill_package_file_too_large');
      paths.add(relative); if (paths.size > LIMITS.files) throw error('skill_package_file_limit');
    } else throw error('skill_package_special_file_forbidden');
  }
  if (await exists(join(root, 'SKILL.md'))) await collect('');
  else if (await exists(join(root, 'skills'))) await collect('skills');
  else for (const name of await readdir(root)) { const entry = join(root, name), stat = await noLink(entry); if (stat.isDirectory() && await exists(join(entry, 'SKILL.md'))) await collect(name); }
  for (const name of ['LICENSE', 'LICENSE.md', 'LICENSE.txt', 'NOTICE']) if (await exists(join(root, name))) await collect(name);
  const files = []; let total = 0;
  for (const path of paths) { const bytes = await readBounded(join(root, path), LIMITS.fileBytes, root); total += bytes.length; if (total > LIMITS.totalBytes) throw error('skill_package_byte_limit'); files.push({ path, bytes, sha256: hash(bytes) }); }
  for (const [path, info] of seen) if (identity(await noLink(path)) !== info) throw error('skill_package_source_changed');
  return validate({ packageKey: 'local-folder', name: basename(root), version: 'local-content', source: 'Local folder import', license: 'See package resources', builtin: false, files });
}
function exactKeys(value, expected) {
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).sort().join('\0') !== [...expected].sort().join('\0')) throw error('skill_package_invalid_manifest');
}
export async function bundledPackage(path) {
  if (typeof path !== 'string' || !path || path.includes('\0')) throw error('skill_package_path_unavailable');
  const raw = await readBounded(resolve(path), LIMITS.manifestBytes);
  let decoded = raw;
  if (raw[0] === 0x1f && raw[1] === 0x8b) { try { decoded = gunzipSync(raw, { maxOutputLength: LIMITS.manifestBytes + 1 }); } catch { throw error('skill_package_bundle_decode'); } }
  if (decoded.length > LIMITS.manifestBytes) throw error('skill_package_manifest_limit');
  let manifest; try { manifest = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(decoded)); } catch { throw error('skill_package_invalid_manifest'); }
  exactKeys(manifest, ['schemaVersion', 'packageKey', 'name', 'version', 'source', 'license', 'files']);
  if (manifest.schemaVersion !== 1 || manifest.packageKey !== 'every-compound-engineering' || !Array.isArray(manifest.files) || manifest.files.length > LIMITS.files) throw error('skill_package_invalid_manifest');
  const files = manifest.files.map(file => {
    exactKeys(file, ['path', 'sha256', 'contentBase64']);
    if (typeof file.contentBase64 !== 'string' || file.contentBase64.length > LIMITS.fileBytes * 4 / 3 + 8) throw error('skill_package_file_too_large');
    if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(file.contentBase64)) throw error('skill_package_invalid_base64');
    const bytes = Buffer.from(file.contentBase64, 'base64'); if (bytes.toString('base64') !== file.contentBase64) throw error('skill_package_invalid_base64');
    return { path: file.path, bytes, sha256: file.sha256 };
  });
  return validate({ ...manifest, builtin: true, files });
}
export const resourceKind = path => path === 'SKILL.md' || path.endsWith('/SKILL.md') ? 'instructions'
  : path.split('/').includes('scripts') || /\.(sh|py|rb|js|ts|ps1|bat|cmd)$/u.test(path) ? 'script' : path.split('/').includes('references') ? 'reference' : 'resource';
export function allowedResource(entry, path) { const prefix = entry.slice(0, -'SKILL.md'.length); return (path.startsWith(prefix) && (prefix !== '' || !path.startsWith('skills/'))) || ['LICENSE', 'LICENSE.md', 'LICENSE.txt', 'NOTICE'].includes(path); }
export function privateResource(path) {
  return path.toLowerCase().split('/').some(name => name === '.env' || name.startsWith('.env.') || ['.ssh', '.aws', '.npmrc', '.netrc', '.git-credentials', 'credentials', 'credentials.json', 'secrets.json', 'auth.json', 'id_rsa', 'id_ed25519', 'cookies.sqlite'].includes(name) || /\.(pem|p12|pfx|key|keychain)$/u.test(name));
}
export function sensitiveText(text) {
  return /-----begin (?:rsa |openssh |ec )?private key-----/iu.test(text)
    || /(?:\bAKIA[0-9A-Z]{16}\b|\b(?:sk-(?:proj-)?|ghp_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{12,}|(?:password|api[_-]?key|access[_-]?token|secret[_-]?key)\s*["']?\s*[:=]\s*["'][^"'\r\n]{6,}["'])/iu.test(text);
}
