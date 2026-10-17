// xAI / OpenAI-compatible model provider for the agent host.
// Auth: (1) Grok OAuth device-code via public Grok-CLI client — access+refresh stored
// locally; Bearer access_token calls api.x.ai. (2) Manual API key as advanced fallback.
// Secrets file is 0600; settings/events/errors never return tokens or keys.
import { mkdirSync, readFileSync, writeFileSync, rmSync, existsSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { hostError } from './store.mjs';
import { proxyFetch, networkFetch } from './net-proxy.mjs';

export const PROVIDERS = Object.freeze({
  xai: { label: 'xAI Grok API', baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7' },
  'openai-compatible': { label: 'OpenAI-compatible endpoint', baseUrl: '', model: '' },
});
export const MAX_TOOL_RESULT_BYTES = 16_384;
export const MAX_AGENT_STEPS = 12;

const loopback = host => host === '127.0.0.1' || host === 'localhost' || host === '[::1]';
export function validateProviderConfig(input = {}) {
  const id = input.provider ?? 'xai';
  if (!Object.hasOwn(PROVIDERS, id)) throw hostError('host_provider_invalid');
  const baseUrl = String(input.baseUrl || PROVIDERS[id].baseUrl).replace(/\/+$/, '');
  let url;
  try { url = new URL(baseUrl); } catch { throw hostError('host_provider_base_url_invalid'); }
  if (url.username || url.password || url.search || url.hash) throw hostError('host_provider_base_url_invalid');
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback(url.hostname))) throw hostError('host_provider_base_url_invalid');
  const model = String(input.model || PROVIDERS[id].model).trim();
  if (!/^[A-Za-z0-9._:/-]{1,128}$/.test(model)) throw hostError('host_provider_model_invalid');
  let apiKey;
  if (input.apiKey !== undefined) {
    apiKey = String(input.apiKey).trim();
    if (apiKey.length < 8 || apiKey.length > 512 || /[\s\0]/.test(apiKey)) throw hostError('host_provider_api_key_invalid');
  }
  return { provider: id, baseUrl, model, ...(apiKey !== undefined ? { apiKey } : {}) };
}

/** Local secret file store; settings only ever see configured flags, never secrets. */
export class ProviderStore {
  constructor(dataDir) { this.dir = join(dataDir, 'secrets'); this.file = join(this.dir, 'model-provider.json'); }
  read() {
    if (!existsSync(this.file)) return null;
    try { return JSON.parse(readFileSync(this.file, 'utf8')); } catch { return null; }
  }
  #write(value) {
    mkdirSync(this.dir, { recursive: true, mode: 0o700 });
    writeFileSync(this.file, JSON.stringify(value), { mode: 0o600 });
    chmodSync(this.file, 0o600);
  }
  /** Resolve the Bearer credential: OAuth access token preferred, else API key. */
  bearer() {
    const value = this.read();
    if (!value) return null;
    if (value.authentication === 'oauth' && value.accessToken) return value.accessToken;
    if (value.apiKey) return value.apiKey;
    if (value.accessToken) return value.accessToken;
    return null;
  }
  save(input) {
    const next = validateProviderConfig(input);
    const previous = this.read();
    if (next.apiKey === undefined) {
      if (previous?.authentication === 'oauth' && previous.accessToken && previous.provider === next.provider) {
        // Model/baseUrl update while keeping OAuth session.
        const merged = { ...previous, provider: next.provider, baseUrl: next.baseUrl, model: next.model };
        this.#write(merged);
        return this.publicView();
      }
      if (!previous?.apiKey || previous.provider !== next.provider) throw hostError('host_provider_api_key_required');
      next.apiKey = previous.apiKey;
    }
    // Explicit API key save replaces OAuth credentials for this provider file.
    const stored = { provider: next.provider, baseUrl: next.baseUrl, model: next.model, apiKey: next.apiKey, authentication: 'api-key' };
    this.#write(stored);
    return this.publicView();
  }
  /** Persist OpenCode-compatible OAuth tokens (access + refresh). */
  saveOAuth(tokens, { model, baseUrl, provider = 'xai' } = {}) {
    if (!tokens?.accessToken) throw hostError('host_oauth_token_invalid');
    const defaults = PROVIDERS[provider] ?? PROVIDERS.xai;
    const previous = this.read();
    const stored = {
      provider,
      baseUrl: String(baseUrl || previous?.baseUrl || defaults.baseUrl).replace(/\/+$/, ''),
      model: String(model || previous?.model || defaults.model).trim(),
      authentication: 'oauth',
      accessToken: String(tokens.accessToken),
      ...(tokens.refreshToken ? { refreshToken: String(tokens.refreshToken) } : {}),
      ...(tokens.expiresAt ? { expiresAt: Number(tokens.expiresAt) } : {}),
      ...(tokens.tokenType ? { tokenType: String(tokens.tokenType) } : {}),
      ...(tokens.scope ? { scope: String(tokens.scope) } : {}),
    };
    // Drop any previous API key when switching to OAuth.
    this.#write(stored);
    return this.publicView();
  }
  clear() { rmSync(this.file, { force: true }); return this.publicView(); }
  publicView() {
    const value = this.read();
    if (!value) {
      return { configured: false, apiKeyConfigured: false, oauthConfigured: false, authentication: null, oauthAvailable: true };
    }
    const oauth = value.authentication === 'oauth' && Boolean(value.accessToken);
    const apiKey = Boolean(value.apiKey);
    return {
      configured: oauth || apiKey,
      provider: value.provider,
      label: PROVIDERS[value.provider]?.label,
      baseUrl: value.baseUrl,
      model: value.model,
      apiKeyConfigured: apiKey,
      oauthConfigured: oauth,
      authentication: oauth ? 'oauth' : apiKey ? 'api-key' : null,
      oauthAvailable: true,
      ...(oauth && value.expiresAt ? { oauthExpiresAt: value.expiresAt } : {}),
    };
  }
}

/** Truncate a tool result to a byte budget, noting it can be re-queried with narrower arguments. */
export function truncateToolResult(value, maxBytes = MAX_TOOL_RESULT_BYTES) {
  const text = typeof value === 'string' ? value : JSON.stringify(value ?? null);
  const bytes = Buffer.from(text);
  if (bytes.length <= maxBytes) return { text, truncated: false, totalBytes: bytes.length };
  let end = maxBytes; while (end > 0 && (bytes[end] & 0xc0) === 0x80) end--;
  return { text: `${bytes.subarray(0, end).toString('utf8')}\n[truncated: showed ${end} of ${bytes.length} bytes; call the tool again with narrower filters, a smaller limit or a cursor to read more]`,
    truncated: true, totalBytes: bytes.length };
}

/** Apply truncation to MCP `content` text blocks without altering other fields. */
export function truncateMcpResult(result, maxBytes = MAX_TOOL_RESULT_BYTES) {
  if (!result || typeof result !== 'object' || !Array.isArray(result.content)) return result;
  let budget = maxBytes, truncated = false;
  const content = result.content.map(item => {
    if (item?.type !== 'text' || typeof item.text !== 'string') return item;
    const cut = truncateToolResult(item.text, Math.max(256, budget));
    budget = Math.max(0, budget - Buffer.byteLength(cut.text)); truncated ||= cut.truncated;
    return { ...item, text: cut.text };
  });
  return truncated ? { ...result, content, _meta: { ...(result._meta ?? {}), truncated: true } } : result;
}

const STRATEGY_HINTS = {
  host_tool_permission_denied: 'Not authorized: do not retry this tool; tell the user which permission to grant or use another granted tool.',
  host_tool_authority_changed: 'Permissions changed: re-read tools/list before choosing the next tool.',
  host_mcp_scope_unavailable: 'Turn scope ended: stop calling tools and summarize what is known.',
  host_app_timeout: 'Tool timed out after retries: narrow the request (smaller limit/filters) or try a different read tool.',
  timeout: 'Tool timed out after retries: narrow the request or try a different read tool.',
  host_tool_input_invalid: 'Arguments were rejected: re-check the tool inputSchema required fields and retry once with corrected arguments.',
};
export function strategyHint(code) {
  const key = String(code ?? '').split(':')[0];
  return STRATEGY_HINTS[key] ?? 'Tool failed: do not repeat the identical call; change arguments, choose another tool, or explain the blocker.';
}

export const PLANNING_GUIDANCE = 'Agent loop policy: first state a short numbered plan (1-5 steps), then execute one step at a time, marking each step done. Call independent read-only tools in the same turn so the host can run them in parallel. If a tool fails, follow its strategy hint instead of repeating the identical call. Large tool results are truncated; re-query narrowly rather than guessing.';

/** Group tool calls into waves: a call waits only for calls it names in `dependsOn` (or whose ids appear in its arguments). */
export function planToolWaves(calls) {
  const ids = new Set(calls.map(call => call.id));
  const deps = new Map(calls.map(call => {
    const explicit = Array.isArray(call.dependsOn) ? call.dependsOn : [];
    const args = typeof call.arguments === 'string' ? call.arguments : JSON.stringify(call.arguments ?? {});
    const implicit = [...ids].filter(id => id !== call.id && id && args.includes(id));
    return [call.id, new Set([...explicit, ...implicit].filter(id => ids.has(id)))];
  }));
  const done = new Set(), waves = [];
  let remaining = [...calls];
  while (remaining.length) {
    const wave = remaining.filter(call => [...deps.get(call.id)].every(id => done.has(id)));
    if (!wave.length) { waves.push(remaining); break; } // cycle: run rest serially as one final wave group
    waves.push(wave); for (const call of wave) done.add(call.id);
    remaining = remaining.filter(call => !wave.includes(call));
  }
  return waves;
}

/** Run tool calls: independent calls concurrently, dependent waves in order. Never rejects; failures become results. */
export async function runToolCalls(calls, execute, { maxBytes = MAX_TOOL_RESULT_BYTES } = {}) {
  const results = new Map();
  for (const wave of planToolWaves(calls)) {
    const settled = await Promise.allSettled(wave.map(call => execute(call)));
    wave.forEach((call, index) => {
      const outcome = settled[index];
      if (outcome.status === 'fulfilled') {
        const cut = truncateToolResult(outcome.value, maxBytes);
        results.set(call.id, { id: call.id, name: call.name, ok: true, content: cut.text, truncated: cut.truncated });
      } else {
        const code = typeof outcome.reason?.code === 'string' ? outcome.reason.code : 'host_tool_failed';
        results.set(call.id, { id: call.id, name: call.name, ok: false, content: JSON.stringify({ error: code, hint: strategyHint(code) }), code });
      }
    });
  }
  return calls.map(call => results.get(call.id));
}

/** Parse an OpenAI-compatible chat.completions SSE stream into text deltas and assembled tool calls. */
export async function readChatStream(body, onText) {
  const decoder = new TextDecoder(); let buffer = '', text = '', finish = null;
  const tools = [];
  const handle = data => {
    if (data === '[DONE]') return;
    let chunk; try { chunk = JSON.parse(data); } catch { return; }
    const choice = chunk.choices?.[0]; if (!choice) return;
    const delta = choice.delta ?? {};
    if (typeof delta.content === 'string' && delta.content) { text += delta.content; onText(delta.content); }
    for (const part of delta.tool_calls ?? []) {
      const index = part.index ?? tools.length;
      tools[index] ??= { id: '', name: '', arguments: '' };
      if (part.id) tools[index].id = part.id;
      if (part.function?.name) tools[index].name += part.function.name;
      if (part.function?.arguments) tools[index].arguments += part.function.arguments;
    }
    if (choice.finish_reason) finish = choice.finish_reason;
  };
  for await (const chunk of body) {
    buffer += typeof chunk === 'string' ? chunk : decoder.decode(chunk, { stream: true });
    let index;
    while ((index = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, index).replace(/\r$/, ''); buffer = buffer.slice(index + 1);
      if (line.startsWith('data:')) handle(line.slice(5).trim());
    }
  }
  if (buffer.startsWith('data:')) handle(buffer.slice(5).trim());
  return { text, toolCalls: tools.filter(Boolean), finishReason: finish };
}

const mcpServerEnv = (servers, name) => servers?.[0]?.env?.find?.(item => item.name === name)?.value;

/**
 * Host-native agent loop against an OpenAI-compatible endpoint (xAI by default).
 * Streams tokens, runs independent tool calls in parallel, truncates results,
 * and injects strategy hints after failures. Implements the host runtime interface.
 */
export class OpenAiCompatibleRuntime {
  constructor({ config, fetch: fetchImpl = proxyFetch, onEvent = () => {}, maxSteps = MAX_AGENT_STEPS, timeoutMs = 120_000 }) {
    const bearer = config?.apiKey || config?.accessToken;
    if (!bearer) throw hostError('host_provider_api_key_required');
    this.config = { ...config, apiKey: bearer }; this.fetch = fetchImpl; this.onEvent = onEvent; this.maxSteps = maxSteps; this.timeoutMs = timeoutMs;
    this.evidence = 'provider-api'; this.command = 'openai-compatible';
    this.status = { pid: -Math.floor(Math.random() * 1e9) - 1, state: 'idle' };
    this.sessions = new Map(); this.host = null;
  }
  bindHost(host) { this.host = host; }
  async start() { this.status.state = 'ready'; return this.status; }
  async newSession({ mcpServers = [] } = {}) {
    const sessionId = `provider-${crypto.randomUUID()}`;
    this.sessions.set(sessionId, { messages: [], token: mcpServerEnv(mcpServers, 'OVIRAPTOR_MCP_TOKEN'), conversationId: mcpServerEnv(mcpServers, 'OVIRAPTOR_CONVERSATION'), abort: null });
    return { sessionId };
  }
  async loadSession({ sessionId, mcpServers = [] }) {
    const existing = this.sessions.get(sessionId) ?? { messages: [] };
    this.sessions.set(sessionId, { ...existing, token: mcpServerEnv(mcpServers, 'OVIRAPTOR_MCP_TOKEN'), conversationId: mcpServerEnv(mcpServers, 'OVIRAPTOR_CONVERSATION') });
  }
  async #tools(session) {
    if (!this.host || !session.token) return [];
    const listed = await this.host.mcp(session.token, session.conversationId, 'tools/list');
    return (listed.tools ?? []).map(tool => ({ type: 'function', function: { name: tool.name, description: tool.description ?? '', parameters: tool.inputSchema ?? { type: 'object' } } }));
  }
  async prompt({ sessionId, prompt }) {
    const session = this.sessions.get(sessionId); if (!session) throw hostError('host_runtime_session_invalid');
    const started = performance.now(); let firstTextMs = null, firstToolMs = null;
    session.abort = new AbortController();
    session.messages.push({ role: 'system', content: PLANNING_GUIDANCE }, { role: 'user', content: typeof prompt === 'string' ? prompt : JSON.stringify(prompt) });
    const emit = event => { try { this.onEvent({ sessionId, ...event }); } catch { /* observer isolated */ } };
    try {
      for (let step = 0; step < this.maxSteps; step++) {
        const tools = await this.#tools(session);
        const response = await networkFetch(this.fetch, `${this.config.baseUrl}/chat/completions`, {
          method: 'POST', redirect: 'error',
          headers: { Authorization: `Bearer ${this.config.apiKey}`, 'Content-Type': 'application/json', Accept: 'text/event-stream' },
          body: JSON.stringify({ model: this.config.model, stream: true, messages: session.messages, ...(tools.length ? { tools, parallel_tool_calls: true } : {}) }),
          signal: AbortSignal.any([session.abort.signal, AbortSignal.timeout(this.timeoutMs)]),
        });
        if (!response.ok || !response.body) throw hostError(response.status === 401 || response.status === 403 ? 'host_provider_unauthorized' : 'host_provider_request_failed', 502, { httpStatus: response.status, outcomeUnknown: false });
        const turn = await readChatStream(response.body, delta => { if (firstTextMs === null) firstTextMs = performance.now() - started; emit({ type: 'text_delta', text: delta }); });
        session.messages.push({ role: 'assistant', content: turn.text || null, ...(turn.toolCalls.length ? { tool_calls: turn.toolCalls.map(call => ({ id: call.id, type: 'function', function: { name: call.name, arguments: call.arguments } })) } : {}) });
        if (!turn.toolCalls.length) return { stopReason: turn.finishReason === 'length' ? 'max_tokens' : 'end_turn', metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started } };
        if (firstToolMs === null) firstToolMs = performance.now() - started;
        const calls = turn.toolCalls.map(call => { let args = {}; try { args = JSON.parse(call.arguments || '{}'); } catch { /* invalid args reported by tool */ } return { id: call.id, name: call.name, arguments: args }; });
        for (const call of calls) emit({ type: 'tool_call', toolCallId: call.id, title: call.name, status: 'in_progress' });
        const results = await runToolCalls(calls, call => this.host.mcp(session.token, session.conversationId, 'tools/call', { name: call.name, arguments: call.arguments }));
        for (const result of results) {
          emit({ type: 'tool_update', toolCallId: result.id, title: result.name, status: result.ok ? 'completed' : 'failed', summary: result.content.slice(0, 240) });
          session.messages.push({ role: 'tool', tool_call_id: result.id, content: result.content });
        }
      }
      return { stopReason: 'max_turn_requests', metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started } };
    } catch (error) {
      if (session.abort.signal.aborted) return { stopReason: 'cancelled', metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started } };
      throw error;
    } finally { session.abort = null; }
  }
  async cancel(sessionId) { const session = this.sessions.get(sessionId); if (!session?.abort) return false; session.abort.abort(); return true; }
  async resolvePermission() { return false; }
  async close() { for (const session of this.sessions.values()) session.abort?.abort(); this.sessions.clear(); this.status.state = 'closed'; }
}
