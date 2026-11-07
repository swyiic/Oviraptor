// External MCP connectors. jev-studio (TypeSafe Jev's typed decision engine,
// `pip install jev-studio`) is the BUILT-IN first consumer; the operator can
// also ADD their own stdio MCP servers. Each connector spawns a configured
// command, speaks newline-delimited JSON-RPC (initialize → tools/list →
// tools/call), and surfaces its tools to the agent PREFIXED by the connector id
// (`<id>_<tool>`). Every MCP tool is advisory only — like memory / technique
// cards / the tool library it NEVER feeds any auth / scope / settlement /
// capability gate. Connectors are disabled by default; any failure (missing
// binary, bad handshake, crash) degrades gracefully to "no extra tools" so the
// agent keeps running.
import { spawn } from 'node:child_process';
import { homedir } from 'node:os';
import { readdirSync } from 'node:fs';

const REQUEST_TIMEOUT_MS = 30000;
const MAX_LINE = 1 << 20; // 1 MB per JSON-RPC line
// jev-studio / the TypeSafe Jev family authenticate with TYPESAFE_API_KEY passed
// through the child process env (not a config-file flag).
const DEFAULT_API_KEY_ENV = 'TYPESAFE_API_KEY';
// Tool-name prefixes already owned by host-native tools; a user connector id may
// not collide with them (its tools would shadow real ones).
const RESERVED_IDS = new Set(['jev', 'laya', 'workspace', 'memory', 'task', 'verifier', 'record', 'skill', 'agent', 'subagent', 'finding', 'inventory', 'scan', 'discovery', 'exposure', 'certificate', 'host', 'oauth', 'mcp', 'project', 'saved']);

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

/** Normalize a connector config object (command/args/apiKey/apiKeyEnv/env/enabled), merging over current. */
function sanitizeConnectorConfig(input = {}, current = {}) {
  return {
    enabled: typeof input.enabled === 'boolean' ? input.enabled : (current.enabled === true),
    command: typeof input.command === 'string' && input.command.trim() ? input.command.trim() : (current.command || ''),
    args: Array.isArray(input.args) ? input.args.filter(a => typeof a === 'string').slice(0, 16) : (current.args || []),
    apiKey: typeof input.apiKey === 'string' ? input.apiKey : (current.apiKey || ''),
    apiKeyEnv: typeof input.apiKeyEnv === 'string' && input.apiKeyEnv.trim() ? input.apiKeyEnv.trim() : (current.apiKeyEnv || DEFAULT_API_KEY_ENV),
    env: input.env === undefined ? sanitizeEnvMap(current.env) : sanitizeEnvMap(input.env),
  };
}

export function readJevConfig(store) {
  try {
    const row = store.db.prepare("SELECT value FROM host_settings WHERE key='jev_mcp'").get();
    const cfg = row ? JSON.parse(row.value) : {};
    const normalized = sanitizeConnectorConfig(cfg, {});
    if (!normalized.command) normalized.command = 'jev-studio';
    return normalized;
  } catch { return { enabled: false, command: 'jev-studio', args: [], apiKey: '', apiKeyEnv: DEFAULT_API_KEY_ENV, env: {} }; }
}

// laya-mlx ships its own stdio MCP server (`laya-mlx-mcp-server`, the laya[mcp]
// extra) exposing local typed-decision tools (laya_decide / laya_route /
// laya_shortlist / laya_tournament …) — a local Apple-silicon sibling of jev.
// Defaults: the bare command (PATH-resolved like jev's jev-studio) and
// LAYA_PRELOAD=0 so the handshake (initialize + tools/list) returns in ~1s
// without downloading or building an MLX checkpoint; models build lazily on the
// first actual laya_* call. An operator env override wins. Laya has no API key,
// so apiKeyEnv is parked on a laya-named var (unused unless a key is set).
const LAYA_DEFAULT_COMMAND = 'laya-mlx-mcp-server';
const LAYA_API_KEY_ENV = 'LAYA_API_KEY';
export function readLayaConfig(store) {
  const withDefaults = (normalized) => {
    if (!normalized.command) normalized.command = LAYA_DEFAULT_COMMAND;
    if (!normalized.apiKeyEnv || normalized.apiKeyEnv === DEFAULT_API_KEY_ENV) normalized.apiKeyEnv = LAYA_API_KEY_ENV;
    if (!normalized.env || Object.keys(normalized.env).length === 0) normalized.env = { LAYA_PRELOAD: '0' };
    return normalized;
  };
  try {
    const row = store.db.prepare("SELECT value FROM host_settings WHERE key='laya_mcp'").get();
    const cfg = row ? JSON.parse(row.value) : {};
    return withDefaults(sanitizeConnectorConfig(cfg, {}));
  } catch { return { enabled: false, command: LAYA_DEFAULT_COMMAND, args: [], apiKey: '', apiKeyEnv: LAYA_API_KEY_ENV, env: { LAYA_PRELOAD: '0' } }; }
}

// GUI-launched hosts get a narrow PATH (/usr/bin:/bin). pip installs land in
// ~/.local/bin, Homebrew, or ~/Library/Python/*/bin, so augment PATH to resolve
// a bare command. A full path in config bypasses this entirely.
function augmentedPath() {
  const home = homedir();
  const extra = [`${home}/.local/bin`, '/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/bin'];
  try { for (const v of readdirSync(`${home}/Library/Python`)) extra.push(`${home}/Library/Python/${v}/bin`); } catch { /* not macOS / none */ }
  const dirs = [...new Set([...(process.env.PATH || '').split(':').filter(Boolean), ...extra])];
  return dirs.join(':');
}

/**
 * Generic stdio MCP connector: the connection mechanism, parameterized by id
 * (which also forms the tool-name prefix `<id>_`). Config is supplied per call
 * (start(config) / statusView(config)) so a registry can own persistence.
 */
export class McpStdioConnector {
  constructor({ id, label } = {}) {
    this.id = id || 'mcp';
    this.label = label || this.id;
    this.prefix = `${this.id}_`;
    this.child = null;
    this.tools = [];
    this.pending = new Map();
    this.nextId = 1;
    this.buffer = '';
    this.state = 'disabled'; // disabled | starting | ready | error
    this.error = '';
  }

  /** Runtime + config view for the UI. Never returns the key value. */
  statusView(config = {}) {
    return {
      id: this.id, label: this.label,
      enabled: config.enabled === true, command: config.command || '', args: config.args || [],
      apiKeyEnv: config.apiKeyEnv || DEFAULT_API_KEY_ENV, hasApiKey: !!config.apiKey, envKeys: Object.keys(config.env || {}),
      state: this.state, error: this.error,
      tools: this.tools.map(t => ({ name: this.prefix + t.name, title: t.name, description: String(t.description ?? '').slice(0, 300) })),
    };
  }

  async start(config = {}) {
    if (!config.enabled || !config.command) { this.state = 'disabled'; return; }
    if (this.child) return;
    this.state = 'starting'; this.error = ''; this.tools = []; this.buffer = '';
    let child;
    try {
      // Base env + augmented PATH, then operator env, then the API key (last so it wins).
      const childEnv = { ...process.env, PATH: augmentedPath(), ...(config.env || {}) };
      if (config.apiKey) childEnv[config.apiKeyEnv || DEFAULT_API_KEY_ENV] = config.apiKey;
      child = spawn(config.command, config.args || [], { stdio: ['pipe', 'pipe', 'ignore'], env: childEnv });
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
      name: this.prefix + t.name,
      description: `[MCP ${this.label}] ${String(t.description ?? t.name).slice(0, 400)}（建议性决策助手：不授予任何权限、不改变 agent 能对谁做什么）`,
      inputSchema: t.inputSchema && typeof t.inputSchema === 'object' ? t.inputSchema : { type: 'object', additionalProperties: true },
    }));
  }

  owns(name) { return typeof name === 'string' && name.startsWith(this.prefix); }

  async callTool(prefixedName, args) {
    if (this.state !== 'ready') throw new Error(`${this.id} connector not ready`);
    const name = String(prefixedName).slice(this.prefix.length);
    if (!this.tools.some(t => t.name === name)) throw new Error(`unknown ${this.id} tool`);
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
        if (msg.error) reject(new Error(String(msg.error?.message ?? 'mcp error').slice(0, 200)));
        else resolve(msg.result);
      }
      // notifications (no id) are ignored
    }
  }

  #write(message) {
    if (!this.child || !this.child.stdin.writable) throw new Error(`${this.id} connector not writable`);
    this.child.stdin.write(JSON.stringify(message) + '\n');
  }

  #request(method, params) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`${this.id} ${method} timeout`)); }, REQUEST_TIMEOUT_MS);
      this.pending.set(id, { resolve, reject, timer });
      try { this.#write({ jsonrpc: '2.0', id, method, params }); }
      catch (e) { clearTimeout(timer); this.pending.delete(id); reject(e); }
    });
  }

  #notify(method, params) {
    try { this.#write({ jsonrpc: '2.0', method, params }); } catch { /* best effort */ }
  }
}

/**
 * The built-in jev connector. Thin subclass that sources its config from
 * host_settings 'jev_mcp' (back-compat), keeping the original no-arg API so its
 * existing tests and the legacy /api/mcp/jev route are unchanged.
 */
export class JevConnector extends McpStdioConnector {
  constructor(store) {
    super({ id: 'jev', label: 'jev-studio' });
    this.store = store;
  }
  config() { return readJevConfig(this.store); }
  status() { return this.statusView(this.config()); }
  async start() { return super.start(this.config()); }
  async configure(patch = {}) {
    const next = sanitizeConnectorConfig(patch, this.config());
    if (!next.command) next.command = 'jev-studio';
    this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('jev_mcp',?)").run(JSON.stringify(next));
    await this.stop();
    if (next.enabled) await this.start();
    return this.status();
  }
}

/**
 * The built-in laya connector. Thin subclass sourcing config from host_settings
 * 'laya_mcp', defaulting to the bare `laya-mlx-mcp-server` command and lazy model
 * preload. Same advisory-only wall as jev: laya_* decision tools never feed any
 * auth / scope / settlement / capability gate.
 */
export class LayaConnector extends McpStdioConnector {
  constructor(store) {
    super({ id: 'laya', label: 'laya-mlx' });
    this.store = store;
  }
  config() { return readLayaConfig(this.store); }
  status() { return this.statusView(this.config()); }
  async start() { return super.start(this.config()); }
  async configure(patch = {}) {
    const next = sanitizeConnectorConfig(patch, this.config());
    if (!next.command) next.command = LAYA_DEFAULT_COMMAND;
    this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('laya_mcp',?)").run(JSON.stringify(next));
    await this.stop();
    if (next.enabled) await this.start();
    return this.status();
  }
}

/**
 * Registry of all MCP connectors: the built-in jev (non-removable) plus any
 * user-added stdio servers (persisted in host_settings 'mcp_connectors'). Merges
 * every ready connector's tools and routes calls by id prefix. Advisory-only wall
 * is unchanged: no MCP tool ever feeds an auth/scope/settlement/capability gate.
 */
export class McpRegistry {
  constructor(store) {
    this.store = store;
    this.jev = new JevConnector(store);
    this.laya = new LayaConnector(store);
    this.user = new Map(); // id -> McpStdioConnector
    for (const d of this.#userDescriptors()) this.#ensure(d.id, d.label);
  }

  #userDescriptors() {
    try {
      const row = this.store.db.prepare("SELECT value FROM host_settings WHERE key='mcp_connectors'").get();
      const list = row ? JSON.parse(row.value) : [];
      if (!Array.isArray(list)) return [];
      return list
        .filter(d => d && typeof d.id === 'string' && /^[a-z][a-z0-9_]{0,24}$/.test(d.id) && !RESERVED_IDS.has(d.id))
        .map(d => ({ id: d.id, label: typeof d.label === 'string' && d.label.trim() ? d.label.trim().slice(0, 60) : d.id, ...sanitizeConnectorConfig(d, {}) }));
    } catch { return []; }
  }

  #writeUser(list) {
    this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('mcp_connectors',?)").run(JSON.stringify(list));
  }

  #ensure(id, label) {
    if (!this.user.has(id)) this.user.set(id, new McpStdioConnector({ id, label }));
    else this.user.get(id).label = label || id;
    return this.user.get(id);
  }

  async startAll() {
    await this.jev.start().catch(() => {});
    await this.laya.start().catch(() => {});
    for (const d of this.#userDescriptors()) {
      if (d.enabled) await this.#ensure(d.id, d.label).start(d).catch(() => {});
    }
  }

  async stopAll() {
    await this.jev.stop().catch(() => {});
    await this.laya.stop().catch(() => {});
    for (const connector of this.user.values()) await connector.stop().catch(() => {});
  }

  list() {
    const out = [
      { builtin: true, removable: false, ...this.jev.status() },
      { builtin: true, removable: false, ...this.laya.status() },
    ];
    for (const d of this.#userDescriptors()) out.push({ builtin: false, removable: true, ...this.#ensure(d.id, d.label).statusView(d) });
    return { connectors: out };
  }

  async configure(id, patch = {}) {
    if (id === 'jev') { await this.jev.configure(patch); return this.list(); }
    if (id === 'laya') { await this.laya.configure(patch); return this.list(); }
    const list = this.#userDescriptors();
    const index = list.findIndex(d => d.id === id);
    if (index < 0) throw new Error('mcp_connector_not_found');
    const label = typeof patch.label === 'string' && patch.label.trim() ? patch.label.trim().slice(0, 60) : list[index].label;
    const next = { id, label, ...sanitizeConnectorConfig(patch, list[index]) };
    list[index] = next;
    this.#writeUser(list);
    const connector = this.#ensure(id, label);
    await connector.stop();
    if (next.enabled) await connector.start(next);
    return this.list();
  }

  async add(input = {}) {
    const id = String(input.id ?? '').trim().toLowerCase();
    if (!/^[a-z][a-z0-9_]{0,24}$/.test(id)) throw new Error('mcp_connector_id_invalid');
    if (RESERVED_IDS.has(id)) throw new Error('mcp_connector_id_reserved');
    const list = this.#userDescriptors();
    if (list.some(d => d.id === id)) throw new Error('mcp_connector_id_exists');
    if (list.length >= 16) throw new Error('mcp_connector_limit');
    const label = typeof input.label === 'string' && input.label.trim() ? input.label.trim().slice(0, 60) : id;
    const config = sanitizeConnectorConfig(input, {});
    if (!config.command) throw new Error('mcp_connector_command_required');
    const descriptor = { id, label, ...config };
    list.push(descriptor);
    this.#writeUser(list);
    const connector = this.#ensure(id, label);
    if (descriptor.enabled) await connector.start(descriptor).catch(() => {});
    return this.list();
  }

  async remove(id) {
    if (id === 'jev' || id === 'laya') throw new Error('mcp_connector_builtin_not_removable');
    const list = this.#userDescriptors();
    if (!list.some(d => d.id === id)) throw new Error('mcp_connector_not_found');
    this.#writeUser(list.filter(d => d.id !== id));
    const connector = this.user.get(id);
    if (connector) { await connector.stop().catch(() => {}); this.user.delete(id); }
    return this.list();
  }

  toolSpecs() {
    return [...this.jev.toolSpecs(), ...this.laya.toolSpecs(), ...[...this.user.values()].flatMap(c => c.toolSpecs())];
  }

  owns(name) {
    return this.jev.owns(name) || this.laya.owns(name) || [...this.user.values()].some(c => c.owns(name));
  }

  async callTool(name, args) {
    if (this.jev.owns(name)) return this.jev.callTool(name, args);
    if (this.laya.owns(name)) return this.laya.callTool(name, args);
    for (const connector of this.user.values()) if (connector.owns(name)) return connector.callTool(name, args);
    throw new Error('unknown mcp tool');
  }
}
