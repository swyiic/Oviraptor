// Agent-host owns these settings. No system proxy, process.env, desktop profile,
// application worker, or ACP child environment is modified by this owner.
import { mkdirSync, readFileSync, writeFileSync, renameSync, rmSync, chmodSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { hostError } from './store.mjs';
import { fetchDirect, fetchViaProxy, isLoopbackHost, noProxyMatches, proxyRouteForUrl } from './net-proxy.mjs';

const MODES = new Set(['system', 'direct', 'manual']);
const ENV_KEYS = ['HTTPS_PROXY', 'https_proxy', 'HTTP_PROXY', 'http_proxy', 'ALL_PROXY', 'all_proxy', 'NO_PROXY', 'no_proxy'];
const defaults = () => ({ mode: 'system', noProxy: '', proxyUrl: '' });

function validateProxy(raw) {
  if (typeof raw !== 'string' || raw.length > 2048 || /[\r\n\0]/.test(raw)) throw hostError('host_network_proxy_invalid');
  let url;
  try { url = new URL(raw.trim()); if (/[\r\n\0]/.test(`${decodeURIComponent(url.username)}${decodeURIComponent(url.password)}`)) throw Error('invalid'); }
  catch { throw hostError('host_network_proxy_invalid'); }
  if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || !['', '/'].includes(url.pathname) || url.search || url.hash) throw hostError('host_network_proxy_invalid');
  return url.toString();
}

function validateBypass(raw) {
  if (typeof raw !== 'string' || raw.length > 4096) throw hostError('host_network_bypass_invalid');
  const entries = raw.split(/[\s,]+/).filter(Boolean);
  if (entries.length > 128 || entries.some(entry => !/^[A-Za-z0-9._*:\[\]-]+$/.test(entry))) throw hostError('host_network_bypass_invalid');
  return entries.join(',');
}

export function redactProxyEndpoint(raw) {
  if (!raw) return null;
  try { const url = new URL(raw); return `${url.protocol}//${url.host}`; } catch { return null; }
}

export class HostNetworkSettings {
  constructor(dataDir, { env = process.env } = {}) {
    this.directory = join(dataDir, 'secrets'); this.file = join(this.directory, 'network.json');
    this.environment = Object.fromEntries(ENV_KEYS.filter(key => typeof env[key] === 'string').map(key => [key, env[key]]));
    this.lastRequest = null;
  }
  read() {
    if (!existsSync(this.file)) return defaults();
    let raw;
    try { raw = JSON.parse(readFileSync(this.file, 'utf8')); } catch { throw hostError('host_network_config_invalid'); }
    if (!MODES.has(raw?.mode)) throw hostError('host_network_config_invalid');
    const proxyUrl = raw.proxyUrl ? validateProxy(raw.proxyUrl) : '';
    if (raw.mode === 'manual' && !proxyUrl) throw hostError('host_network_config_invalid');
    return { mode: raw.mode, proxyUrl, noProxy: validateBypass(raw.noProxy ?? '') };
  }
  save(input = {}) {
    if (!input || typeof input !== 'object' || Array.isArray(input) || Object.keys(input).some(key => !['mode', 'proxyUrl', 'noProxy', 'clearProxy'].includes(key))) throw hostError('host_network_config_invalid');
    if (!MODES.has(input.mode)) throw hostError('host_network_mode_invalid');
    if (input.clearProxy !== undefined && typeof input.clearProxy !== 'boolean') throw hostError('host_network_config_invalid');
    let current;
    try { current = this.read(); } catch { current = defaults(); }
    const proxyUrl = input.clearProxy === true ? '' : input.proxyUrl !== undefined ? validateProxy(input.proxyUrl) : current.proxyUrl;
    if (input.mode === 'manual' && !proxyUrl) throw hostError('host_network_proxy_required');
    const value = { mode: input.mode, proxyUrl, noProxy: input.noProxy !== undefined ? validateBypass(input.noProxy) : current.noProxy };
    mkdirSync(this.directory, { recursive: true, mode: 0o700 });
    const staging = join(this.directory, `.network-${randomUUID()}.tmp`);
    try { writeFileSync(staging, JSON.stringify(value), { mode: 0o600, flag: 'wx' }); renameSync(staging, this.file); chmodSync(this.file, 0o600); }
    finally { rmSync(staging, { force: true }); }
    return value;
  }
  route(url) {
    const config = this.read(), target = new URL(url);
    let selected;
    if (isLoopbackHost(target.hostname)) selected = { proxyUrl: null, source: 'loopback', reason: 'loopback' };
    else if (config.mode === 'direct') selected = { proxyUrl: null, source: 'host-direct', reason: 'direct-mode' };
    else if (config.mode === 'manual') {
      const bypass = noProxyMatches(url, { NO_PROXY: config.noProxy });
      selected = { proxyUrl: bypass ? null : config.proxyUrl, source: bypass ? 'manual-bypass' : 'manual-proxy', reason: bypass ? 'no-proxy' : 'manual-proxy' };
    } else {
      const inherited = proxyRouteForUrl(url, this.environment);
      selected = { ...inherited, source: inherited.proxyUrl ? 'inherited-environment' : 'inherited-direct' };
    }
    const proxy = selected.proxyUrl ? new URL(selected.proxyUrl) : null;
    const publicRoute = {
      mode: config.mode, target: target.origin, source: selected.source, reason: selected.reason,
      proxyEndpoint: redactProxyEndpoint(selected.proxyUrl), proxyCredentialsConfigured: Boolean(proxy?.username || proxy?.password),
      ...(selected.variable ? { environmentVariable: selected.variable } : {}),
    };
    return { proxyUrl: selected.proxyUrl, publicRoute };
  }
  publicView({ targets = {}, transportInjected = false } = {}) {
    try {
      const config = this.read(), proxy = config.proxyUrl ? new URL(config.proxyUrl) : null;
      return {
        valid: true, mode: config.mode, noProxy: config.noProxy,
        manualProxyConfigured: Boolean(config.proxyUrl), manualProxyEndpoint: redactProxyEndpoint(config.proxyUrl), manualProxyCredentialsConfigured: Boolean(proxy?.username || proxy?.password),
        inheritedEnvironmentConfigured: ENV_KEYS.some(key => !/no_proxy/i.test(key) && Boolean(this.environment[key])),
        systemSource: 'inherited-startup-environment', transport: transportInjected ? 'injected' : 'host-network-owner',
        routes: Object.fromEntries(Object.entries(targets).filter(([, url]) => url).map(([name, url]) => [name, this.route(url).publicRoute])),
        lastRequest: this.lastRequest ? { ...this.lastRequest } : null,
      };
    } catch { return { valid: false, mode: 'system', error: 'host_network_config_invalid', routes: {}, lastRequest: null }; }
  }
  createFetch({ baseFetch = fetchDirect, viaProxy = fetchViaProxy } = {}) {
    return async (input, init = {}) => {
      const url = typeof input === 'string' || input instanceof URL ? String(input) : input.url;
      const { proxyUrl, publicRoute } = this.route(url);
      const record = { ...publicRoute, startedAt: new Date().toISOString(), outcome: 'pending' };
      this.lastRequest = record;
      try {
        // A chosen proxy failure is surfaced. It never retries through inherited
        // settings or direct transport, which could disclose the request elsewhere.
        const response = await (proxyUrl ? viaProxy(input, init, proxyUrl) : baseFetch(input, init));
        record.outcome = 'response'; record.httpStatus = response.status; return response;
      } catch (error) { record.outcome = init.signal?.aborted ? 'cancelled' : 'error'; throw error; }
    };
  }
}
