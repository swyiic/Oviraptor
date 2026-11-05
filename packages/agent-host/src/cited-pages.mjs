import { createHash } from 'node:crypto';
import { lookup } from 'node:dns/promises';
import { constants } from 'node:fs';
import { isIP, BlockList } from 'node:net';
import { mkdir, open, unlink, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

const MAX_FILE = 8 * 1024 * 1024;
const SPILL_MAX = 8 * 1024 * 1024 * 1024;
const MAX_SCRIPTS = 8;
const MAX_PAGES = 1;

const blocked = new BlockList();
blocked.addSubnet('169.254.0.0', 16, 'ipv4');
blocked.addAddress('0.0.0.0', 'ipv4');
blocked.addSubnet('fe80::', 10, 'ipv6');
blocked.addAddress('::', 'ipv6');

export function blockedAddress(address) {
  const family = isIP(address);
  if (family !== 4 && family !== 6) return false;
  return blocked.check(address, family === 6 ? 'ipv6' : 'ipv4');
}

export function citedHosts(text) {
  const hosts = new Set();
  const value = String(text ?? '');
  for (const match of value.matchAll(/https?:\/\/[^\s"'<>]{1,2000}/giu)) {
    try {
      const url = new URL(match[0].replace(/[。，、；;,.!?）)】\]]+$/u, ''));
      if (!url.username && !url.password && (url.protocol === 'http:' || url.protocol === 'https:')) hosts.add(url.hostname.toLowerCase());
    } catch { /* skip a fragment that is not a URL */ }
  }
  for (const match of value.matchAll(/(?:^|\n)[ \t]*host:[ \t]*([^\s/:]+)/giu)) hosts.add(match[1].toLowerCase());
  return [...hosts];
}

export function pastedRequest(text) {
  const value = String(text ?? '');
  const match = value.match(/(?:^|\n)([A-Z]{3,12})[ \t]+(\S+)[ \t]+HTTP\/\d(?:\.\d)?[^\n]*\n/);
  if (!match) return null;
  const start = match.index + (value[match.index] === '\n' ? 1 : 0);
  const raw = value.slice(start);
  const header = raw.split(/\n[ \t]*\n/)[0];
  const hostLine = header.match(/^[ \t]*host:[ \t]*([^\s/:]+)/im);
  let host = hostLine?.[1]?.toLowerCase();
  if (!host && /^https?:\/\//iu.test(match[2])) {
    try { host = new URL(match[2]).hostname.toLowerCase(); } catch { host = ''; }
  }
  if (!host) return null;
  return { host, raw: raw.trimEnd() };
}

export function citedPageUrls(text) {
  const value = String(text ?? '');
  const urls = [];
  for (const match of value.matchAll(/https?:\/\/[^\s"'<>]{1,2000}/giu)) {
    let raw = match[0].replace(/[。，、；;,.!?）)】\]]+$/u, '');
    try {
      const url = new URL(raw);
      if (url.username || url.password) continue;
      if (url.protocol !== 'http:' && url.protocol !== 'https:') continue;
      url.hash = '';
      if (!urls.some(item => item.href === url.href)) urls.push(url);
    } catch { /* not a page URL */ }
  }
  return urls.slice(0, MAX_PAGES);
}

async function assertReachable(url, lookupImpl) {
  if (isIP(url.hostname)) {
    if (blockedAddress(url.hostname)) throw Object.assign(Error('blocked'), { code: 'blocked_address' });
    return;
  }
  const records = await lookupImpl(url.hostname, { all: true, verbatim: true });
  if (!records.length) throw Object.assign(Error('dns'), { code: 'dns' });
  if (records.some(record => blockedAddress(record.address))) throw Object.assign(Error('blocked'), { code: 'blocked_address' });
}

async function readBody(response, max) {
  const reader = response.body?.getReader();
  if (!reader) return { bytes: Buffer.alloc(0), truncated: false };
  const chunks = [];
  let total = 0;
  let truncated = false;
  while (true) {
    const step = await reader.read();
    if (step.done) break;
    const piece = Buffer.from(step.value);
    if (total + piece.length > max) {
      chunks.push(piece.subarray(0, max - total));
      truncated = true;
      await reader.cancel().catch(() => {});
      break;
    }
    chunks.push(piece);
    total += piece.length;
  }
  return { bytes: Buffer.concat(chunks), truncated };
}

function scriptUrls(html, pageUrl) {
  const found = [];
  for (const tag of html.matchAll(/<(?:script|link)\b[^>]*>/giu)) {
    const text = tag[0];
    const source = /(?:\bsrc|\bhref)\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s>]+))/iu.exec(text);
    const value = source?.[1] || source?.[2] || source?.[3];
    if (!value || value.startsWith('data:') || value.startsWith('javascript:')) continue;
    const isScript = /^<script/iu.test(text);
    const isModule = /modulepreload/iu.test(text) || (/\bas\s*=\s*["']?script/iu.test(text) && /\.m?js(?:[?#]|$)/iu.test(value));
    if (!isScript && !isModule) continue;
    try {
      const absolute = new URL(value, pageUrl);
      absolute.hash = '';
      if (absolute.hostname !== pageUrl.hostname) continue;
      if (absolute.protocol !== 'http:' && absolute.protocol !== 'https:') continue;
      if (!found.some(item => item.href === absolute.href)) found.push(absolute);
    } catch { /* ignore a bad reference */ }
  }
  return found.slice(0, MAX_SCRIPTS);
}

function safeSegment(value, fallback) {
  const clean = String(value || '').toLowerCase().replace(/[^a-z0-9.-]+/gu, '_').replace(/^_+|_+$/gu, '').slice(0, 80);
  return clean || fallback;
}

function scriptName(url, index) {
  const tail = url.pathname.split('/').filter(Boolean).slice(-2).join('_') || `script-${index}`;
  const clean = tail.replace(/[^A-Za-z0-9._-]+/gu, '_').slice(-80);
  return clean.endsWith('.js') ? clean : `${clean}.js`;
}

async function streamBody(response, file, maxBytes, spillAt) {
  await mkdir(dirname(file), { recursive: true, mode: 0o700 });
  const handle = await open(file, constants.O_WRONLY | constants.O_CREAT | constants.O_TRUNC, 0o600);
  const reader = response.body?.getReader();
  const chunks = [];
  let total = 0;
  let truncated = false;
  let spilled = false;
  try {
    while (reader) {
      const step = await reader.read();
      if (step.done) break;
      let piece = Buffer.from(step.value);
      if (total + piece.length > maxBytes) {
        piece = piece.subarray(0, maxBytes - total);
        truncated = true;
      }
      if (!spilled && total + piece.length > spillAt) spilled = true;
      if (!spilled) chunks.push(piece);
      await handle.write(piece);
      total += piece.length;
      if (truncated) {
        await reader.cancel().catch(() => {});
        break;
      }
    }
  } finally { await handle.close(); }
  if (!spilled) {
    await unlink(file).catch(() => {});
    return { bytes: Buffer.concat(chunks), truncated, spilled: false };
  }
  return { bytes: Buffer.alloc(0), truncated, spilled: true, size: total, absolutePath: file };
}

async function fetchSameHost(fetchImpl, start, lookupImpl, options = {}) {
  let current = start;
  for (let hop = 0; hop < 3; hop++) {
    await assertReachable(current, lookupImpl);
    const response = await fetchImpl(current, {
      redirect: 'manual',
      method: options.method || 'GET',
      headers: {
        accept: 'text/html,application/javascript,text/javascript,*/*;q=0.8',
        'user-agent': 'Oviraptor/cited-page',
        ...(options.headers || {}),
      },
      body: options.body || undefined,
      signal: AbortSignal.timeout(20000),
    });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      const location = response.headers.get('location');
      await response.body?.cancel?.().catch(() => {});
      if (!location) return { error: 'redirect_missing', status: response.status };
      const next = new URL(location, current);
      if (next.hostname !== start.hostname || next.username || next.password) return { error: 'redirect_left_host', status: response.status };
      next.hash = '';
      current = next;
      continue;
    }
    const body = options.spillFile
      ? await streamBody(response, options.spillFile, options.maxBytes ?? SPILL_MAX, options.spillAt ?? MAX_FILE)
      : await readBody(response, options.maxBytes ?? MAX_FILE);
    return {
      status: response.status,
      finalUrl: current,
      contentType: response.headers.get('content-type') || '',
      ...body,
    };
  }
  return { error: 'redirect_limit' };
}

const HTTP_METHODS = new Set(['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS']);

function requestOptions(input = {}) {
  const method = String(input.method || 'GET').toUpperCase();
  if (!HTTP_METHODS.has(method)) throw Object.assign(Error('fetch_url_invalid'), { code: 'fetch_url_invalid' });
  const headers = {};
  const source = input.headers && typeof input.headers === 'object' && !Array.isArray(input.headers) ? input.headers : {};
  for (const [name, value] of Object.entries(source).slice(0, 40)) {
    if (typeof name !== 'string' || typeof value !== 'string') continue;
    if (/^(host|content-length|connection|transfer-encoding)$/iu.test(name)) continue;
    headers[name] = value.slice(0, 8192);
  }
  const body = typeof input.body === 'string' ? input.body : '';
  if (Buffer.byteLength(body) > 256 * 1024) throw Object.assign(Error('fetch_url_invalid'), { code: 'fetch_url_invalid' });
  return { method, headers, body: body && method !== 'GET' && method !== 'HEAD' ? body : undefined };
}

export async function saveFetchedUrl(workspaceRoot, rawUrl, { allowedHosts = [], openNetwork = false, materialRoot = '', fetchImpl = fetch, lookupImpl = lookup, method, headers, body } = {}) {
  let url;
  try { url = new URL(String(rawUrl)); }
  catch { throw Object.assign(Error('fetch_url_invalid'), { code: 'fetch_url_invalid' }); }
  url.hash = '';
  if (url.username || url.password || (url.protocol !== 'http:' && url.protocol !== 'https:')) {
    throw Object.assign(Error('fetch_url_invalid'), { code: 'fetch_url_invalid' });
  }
  const allowed = new Set(allowedHosts.map(host => String(host).toLowerCase()));
  if (!openNetwork && !allowed.has(url.hostname.toLowerCase())) throw Object.assign(Error('fetch_host_not_allowed'), { code: 'fetch_host_not_allowed' });
  const host = safeSegment(url.hostname, 'host');
  const name = `${createHash('sha256').update(url.href).digest('hex').slice(0, 10)}-${scriptName(url, 1)}`;
  const spillFile = openNetwork && materialRoot ? join(materialRoot, `${host}-${name}`) : null;
  const request = requestOptions({ method, headers, body });
  const fetched = await fetchSameHost(fetchImpl, url, lookupImpl, { ...(spillFile ? { spillFile, maxBytes: SPILL_MAX } : {}), ...request });
  if (fetched.spilled) {
    return { absolutePath: fetched.absolutePath, url: fetched.finalUrl.href, status: fetched.status, bytes: fetched.size, truncated: Boolean(fetched.truncated), contentType: fetched.contentType, spilled: true };
  }
  if (fetched.error || !fetched.bytes) throw Object.assign(Error(fetched.error || 'fetch_empty'), { code: fetched.error || 'fetch_empty' });
  const relative = `cited-web/${host}/fetched/${name}`;
  await mkdir(join(workspaceRoot, 'cited-web', host, 'fetched'), { recursive: true, mode: 0o700 });
  await writeFile(join(workspaceRoot, relative), fetched.bytes, { mode: 0o600 });
  return { path: relative, url: fetched.finalUrl.href, method: request.method, status: fetched.status, bytes: fetched.bytes.length, truncated: Boolean(fetched.truncated), contentType: fetched.contentType };
}

export async function savePastedRequest(text, workspaceRoot) {
  const request = pastedRequest(text);
  if (!request) return null;
  const host = safeSegment(request.host, 'host');
  const relative = `cited-web/${host}/user-request.txt`;
  await mkdir(join(workspaceRoot, 'cited-web', host), { recursive: true, mode: 0o700 });
  await writeFile(join(workspaceRoot, relative), request.raw, { mode: 0o600 });
  return { path: relative, host: request.host };
}

export async function importCitedPages(text, workspaceRoot, { fetchImpl = fetch, lookupImpl = lookup } = {}) {
  const pages = citedPageUrls(text);
  if (!pages.length) return { imported: [], skipped: [], note: '' };
  const imported = [];
  const skipped = [];
  for (const page of pages) {
    const host = safeSegment(page.hostname, 'host');
    const dir = join(workspaceRoot, 'cited-web', host);
    try {
      const document = await fetchSameHost(fetchImpl, page, lookupImpl);
      if (document.error || !document.bytes) {
        skipped.push({ url: page.href, reason: document.error || 'empty' });
        continue;
      }
      await mkdir(join(dir, 'scripts'), { recursive: true, mode: 0o700 });
      const pagePath = 'cited-web/' + host + '/page.html';
      await writeFile(join(workspaceRoot, pagePath), document.bytes, { mode: 0o600 });
      const saved = [{ path: pagePath, url: document.finalUrl.href, bytes: document.bytes.length, status: document.status, truncated: document.truncated }];
      const html = document.bytes.toString('utf8');
      const looksHtml = /html|javascript/iu.test(document.contentType) || /<script|<html/iu.test(html.slice(0, 4000));
      if (looksHtml) {
        let index = 0;
        for (const script of scriptUrls(html, document.finalUrl)) {
          index += 1;
          const fetched = await fetchSameHost(fetchImpl, script, lookupImpl);
          if (fetched.error || !fetched.bytes) {
            skipped.push({ url: script.href, reason: fetched.error || 'empty' });
            continue;
          }
          const name = `${index}-${scriptName(script, index)}`;
          const relative = `cited-web/${host}/scripts/${name}`;
          await writeFile(join(workspaceRoot, relative), fetched.bytes, { mode: 0o600 });
          saved.push({ path: relative, url: script.href, bytes: fetched.bytes.length, status: fetched.status, truncated: fetched.truncated });
        }
      }
      const manifest = { page: page.href, files: saved };
      await writeFile(join(dir, 'manifest.json'), JSON.stringify(manifest, null, 2), { mode: 0o600 });
      imported.push(...saved, { path: `cited-web/${host}/manifest.json`, url: page.href, bytes: Buffer.byteLength(JSON.stringify(manifest)), status: 0, truncated: false });
    } catch (error) {
      skipped.push({ url: page.href, reason: error.code === 'blocked_address' ? 'blocked_address' : 'fetch_failed' });
    }
  }
  return { imported, skipped, note: pageNote(imported, skipped) };
}

function pageNote(imported, skipped) {
  if (!imported.length && !skipped.length) return '';
  const lines = ['Host: 用户点名的页面已经抓取并落盘。还缺同站脚本或接口响应时，调用 workspace_fetch，文件会写进 cited-web。不要说不能联网，也不要让用户代你保存。Python 脚本读这些落盘文件做分析，不要在脚本里自己访问网络。'];
  for (const file of imported) {
    if (file.path.endsWith('manifest.json')) continue;
    lines.push(`- ${file.path}（${file.bytes} 字节，HTTP ${file.status}${file.truncated ? '，已按 8MB 截断' : ''}）`);
  }
  lines.push('用 workspace_run 跑 Python 搜索这些文件里的 encrypt、decrypt、AES、DES、SM2、SM4、CryptoJS、JSEncrypt、sign、iv、key。workspace_grep 只看一行的前 2000 字，打包后的单行脚本必须用 Python 搜全文。');
  lines.push('回答要写出完整过程：在哪个文件、哪一段发现算法；页面或请求怎么跳到这段；密钥、IV、编码从哪来；解密的具体步骤。能在隔离环境复现的，写成脚本跑出结果并保存到工作区。密钥只在服务端、文件被截断、或这段根本不是加密时，写明卡在哪一步，不要在抓取前拒绝。');
  for (const file of skipped) lines.push(`- 未保存 ${file.reason === 'blocked_address' ? '链路地址' : '一次抓取'}（${file.reason}）。不要把没抓到的内容编造成已分析。`);
  return lines.join('\n');
}
