import { spawn } from 'node:child_process';
import { constants, existsSync } from 'node:fs';
import { mkdtemp, mkdir, open, readdir, rm, writeFile } from 'node:fs/promises';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { join, isAbsolute } from 'node:path';
import { lookup } from 'node:dns/promises';
import { isIP } from 'node:net';
import { blockedAddress } from './cited-pages.mjs';

const OUTPUT_LIMIT = 64 * 1024;

export async function sourceList(dir) {
  if (typeof dir !== 'string' || !isAbsolute(dir)) throw shellError('source_invalid');
  const entries = await readdir(dir, { withFileTypes: true });
  return {
    path: dir,
    truncated: entries.length > 200,
    entries: entries.slice(0, 200).map(entry => ({ name: entry.name, kind: entry.isSymbolicLink() ? 'symlink' : entry.isDirectory() ? 'dir' : 'file' })),
  };
}

export async function sourceSearch(dir, query) {
  const needle = String(query ?? '');
  if (typeof dir !== 'string' || !isAbsolute(dir) || !needle || needle.length > 200) throw shellError('source_invalid');
  const matches = [];
  async function walk(current, depth) {
    if (depth > 4 || matches.length >= 40) return;
    let entries = [];
    try { entries = await readdir(current, { withFileTypes: true }); }
    catch { return; }
    for (const entry of entries) {
      if (matches.length >= 40) return;
      if (entry.isSymbolicLink()) continue;
      const path = join(current, entry.name);
      if (entry.isDirectory()) { await walk(path, depth + 1); continue; }
      if (!entry.isFile()) continue;
      const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW).catch(() => null);
      if (!handle) continue;
      try {
        const info = await handle.stat();
        if (!info.isFile() || info.size > 8 * 1024 * 1024 || info.size === 0) continue;
        const text = (await handle.readFile()).toString('utf8');
        const at = text.indexOf(needle);
        if (at >= 0) matches.push({ path, offset: at, text: text.slice(Math.max(0, at - 60), at + 140) });
      } finally { await handle.close(); }
    }
  }
  await walk(dir, 0);
  return { path: dir, matches };
}

export function browserExecutable() {
  const candidates = [process.env.OVIRAPTOR_BROWSER_EXECUTABLE].filter(Boolean);
  if (process.platform === 'darwin') {
    candidates.push(
      '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
      '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
      '/Applications/Chromium.app/Contents/MacOS/Chromium',
    );
  } else if (process.platform === 'linux') {
    candidates.push('/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser');
  }
  return candidates.find(candidate => candidate && existsSync(candidate)) || '';
}

function shellError(code) { return Object.assign(Error(code), { code }); }

export function runHostShell(command, { cwd, timeoutMs = 30000 } = {}) {
  const text = String(command ?? '');
  if (!text.trim() || text.length > 4000 || text.includes('\0')) return Promise.reject(shellError('shell_invalid'));
  const limit = Number.isSafeInteger(timeoutMs) && timeoutMs >= 100 && timeoutMs <= 60000 ? timeoutMs : 30000;
  return new Promise((resolve, reject) => {
    const child = spawn('/bin/sh', ['-c', text], {
      cwd,
      timeout: limit,
      env: { PATH: '/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin', HOME: cwd, LANG: 'en_US.UTF-8' },
    });
    const out = [];
    const err = [];
    let outBytes = 0;
    let errBytes = 0;
    const take = (bucket, setBytes, getBytes, chunk) => {
      const piece = Buffer.from(chunk);
      const room = OUTPUT_LIMIT - getBytes();
      if (room <= 0) return;
      bucket.push(piece.subarray(0, room));
      setBytes(getBytes() + Math.min(piece.length, room));
    };
    child.stdout.on('data', chunk => take(out, value => { outBytes = value; }, () => outBytes, chunk));
    child.stderr.on('data', chunk => take(err, value => { errBytes = value; }, () => errBytes, chunk));
    child.on('error', () => reject(shellError('shell_failed')));
    child.on('close', (exitCode, signal) => {
      resolve({
        exitCode,
        signal: signal || '',
        stdout: Buffer.concat(out).toString('utf8'),
        stderr: Buffer.concat(err).toString('utf8'),
        truncated: outBytes >= OUTPUT_LIMIT || errBytes >= OUTPUT_LIMIT,
      });
    });
  });
}

function getJson(url) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.on('end', () => {
        try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf8'))); }
        catch { reject(shellError('browser_unavailable')); }
      });
    });
    request.on('error', () => reject(shellError('browser_unavailable')));
  });
}

function sleep(ms) { return new Promise(resolve => setTimeout(resolve, ms)); }

async function waitForBrowser(port) {
  for (let attempt = 0; attempt < 40; attempt++) {
    try { return await getJson(`http://127.0.0.1:${port}/json/version`); }
    catch { await sleep(100); }
  }
  throw shellError('browser_unavailable');
}

export async function browsePage(rawUrl, { workspaceRoot, timeoutMs = 15000 } = {}) {
  let url;
  try { url = new URL(String(rawUrl)); }
  catch { throw shellError('browser_url_invalid'); }
  if (url.username || url.password || (url.protocol !== 'http:' && url.protocol !== 'https:')) throw shellError('browser_url_invalid');
  const addresses = isIP(url.hostname) ? [url.hostname] : (await lookup(url.hostname, { all: true })).map(record => record.address);
  if (!addresses.length || addresses.some(address => blockedAddress(address))) throw shellError('browser_blocked');
  const executable = browserExecutable();
  if (!executable) throw shellError('browser_missing');
  const port = 43000 + Math.floor(Math.random() * 8000);
  const profile = await mkdtemp(join(tmpdir(), 'ovi-chat-browser-'));
  const child = spawn(executable, [
    '--headless=new',
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    '--no-first-run',
    '--disable-extensions',
    'about:blank',
  ], { stdio: 'ignore' });
  try {
    const version = await waitForBrowser(port);
    const socket = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', () => reject(shellError('browser_unavailable')), { once: true });
    });
    let nextId = 0;
    const pending = new Map();
    const seen = [];
    const loaded = new Promise(resolve => {
      const timer = setTimeout(resolve, Math.min(Math.max(timeoutMs, 1000), 30000));
      socket.addEventListener('message', event => {
        const message = JSON.parse(event.data);
        if (message.id && pending.has(message.id)) pending.get(message.id)(message.result || {});
        if (message.method === 'Network.responseReceived') {
          const response = message.params?.response || {};
          seen.push({
            requestId: message.params.requestId,
            url: String(response.url || '').slice(0, 2000),
            status: response.status || 0,
            mime: String(response.mimeType || '').slice(0, 120),
            type: String(message.params.type || ''),
          });
        }
        if (message.method === 'Page.loadEventFired') {
          clearTimeout(timer);
          setTimeout(resolve, 500);
        }
      });
    });
    const call = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++nextId;
      const timer = setTimeout(() => { pending.delete(id); reject(shellError('browser_timeout')); }, 15000);
      pending.set(id, result => { clearTimeout(timer); pending.delete(id); resolve(result); });
      socket.send(JSON.stringify({ id, method, params }));
    });
    await call('Network.enable');
    await call('Page.enable');
    await call('Page.navigate', { url: url.href });
    await loaded;
    const host = url.hostname.toLowerCase().replace(/[^a-z0-9.-]+/gu, '_') || 'host';
    const dir = join(workspaceRoot, 'cited-web', host, 'browser');
    await mkdir(dir, { recursive: true, mode: 0o700 });
    const scripts = [];
    for (const item of seen.filter(entry => entry.type === 'Script' || /javascript/iu.test(entry.mime)).slice(0, 12)) {
      try {
        const body = await call('Network.getResponseBody', { requestId: item.requestId });
        const bytes = body.base64Encoded ? Buffer.from(body.body || '', 'base64') : Buffer.from(body.body || '');
        if (!bytes.length || bytes.length > 2 * 1024 * 1024) continue;
        const name = `${scripts.length + 1}.js`;
        await writeFile(join(dir, name), bytes, { mode: 0o600 });
        scripts.push({ path: `cited-web/${host}/browser/${name}`, url: item.url, bytes: bytes.length });
      } catch { /* a finished request can still refuse its body */ }
    }
    socket.close();
    return { url: url.href, browser: version.Browser || '', requests: seen.slice(0, 80), scripts };
  } finally {
    child.kill('SIGKILL');
    await rm(profile, { recursive: true, force: true });
  }
}
