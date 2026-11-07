// External MCP connector (first consumer: jev-studio — TypeSafe Jev's typed
// decision engine, which `pip install jev-studio` serves over stdio). Speaks
// newline-delimited JSON-RPC (same framing as mcp-bridge.mjs), spawns the
// configured stdio MCP server, lists its tools and forwards calls. The tools are
// surfaced to the agent as advisory DECISION helpers (`jev_<name>`): they help
// the agent make disciplined typed decisions at forks, but — like memory /
// technique cards / the tool library — they are advisory only and NEVER feed any
// auth / scope / settlement / capability gate. Disabled by default; any failure
// (missing binary, bad handshake, crash) degrades gracefully to "no extra tools"
// so the agent keeps running.
import { spawn } from 'node:child_process';
import { homedir } from 'node:os';
import { readdirSync } from 'node:fs';

const REQUEST_TIMEOUT_MS = 30000;
const MAX_LINE = 1 << 20; // 1 MB per JSON-RPC line
const TOOL_PREFIX = 'jev_';
// jev-studio / the TypeSafe Jev family authenticate with TYPESAFE_API_KEY passed
// through the child process env (not a config-file flag). This is the researched
// default so the common case is zero-friction: the operator pastes one key.
const DEFAULT_API_KEY_ENV = 'TYPESAFE_API_KEY';

/** Keep only VALID_NAME=string pairs, bounded, so arbitrary junk can't reach spawn env. */
function sanitizeEnvMap(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const out = {};
  for (const [k, v] of Object.entries(value)) {
    if (typeof k !== 'string' || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(k)) continue;
    if (typeof v !== 'string') continue;
    out[k] = v.slice(0, 4096);
    if (Object.keys(out).length >= 24) break;
  }
  return out;
}

export function readJevConfig(store) {
  try {
    const row = store.db.prepare("SELECT value FROM host_settings WHERE key='jev_mcp'").get();
    const cfg = row ? JSON.parse(row.value) : {};
    return {
      enabled: cfg.enabled === true,
      command: typeof cfg.command === 'string' && cfg.command.trim() ? cfg.command.trim() : 'jev-studio',
      args: Array.isArray(cfg.args) ? cfg.args.filter(a => typeof a === 'string').slice(0, 16) : [],
      apiKey: typeof cfg.apiKey === 'string' ? cfg.apiKey : '',
      apiKeyEnv: typeof cfg.apiKeyEnv === 'string' && cfg.apiKeyEnv.trim() ? cfg.apiKeyEnv.trim() : DEFAULT_API_KEY_ENV,
      env: sanitizeEnvMap(cfg.env),
    };
  } catch { return { enabled: false, command: 'jev-studio', args: [], apiKey: '', apiKeyEnv: DEFAULT_API_KEY_ENV, env: {} }; }
}

// GUI-launched hosts get a narrow PATH (/usr/bin:/bin). pip installs land in
// ~/.local/bin, Homebrew, or ~/Library/Python/*/bin, so augment PATH to resolve
// a bare `jev-studio` command. A full path in config bypasses this entirely.
function augmentedPath() {
  const home = homedir();
  const extra = [`${home}/.local/bin`, '/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/bin'];
  try { for (const v of readdirSync(`${home}/Library/Python`)) extra.push(`${home}/Library/Python/${v}/bin`); } catch { /* not macOS / none */ }
  const dirs = [...new Set([...(process.env.PATH || '').split(':').filter(Boolean), ...extra])];
  return dirs.join(':');
}

export class JevConnector {
  constructor(store) {
    this.store = store;
    this.child = null;
    this.tools = [];
    this.pending = new Map();
    this.nextId = 1;
    this.buffer = '';
    this.state = 'disabled'; // disabled | starting | ready | error
    this.error = '';
  }

  config() { return readJevConfig(this.store); }

  status() {
    const cfg = this.config();
    // Never return the key value — only whether one is set and under what env name.
    return {
      enabled: cfg.enabled, command: cfg.command, args: cfg.args,
      apiKeyEnv: cfg.apiKeyEnv, hasApiKey: !!cfg.apiKey, envKeys: Object.keys(cfg.env),
      state: this.state, error: this.error,
      tools: this.tools.map(t => ({ name: TOOL_PREFIX + t.name, title: t.name, description: String(t.description ?? '').slice(0, 300) })),
    };
  }

  /** Persist config, then (re)start or stop to match. Returns status(). */
  async configure({ enabled, command, args, apiKey, apiKeyEnv, env } = {}) {
    const current = this.config();
    // apiKey/env: undefined = keep; a provided string/object replaces ('' clears the key).
    const next = {
      enabled: typeof enabled === 'boolean' ? enabled : current.enabled,
      command: typeof command === 'string' && command.trim() ? command.trim() : current.command,
      args: Array.isArray(args) ? args.filter(a => typeof a === 'string').slice(0, 16) : current.args,
      apiKey: typeof apiKey === 'string' ? apiKey : current.apiKey,
      apiKeyEnv: typeof apiKeyEnv === 'string' && apiKeyEnv.trim() ? apiKeyEnv.trim() : current.apiKeyEnv,
      env: env === undefined ? current.env : sanitizeEnvMap(env),
    };
    this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('jev_mcp',?)").run(JSON.stringify(next));
    await this.stop();
    if (next.enabled) await this.start();
    return this.status();
  }

  async start() {
    const cfg = this.config();
    if (!cfg.enabled) { this.state = 'disabled'; return; }
    if (this.child) return;
    this.state = 'starting'; this.error = ''; this.tools = []; this.buffer = '';
    let child;
    try {
      // Base env + augmented PATH, then operator-configured env, then the API key
      // under its env name (last so it always wins).
      const childEnv = { ...process.env, PATH: augmentedPath(), ...cfg.env };
      if (cfg.apiKey) childEnv[cfg.apiKeyEnv] = cfg.apiKey;
      child = spawn(cfg.command, cfg.args, { stdio: ['pipe', 'pipe', 'ignore'], env: childEnv });
    } catch (e) { this.state = 'error'; this.error = `spawn failed: ${String(e?.message || e).slice(0, 160)}`; return; }
    this.child = child;
    child.on('error', e => this.#fail(`process error: ${String(e?.message || e).slice(0, 160)}`));
    child.on('exit', code => { if (this.child === child) this.#fail(`exited (${code})`); });
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => this.#onData(chunk));
    try {
      await this.#request('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'oviraptor', version: '1' } });
      this.#notify('notifications/initialized', {});
      const listed = await this.#request('tools/list', {});
      this.tools = Array.isArray(listed?.tools) ? listed.tools.filter(t => t && typeof t.name === 'string') : [];
      this.state = 'ready';
    } catch (e) { this.#fail(`handshake failed: ${String(e?.message || e).slice(0, 160)}`); }
  }

  async stop() {
    const child = this.child; this.child = null;
    for (const { reject, timer } of this.pending.values()) { clearTimeout(timer); try { reject(new Error('connector stopped')); } catch { /* ignore */ } }
    this.pending.clear(); this.tools = []; this.buffer = '';
    if (this.state !== 'error') this.state = 'disabled';
    if (child) { try { child.kill('SIGTERM'); } catch { /* ignore */ } }
  }

  /** Tool specs to merge into the agent's tools/list (prefixed). [] unless ready. */
  toolSpecs() {
    if (this.state !== 'ready') return [];
    return this.tools.map(t => ({
      name: TOOL_PREFIX + t.name,
      description: `[jev 决策] ${String(t.description ?? t.name).slice(0, 400)}(建议性:帮你做有纪律的类型化决策,不授予任何权限、不改变能对谁做什么)`,
      inputSchema: t.inputSchema && typeof t.inputSchema === 'object' ? t.inputSchema : { type: 'object', additionalProperties: true },
    }));
  }

  owns(name) { return typeof name === 'string' && name.startsWith(TOOL_PREFIX); }

  async callTool(prefixedName, args) {
    if (this.state !== 'ready') throw new Error('jev connector not ready');
    const name = String(prefixedName).slice(TOOL_PREFIX.length);
    if (!this.tools.some(t => t.name === name)) throw new Error('unknown jev tool');
    return this.#request('tools/call', { name, arguments: args ?? {} });
  }

  #fail(msg) {
    this.state = 'error'; this.error = msg;
    const child = this.child; this.child = null;
    for (const { reject, timer } of this.pending.values()) { clearTimeout(timer); try { reject(new Error(msg)); } catch { /* ignore */ } }
    this.pending.clear();
    if (child) { try { child.kill('SIGKILL'); } catch { /* ignore */ } }
  }

  #onData(chunk) {
    this.buffer += chunk;
    if (this.buffer.length > MAX_LINE * 4) { this.#fail('oversized output'); return; }
    let idx;
    while ((idx = this.buffer.indexOf('\n')) >= 0) {
      const line = this.buffer.slice(0, idx).trim(); this.buffer = this.buffer.slice(idx + 1);
      if (!line) continue;
      if (line.length > MAX_LINE) { this.#fail('oversized frame'); return; }
      let msg; try { msg = JSON.parse(line); } catch { continue; }
      if (msg && msg.id != null && this.pending.has(msg.id)) {
        const { resolve, reject, timer } = this.pending.get(msg.id); clearTimeout(timer); this.pending.delete(msg.id);
        if (msg.error) reject(new Error(String(msg.error?.message ?? 'jev error').slice(0, 200)));
        else resolve(msg.result);
      }
      // notifications (no id) are ignored
    }
  }

  #write(message) {
    if (!this.child || !this.child.stdin.writable) throw new Error('jev connector not writable');
    this.child.stdin.write(JSON.stringify(message) + '\n');
  }

  #request(method, params) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`jev ${method} timeout`)); }, REQUEST_TIMEOUT_MS);
      this.pending.set(id, { resolve, reject, timer });
      try { this.#write({ jsonrpc: '2.0', id, method, params }); }
      catch (e) { clearTimeout(timer); this.pending.delete(id); reject(e); }
    });
  }

  #notify(method, params) {
    try { this.#write({ jsonrpc: '2.0', method, params }); } catch { /* best effort */ }
  }
}
