// Outbound proxy support for agent-host fetches (xAI OAuth + provider).
// Node's built-in fetch ignores HTTPS_PROXY/HTTP_PROXY/ALL_PROXY, and the macOS system
// proxy is invisible to it, so we resolve env proxies here and tunnel via HTTP CONNECT.
// Loopback (127.0.0.1 / localhost / ::1) never goes through a proxy.
import http from 'node:http';
import https from 'node:https';
import tls from 'node:tls';
import { Readable } from 'node:stream';
import { hostError } from './store.mjs';

const LOOPBACK = new Set(['localhost', '127.0.0.1', '::1', '[::1]']);
const envGet = (env, name) => env[name] ?? env[name.toLowerCase()] ?? '';

export function isLoopbackHost(host) {
  const h = String(host || '').toLowerCase().replace(/^\[|\]$/g, '');
  return LOOPBACK.has(h) || h === '::1' || /^127\./.test(h) || h.endsWith('.localhost');
}

export function noProxyMatches(url, env = process.env) {
  const target = new URL(url);
  const host = target.hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (isLoopbackHost(host)) return true;
  const port = target.port || (target.protocol === 'https:' ? '443' : '80');
  const list = envGet(env, 'NO_PROXY').split(/[\s,]+/).map(s => s.trim().toLowerCase()).filter(Boolean);
  return list.some(entry => {
    if (entry === '*') return true;
    let [pattern, entryPort] = entry.split(/:(?=\d+$)/);
    if (entryPort && entryPort !== port) return false;
    pattern = pattern.replace(/^\*?\./, '.');
    if (pattern.startsWith('.')) return host.endsWith(pattern) || host === pattern.slice(1);
    return host === pattern;
  });
}

/** Returns the proxy URL string to use for `url`, or null for a direct connection. */
export function proxyForUrl(url, env = process.env) {
  const target = new URL(url);
  if (noProxyMatches(url, env)) return null;
  const candidates = target.protocol === 'https:'
    ? [envGet(env, 'HTTPS_PROXY'), envGet(env, 'ALL_PROXY')]
    : [envGet(env, 'HTTP_PROXY'), envGet(env, 'ALL_PROXY')];
  for (const raw of candidates) {
    if (!raw) continue;
    const value = /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `http://${raw}`;
    try {
      const proxy = new URL(value);
      if (proxy.protocol === 'http:' || proxy.protocol === 'https:') return proxy.toString();
    } catch { /* ignore malformed proxy value */ }
  }
  return null;
}

function proxyAuthHeader(proxy) {
  if (!proxy.username) return {};
  const creds = `${decodeURIComponent(proxy.username)}:${decodeURIComponent(proxy.password)}`;
  return { 'Proxy-Authorization': `Basic ${Buffer.from(creds).toString('base64')}` };
}

function connectTunnel(proxy, target, signal) {
  return new Promise((resolve, reject) => {
    const mod = proxy.protocol === 'https:' ? https : http;
    const authority = `${target.hostname}:${target.port || 443}`;
    const req = mod.request({
      host: proxy.hostname, port: proxy.port || (proxy.protocol === 'https:' ? 443 : 80),
      method: 'CONNECT', path: authority, headers: { Host: authority, ...proxyAuthHeader(proxy) }, signal,
    });
    req.once('connect', (res, socket) => {
      if (res.statusCode !== 200) { socket.destroy(); reject(Object.assign(new Error(`proxy CONNECT ${res.statusCode}`), { code: 'EPROXYCONNECT' })); return; }
      resolve(socket);
    });
    req.once('error', reject);
    req.end();
  });
}

function headersToObject(headers) {
  const out = {};
  new Headers(headers ?? {}).forEach((value, key) => { out[key] = value; });
  return out;
}

async function bodyToBuffer(body) {
  if (body == null) return null;
  if (typeof body === 'string' || Buffer.isBuffer(body)) return Buffer.from(body);
  if (body instanceof URLSearchParams) return Buffer.from(body.toString());
  if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) return Buffer.from(body.buffer ?? body);
  return Buffer.from(await new Response(body).arrayBuffer());
}

/** fetch() through an explicit HTTP(S) proxy. Returns a WHATWG Response (streamable body). */
export async function fetchViaProxy(input, init = {}, proxyUrl) {
  const target = new URL(typeof input === 'string' || input instanceof URL ? input : input.url);
  const proxy = new URL(proxyUrl);
  const signal = init.signal ?? undefined;
  const payload = await bodyToBuffer(init.body);
  const headers = headersToObject(init.headers);
  if (payload && !Object.keys(headers).some(k => k.toLowerCase() === 'content-length')) headers['content-length'] = String(payload.length);
  headers.host ??= target.host;
  let request;
  if (target.protocol === 'https:') {
    const socket = await connectTunnel(proxy, target, signal);
    request = https.request({
      host: target.hostname, port: target.port || 443, path: `${target.pathname}${target.search}`,
      method: init.method ?? 'GET', headers, signal, servername: target.hostname,
      createConnection: () => tls.connect({ socket, servername: target.hostname, ALPNProtocols: ['http/1.1'] }),
      agent: false,
    });
  } else {
    request = (proxy.protocol === 'https:' ? https : http).request({
      host: proxy.hostname, port: proxy.port || 80, path: target.toString(),
      method: init.method ?? 'GET', headers: { ...headers, ...proxyAuthHeader(proxy) }, signal,
    });
  }
  const res = await new Promise((resolve, reject) => {
    request.once('response', resolve);
    request.once('error', reject);
    request.end(payload ?? undefined);
  });
  const status = res.statusCode ?? 502;
  if (init.redirect === 'error' && status >= 300 && status < 400) { res.destroy(); throw new TypeError('fetch failed: redirect'); }
  const responseHeaders = new Headers();
  for (const [key, value] of Object.entries(res.headers)) {
    if (Array.isArray(value)) value.forEach(v => responseHeaders.append(key, v)); else if (value != null) responseHeaders.set(key, String(value));
  }
  const nullBody = status === 204 || status === 304 || (init.method ?? 'GET').toUpperCase() === 'HEAD';
  if (nullBody) res.resume();
  return new Response(nullBody ? null : Readable.toWeb(res), { status, statusText: res.statusMessage, headers: responseHeaders });
}

/** Env-aware fetch: uses HTTPS_PROXY/HTTP_PROXY/ALL_PROXY unless NO_PROXY/loopback match. */
export function createProxyFetch({ env = process.env, baseFetch = globalThis.fetch, viaProxy = fetchViaProxy } = {}) {
  return async function proxyFetch(input, init = {}) {
    const url = typeof input === 'string' || input instanceof URL ? String(input) : input.url;
    const proxyUrl = proxyForUrl(url, env);
    return proxyUrl ? viaProxy(input, init, proxyUrl) : baseFetch(input, init);
  };
}

export const proxyFetch = (input, init) => createProxyFetch()(input, init);

const NETWORK_CODES = new Set(['ECONNRESET', 'ECONNREFUSED', 'ENOTFOUND', 'EAI_AGAIN', 'ETIMEDOUT', 'EHOSTUNREACH', 'ENETUNREACH', 'EPIPE', 'EPROXYCONNECT', 'UND_ERR_CONNECT_TIMEOUT', 'UND_ERR_SOCKET', 'ERR_SSL_WRONG_VERSION_NUMBER']);

/** Map a low-level fetch failure to a readable host error code. Passes host errors through. */
export function toNetworkHostError(error, host) {
  if (typeof error?.code === 'string' && error.code.startsWith('host_')) return error;
  if (error?.name === 'AbortError') return error;
  const cause = error?.cause ?? error;
  const code = cause?.code ?? error?.code;
  const isNetwork = error instanceof TypeError || NETWORK_CODES.has(code) || /tls|ssl|socket|network|fetch failed/i.test(String(cause?.message ?? error?.message ?? ''));
  if (!isNetwork) return error;
  return hostError('host_provider_network_unreachable', 502, { host, cause: String(code ?? cause?.message ?? 'network').slice(0, 120), retryable: true, outcomeUnknown: false });
}

export async function networkFetch(fetchImpl, url, init) {
  try { return await fetchImpl(url, init); }
  catch (error) { throw toNetworkHostError(error, new URL(String(url)).host); }
}
