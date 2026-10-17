// xAI / OpenAI-compatible model provider for the agent host.
// Auth: (1) Grok OAuth device-code via public Grok-CLI client — access+refresh stored
// locally; Bearer access_token calls api.x.ai. (2) Manual API key as advanced fallback.
// Secrets file is 0600; settings/events/errors never return tokens or keys.
import { mkdirSync, readFileSync, writeFileSync, rmSync, existsSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { hostError } from './store.mjs';
import { proxyFetch, networkFetch } from './net-proxy.mjs';
import {
  PLANNING_GUIDANCE,
  DATA_TOOL_HINT,
  answerModeOptions,
  buildQualitySystemPrompt,
  looksLikeFactualClaim,
  looksLikeDataQuestion,
  normalizeAnswerMode,
  prefersResponsesApi,
  normalizeReasoningEffort,
} from './quality-prompt.mjs';
import {
  emptyAgentState,
  setGoal,
  absorbToolResults,
  shouldBlockEmptyCompletion,
  shouldInjectReviewer,
  noteReviewDone,
  isDeadEnd,
  REVIEWER_HINT,
  DEEP_THINK_HINT,
  formatAgentStatePromptBlock,
  verifyBeforeEnd,
} from './agent-state.mjs';
import { compressSessionMessages } from './memory.mjs';

export { PLANNING_GUIDANCE, DATA_TOOL_HINT, buildQualitySystemPrompt } from './quality-prompt.mjs';
export { compressMemory, compressSessionMessages } from './memory.mjs';

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

/** Truncate a tool result to a byte budget (head + tail) so both start and end stay readable. */
export function truncateToolResult(value, maxBytes = MAX_TOOL_RESULT_BYTES) {
  const text = typeof value === 'string' ? value : JSON.stringify(value ?? null);
  const bytes = Buffer.from(text);
  if (bytes.length <= maxBytes) return { text, truncated: false, totalBytes: bytes.length };
  const marker = `\n[truncated: showed head+tail of ${bytes.length} bytes; call the tool again with narrower filters, a smaller limit or a cursor to read more]\n`;
  const markerBytes = Buffer.byteLength(marker);
  const budget = Math.max(256, maxBytes - markerBytes);
  const headBudget = Math.floor(budget * 0.7);
  const tailBudget = budget - headBudget;
  let headEnd = headBudget;
  while (headEnd > 0 && (bytes[headEnd] & 0xc0) === 0x80) headEnd--;
  let tailStart = bytes.length - tailBudget;
  while (tailStart < bytes.length && (bytes[tailStart] & 0xc0) === 0x80) tailStart++;
  const head = bytes.subarray(0, headEnd).toString('utf8');
  const tail = bytes.subarray(Math.max(headEnd, tailStart)).toString('utf8');
  return { text: `${head}${marker}${tail}`, truncated: true, totalBytes: bytes.length };
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
  host_tool_permission_denied: 'Not authorized: do not retry this tool; tell the user which permission to grant or use another granted tool. Scope: permission missing ≠ data absent.',
  host_tool_authority_changed: 'Permissions changed: re-read tools/list before choosing the next tool. Do not reuse old grantRevision.',
  host_mcp_scope_unavailable: 'Turn scope ended: stop calling tools and summarize what is known within this turn only.',
  host_app_timeout: 'Tool timed out after retries: narrow the request (smaller limit/filters) or try a different read tool. Timeout ≠ empty inventory.',
  timeout: 'Tool timed out after retries: narrow the request or try a different read tool. Timeout ≠ data does not exist.',
  host_tool_input_invalid: 'Arguments were rejected: re-check the tool inputSchema required fields and retry once with corrected arguments (max one retry).',
  host_app_process_exited: 'App worker exited: do not hammer the same call; report scoped failure and try another granted read tool if any.',
};
export function strategyHint(code) {
  const key = String(code ?? '').split(':')[0];
  return STRATEGY_HINTS[key] ?? 'Tool failed: do not repeat the identical call; change arguments, choose another tool, or explain the blocker with scope (this attempt only).';
}

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

/**
 * Parse xAI / OpenAI Responses API SSE stream.
 * Emits user-visible text via onText; keeps reasoning encrypted items for next-turn input.
 */
export async function readResponsesStream(body, onText) {
  const decoder = new TextDecoder(); let buffer = '';
  let text = '';
  const tools = [];
  const toolByIndex = new Map();
  const outputItems = [];
  let responseId = null;
  let finish = null;
  const handleEvent = (eventType, data) => {
    if (data === '[DONE]') return;
    let chunk; try { chunk = JSON.parse(data); } catch { return; }
    const type = eventType || chunk.type || '';
    if (chunk.id && (type.startsWith('response.') || type === 'response')) responseId = chunk.id ?? responseId;
    if (chunk.response?.id) responseId = chunk.response.id;
    if (type === 'response.output_text.delta' || type === 'response.text.delta') {
      const delta = chunk.delta ?? chunk.text ?? '';
      if (typeof delta === 'string' && delta) { text += delta; onText(delta); }
      return;
    }
    if (type === 'response.content_part.delta' && typeof chunk.delta?.text === 'string') {
      text += chunk.delta.text; onText(chunk.delta.text); return;
    }
    // Ignore reasoning deltas for UI answer stream (still billed; encrypted items captured on completed).
    if (type === 'response.output_item.added' || type === 'response.output_item.done') {
      const item = chunk.item ?? chunk.output_item;
      if (!item) return;
      if (item.type === 'function_call') {
        const index = chunk.output_index ?? tools.length;
        toolByIndex.set(index, {
          id: item.call_id || item.id || `call_${index}`,
          name: item.name || '',
          arguments: typeof item.arguments === 'string' ? item.arguments : JSON.stringify(item.arguments ?? {}),
        });
      } else if (item.type === 'reasoning' || item.encrypted_content || item.type === 'message') {
        outputItems.push(item);
      }
      return;
    }
    if (type === 'response.function_call_arguments.delta') {
      const index = chunk.output_index ?? 0;
      const row = toolByIndex.get(index) ?? { id: chunk.call_id || `call_${index}`, name: chunk.name || '', arguments: '' };
      row.arguments += chunk.delta ?? '';
      if (chunk.name) row.name = chunk.name;
      if (chunk.call_id) row.id = chunk.call_id;
      toolByIndex.set(index, row);
      return;
    }
    if (type === 'response.function_call_arguments.done') {
      const index = chunk.output_index ?? 0;
      const row = toolByIndex.get(index) ?? { id: chunk.call_id || `call_${index}`, name: chunk.name || '', arguments: '' };
      if (chunk.arguments) row.arguments = chunk.arguments;
      if (chunk.name) row.name = chunk.name;
      if (chunk.call_id) row.id = chunk.call_id;
      toolByIndex.set(index, row);
      return;
    }
    if (type === 'response.completed') {
      finish = 'stop';
      const response = chunk.response ?? chunk;
      responseId = response.id ?? responseId;
      for (const item of response.output ?? []) {
        if (item.type === 'function_call') {
          tools.push({
            id: item.call_id || item.id,
            name: item.name,
            arguments: typeof item.arguments === 'string' ? item.arguments : JSON.stringify(item.arguments ?? {}),
          });
        } else if (item.type === 'message') {
          for (const part of item.content ?? []) {
            if (part.type === 'output_text' && typeof part.text === 'string' && !text.includes(part.text)) {
              // Already streamed usually; only fill if empty
              if (!text) { text = part.text; onText(part.text); }
            }
          }
          outputItems.push(item);
        } else if (item.type === 'reasoning') {
          outputItems.push(item);
        }
      }
      if (!tools.length && toolByIndex.size) tools.push(...[...toolByIndex.entries()].sort((a, b) => a[0] - b[0]).map(([, v]) => v));
      return;
    }
    if (type === 'response.incomplete') finish = 'length';
  };

  let currentEvent = '';
  for await (const chunk of body) {
    buffer += typeof chunk === 'string' ? chunk : decoder.decode(chunk, { stream: true });
    let index;
    while ((index = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, index).replace(/\r$/, ''); buffer = buffer.slice(index + 1);
      if (line.startsWith('event:')) { currentEvent = line.slice(6).trim(); continue; }
      if (line.startsWith('data:')) {
        handleEvent(currentEvent, line.slice(5).trim());
        currentEvent = '';
      }
      if (line === '') currentEvent = '';
    }
  }
  if (buffer.startsWith('data:')) handleEvent(currentEvent, buffer.slice(5).trim());
  if (!tools.length && toolByIndex.size) {
    for (const [, value] of [...toolByIndex.entries()].sort((a, b) => a[0] - b[0])) tools.push(value);
  }
  return { text, toolCalls: tools.filter(t => t.name), finishReason: finish, responseId, outputItems };
}

const mcpServerEnv = (servers, name) => servers?.[0]?.env?.find?.(item => item.name === name)?.value;

function toResponsesTools(chatTools) {
  return (chatTools ?? []).map(tool => ({
    type: 'function',
    name: tool.function?.name ?? tool.name,
    description: tool.function?.description ?? tool.description ?? '',
    parameters: tool.function?.parameters ?? tool.parameters ?? { type: 'object' },
  }));
}

function chatMessagesToResponsesInput(messages) {
  const input = [];
  for (const message of messages) {
    if (message.role === 'system') {
      input.push({ role: 'system', content: message.content ?? '' });
    } else if (message.role === 'user') {
      input.push({ role: 'user', content: typeof message.content === 'string' ? message.content : JSON.stringify(message.content ?? '') });
    } else if (message.role === 'assistant') {
      if (message.tool_calls?.length) {
        for (const call of message.tool_calls) {
          input.push({
            type: 'function_call',
            call_id: call.id,
            name: call.function?.name ?? call.name,
            arguments: call.function?.arguments ?? call.arguments ?? '{}',
          });
        }
      }
      if (message.content) input.push({ role: 'assistant', content: message.content });
    } else if (message.role === 'tool') {
      input.push({ type: 'function_call_output', call_id: message.tool_call_id, output: message.content ?? '' });
    }
  }
  return input;
}

/**
 * Host-native agent loop against xAI Responses API (preferred) or Chat Completions.
 * Streams tokens, parallel tools, truncation, quality harness (fact/goal/reviewer), reasoning.effort.
 */
export class OpenAiCompatibleRuntime {
  constructor({ config, fetch: fetchImpl = proxyFetch, onEvent = () => {}, maxSteps, timeoutMs, answerMode = 'quality' } = {}) {
    const bearer = config?.apiKey || config?.accessToken;
    if (!bearer) throw hostError('host_provider_api_key_required');
    this.answerMode = normalizeAnswerMode(answerMode);
    const mode = answerModeOptions(this.answerMode);
    this.config = { ...config, apiKey: bearer }; this.fetch = fetchImpl; this.onEvent = onEvent;
    this.maxSteps = maxSteps ?? mode.maxSteps;
    this.timeoutMs = timeoutMs ?? mode.timeoutMs;
    this.temperature = mode.temperature; this.maxTokens = mode.maxTokens; this.toolHintGuard = mode.toolHintGuard;
    this.forceToolOnDataQuestion = mode.forceToolOnDataQuestion;
    this.reasoningEffort = mode.reasoningEffort;
    this.useResponsesApi = mode.useResponsesApi && prefersResponsesApi(this.config);
    this.evidence = 'provider-api'; this.command = 'openai-compatible';
    this.status = { pid: -Math.floor(Math.random() * 1e9) - 1, state: 'idle' };
    this.sessions = new Map(); this.host = null;
  }
  bindHost(host) { this.host = host; }
  async start() { this.status.state = 'ready'; return this.status; }
  async newSession({ mcpServers = [] } = {}) {
    const sessionId = `provider-${crypto.randomUUID()}`;
    this.sessions.set(sessionId, {
      messages: [],
      token: mcpServerEnv(mcpServers, 'OVIRAPTOR_MCP_TOKEN'),
      conversationId: mcpServerEnv(mcpServers, 'OVIRAPTOR_CONVERSATION'),
      abort: null,
      harness: emptyAgentState(),
      previousResponseId: null,
      reasoningItems: [],
    });
    return { sessionId };
  }
  async loadSession({ sessionId, mcpServers = [] }) {
    const existing = this.sessions.get(sessionId) ?? { messages: [], harness: emptyAgentState(), reasoningItems: [] };
    this.sessions.set(sessionId, {
      ...existing,
      harness: existing.harness ?? emptyAgentState(),
      reasoningItems: existing.reasoningItems ?? [],
      token: mcpServerEnv(mcpServers, 'OVIRAPTOR_MCP_TOKEN'),
      conversationId: mcpServerEnv(mcpServers, 'OVIRAPTOR_CONVERSATION'),
    });
  }
  async #tools(session) {
    if (!this.host || !session.token) return [];
    const listed = await this.host.mcp(session.token, session.conversationId, 'tools/list');
    return (listed.tools ?? []).map(tool => ({ type: 'function', function: { name: tool.name, description: tool.description ?? '', parameters: tool.inputSchema ?? { type: 'object' } } }));
  }
  #applyMode(answerMode) {
    if (!answerMode) return;
    this.answerMode = normalizeAnswerMode(answerMode);
    const mode = answerModeOptions(this.answerMode);
    this.temperature = mode.temperature; this.maxTokens = mode.maxTokens; this.toolHintGuard = mode.toolHintGuard;
    this.forceToolOnDataQuestion = mode.forceToolOnDataQuestion;
    this.reasoningEffort = mode.reasoningEffort;
    this.useResponsesApi = mode.useResponsesApi && prefersResponsesApi(this.config);
    this.timeoutMs = mode.timeoutMs;
    if (mode.maxSteps < this.maxSteps) this.maxSteps = mode.maxSteps;
  }
  async #chatTurn(session, tools, { toolChoice } = {}) {
    const body = {
      model: this.config.model,
      stream: true,
      temperature: this.temperature,
      max_tokens: this.maxTokens,
      messages: session.messages,
      ...(tools.length ? { tools, parallel_tool_calls: true } : {}),
      ...(toolChoice ? { tool_choice: toolChoice } : {}),
    };
    const response = await networkFetch(this.fetch, `${this.config.baseUrl}/chat/completions`, {
      method: 'POST', redirect: 'error',
      headers: { Authorization: `Bearer ${this.config.apiKey}`, 'Content-Type': 'application/json', Accept: 'text/event-stream' },
      body: JSON.stringify(body),
      signal: AbortSignal.any([session.abort.signal, AbortSignal.timeout(this.timeoutMs)]),
    });
    if (!response.ok || !response.body) throw hostError(response.status === 401 || response.status === 403 ? 'host_provider_unauthorized' : 'host_provider_request_failed', 502, { httpStatus: response.status, outcomeUnknown: false });
    return { api: 'chat', turn: await readChatStream(response.body, this._onText) };
  }
  async #responsesTurn(session, tools, { toolChoice } = {}) {
    const effort = normalizeReasoningEffort(this.reasoningEffort, this.answerMode);
    const input = [
      ...chatMessagesToResponsesInput(session.messages),
      ...((session.reasoningItems ?? []).filter(item => item?.type === 'reasoning')),
    ];
    const body = {
      model: this.config.model,
      stream: true,
      store: false,
      max_output_tokens: this.maxTokens,
      reasoning: { effort },
      input,
      ...(tools.length ? { tools: toResponsesTools(tools), parallel_tool_calls: true } : {}),
      ...(toolChoice ? { tool_choice: toolChoice } : {}),
      prompt_cache_key: session.conversationId || session.sessionId || 'oviraptor-agent',
    };
    const response = await networkFetch(this.fetch, `${this.config.baseUrl}/responses`, {
      method: 'POST', redirect: 'error',
      headers: {
        Authorization: `Bearer ${this.config.apiKey}`,
        'Content-Type': 'application/json',
        Accept: 'text/event-stream',
      },
      body: JSON.stringify(body),
      signal: AbortSignal.any([session.abort.signal, AbortSignal.timeout(this.timeoutMs)]),
    });
    if (!response.ok || !response.body) {
      // Fall back to chat completions once if Responses unavailable on this endpoint.
      if (response.status === 404 || response.status === 405 || response.status === 501) {
        return this.#chatTurn(session, tools, { toolChoice });
      }
      throw hostError(response.status === 401 || response.status === 403 ? 'host_provider_unauthorized' : 'host_provider_request_failed', 502, { httpStatus: response.status, outcomeUnknown: false });
    }
    const turn = await readResponsesStream(response.body, this._onText);
    session.previousResponseId = turn.responseId ?? session.previousResponseId;
    session.reasoningItems = (turn.outputItems ?? []).filter(item => item?.type === 'reasoning');
    return { api: 'responses', turn };
  }
  /** One model HTTP turn; on 401, ask host to refresh OAuth once and retry if no text yet. */
  async #modelTurn(session, tools, { toolChoice } = {}) {
    const attempt = async () => (this.useResponsesApi
      ? await this.#responsesTurn(session, tools, { toolChoice })
      : await this.#chatTurn(session, tools, { toolChoice }));
    try {
      return await attempt();
    } catch (error) {
      const unauthorized = error?.code === 'host_provider_unauthorized';
      if (!unauthorized || session._authRetried || !this.host?.ensureOAuthAccess) throw error;
      // Only retry when nothing streamed yet (avoid double-billing partial answers).
      if (session._producedText) throw error;
      session._authRetried = true;
      try {
        await this.host.ensureOAuthAccess({ force: true });
        const bearer = this.host.providerStore?.bearer?.();
        if (bearer) this.config.apiKey = bearer;
      } catch {
        throw error;
      }
      return await attempt();
    }
  }

  async prompt({ sessionId, prompt, userText: rawUserText, answerMode } = {}) {
    const session = this.sessions.get(sessionId); if (!session) throw hostError('host_runtime_session_invalid');
    this.#applyMode(answerMode);
    const started = performance.now(); let firstTextMs = null, firstToolMs = null;
    session.abort = new AbortController();
    session._authRetried = false;
    const fullPrompt = typeof prompt === 'string' ? prompt : JSON.stringify(prompt);
    // Goal must be the user's raw question — never the whole turn prompt blob.
    const goalText = (typeof rawUserText === 'string' && rawUserText.trim())
      ? rawUserText.trim()
      : fullPrompt;
    session.harness = setGoal(session.harness ?? emptyAgentState(), goalText);
    const system = buildQualitySystemPrompt({ answerMode: this.answerMode, reasoningEffort: this.reasoningEffort });
    const harnessBlock = formatAgentStatePromptBlock(session.harness);
    if (!session.messages.some(message => message.role === 'system')) {
      session.messages.push({ role: 'system', content: system });
    } else {
      session.messages[0] = { role: 'system', content: system };
    }
    const userContent = harnessBlock ? `${harnessBlock}\n\n${fullPrompt}` : fullPrompt;
    session.messages.push({ role: 'user', content: userContent });
    // Compress API messages (same shape as SQLite memory): recent window + fold old tools into harness.
    {
      const packed = compressSessionMessages(session.messages);
      session.messages = packed.messages;
      if (packed.foldedFacts.length || packed.foldedHypotheses.length) {
        const next = { ...session.harness };
        next.confirmedFacts = [...(next.confirmedFacts || []), ...packed.foldedFacts].slice(-12);
        next.hypotheses = [...(next.hypotheses || []), ...packed.foldedHypotheses].slice(-6);
        session.harness = next;
      }
    }
    const emit = event => { try { this.onEvent({ sessionId, ...event }); } catch { /* observer isolated */ } };
    this._onText = delta => { if (firstTextMs === null) firstTextMs = performance.now() - started; session._producedText = true; emit({ type: 'text_delta', text: delta }); };
    let usedToolThisPrompt = false;
    let toolNamesThisPrompt = [];
    let forcedToolHint = false;
    let forcedEmptyBlock = false;
    let forcedVerifier = false;
    let injectedReview = false;
    let apiUsed = this.useResponsesApi ? 'responses' : 'chat';
    try {
      for (let step = 0; step < this.maxSteps; step++) {
        const tools = await this.#tools(session);
        const wantForceTool = this.forceToolOnDataQuestion && !usedToolThisPrompt && tools.length && looksLikeDataQuestion(goalText);
        const toolChoice = wantForceTool ? 'required' : undefined;

        if (!injectedReview && shouldInjectReviewer(session.harness, { answerMode: this.answerMode, step })) {
          injectedReview = true;
          session.harness = noteReviewDone(session.harness, step);
          const hint = isDeadEnd(session.harness) ? `${REVIEWER_HINT}\n\n${DEEP_THINK_HINT}` : REVIEWER_HINT;
          session.messages.push({ role: 'user', content: hint });
          emit({ type: 'tool_call', toolCallId: `review-${step}`, title: '方向审查', status: 'in_progress' });
          emit({ type: 'tool_update', toolCallId: `review-${step}`, title: '方向审查', status: 'completed', summary: 'harness reviewer' });
        }

        const result = await this.#modelTurn(session, tools, { toolChoice });
        apiUsed = result.api;
        const turn = result.turn;

        session.messages.push({
          role: 'assistant',
          content: turn.text || null,
          ...(turn.toolCalls.length ? { tool_calls: turn.toolCalls.map(call => ({ id: call.id, type: 'function', function: { name: call.name, arguments: call.arguments } })) } : {}),
        });

        if (!turn.toolCalls.length) {
          const block = shouldBlockEmptyCompletion(session.harness, {
            answerMode: this.answerMode,
            usedToolThisPrompt,
            forcedToolHint: forcedToolHint || forcedEmptyBlock,
            assistantText: turn.text || '',
            userText: goalText,
          });
          if (block.block && !forcedEmptyBlock && tools.length) {
            forcedEmptyBlock = true;
            forcedToolHint = true;
            emit({ type: 'tool_call', toolCallId: `goal-${step}`, title: '目标未满足·需补查', status: 'in_progress' });
            emit({ type: 'tool_update', toolCallId: `goal-${step}`, title: '目标未满足·需补查', status: 'completed', summary: block.reason });
            session.messages.push({ role: 'user', content: block.hint });
            continue;
          }
          if (
            this.toolHintGuard
            && !forcedToolHint
            && !usedToolThisPrompt
            && tools.length
            && looksLikeFactualClaim(turn.text || '')
          ) {
            forcedToolHint = true;
            emit({ type: 'tool_call', toolCallId: `hint-${step}`, title: '需要先查项目数据', status: 'in_progress' });
            emit({ type: 'tool_update', toolCallId: `hint-${step}`, title: '需要先查项目数据', status: 'completed', summary: 'quality guard: force one tool verify pass' });
            session.messages.push({ role: 'user', content: DATA_TOOL_HINT });
            continue;
          }
          // Lightweight verifier before end_turn (at most once).
          if (!forcedVerifier) {
            const verdict = verifyBeforeEnd(session.harness, {
              answerMode: this.answerMode,
              usedToolThisPrompt,
              assistantText: turn.text || '',
              userText: goalText,
              toolNamesThisPrompt,
            });
            if (!verdict.ok && tools.length) {
              forcedVerifier = true;
              forcedToolHint = true;
              emit({ type: 'tool_call', toolCallId: `verify-${step}`, title: '结束前验收·证据缺口', status: 'in_progress' });
              emit({ type: 'tool_update', toolCallId: `verify-${step}`, title: '结束前验收·证据缺口', status: 'completed', summary: verdict.reason });
              session.messages.push({ role: 'user', content: verdict.hint });
              continue;
            }
          }
          return {
            stopReason: turn.finishReason === 'length' ? 'max_tokens' : 'end_turn',
            metrics: {
              firstTextMs, firstToolMs, totalMs: performance.now() - started,
              forcedToolHint, forcedEmptyBlock, forcedVerifier,
              api: apiUsed, reasoningEffort: this.reasoningEffort,
              messageCount: session.messages.length,
            },
            harness: session.harness,
          };
        }

        if (firstToolMs === null) firstToolMs = performance.now() - started;
        usedToolThisPrompt = true;
        const calls = turn.toolCalls.map(call => {
          let args = {};
          try { args = JSON.parse(call.arguments || '{}'); } catch { /* invalid args reported by tool */ }
          return { id: call.id, name: call.name, arguments: args };
        });
        for (const call of calls) {
          toolNamesThisPrompt.push(call.name);
          emit({ type: 'tool_call', toolCallId: call.id, title: call.name, status: 'in_progress' });
        }
        const results = await runToolCalls(calls, call => this.host.mcp(session.token, session.conversationId, 'tools/call', { name: call.name, arguments: call.arguments }));
        session.harness = absorbToolResults(session.harness, results.map((result, index) => ({
          ...result,
          arguments: calls[index]?.arguments,
        })));
        for (const result of results) {
          emit({ type: 'tool_update', toolCallId: result.id, title: result.name, status: result.ok ? 'completed' : 'failed', summary: result.content.slice(0, 240) });
          session.messages.push({ role: 'tool', tool_call_id: result.id, content: result.content, name: result.name });
        }
        if (isDeadEnd(session.harness) && !injectedReview) {
          injectedReview = true;
          session.messages.push({ role: 'user', content: `${REVIEWER_HINT}\n\n${DEEP_THINK_HINT}` });
          session.harness = noteReviewDone(session.harness, step);
        }
        // Soft compress after tool waves only when session context is growing large.
        if (session.messages.length > 12 || Buffer.byteLength(JSON.stringify(session.messages)) > 48_000) {
          const packed = compressSessionMessages(session.messages);
          session.messages = packed.messages;
        }
      }
      return {
        stopReason: 'max_turn_requests',
        metrics: {
          firstTextMs, firstToolMs, totalMs: performance.now() - started,
          forcedToolHint, forcedEmptyBlock, forcedVerifier,
          api: apiUsed, reasoningEffort: this.reasoningEffort,
          messageCount: session.messages.length,
        },
        harness: session.harness,
      };
    } catch (error) {
      if (session.abort.signal.aborted) {
        return {
          stopReason: 'cancelled',
          metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed },
          harness: session.harness,
        };
      }
      throw error;
    } finally { session.abort = null; this._onText = () => {}; }
  }

  async cancel(sessionId) { const session = this.sessions.get(sessionId); if (!session?.abort) return false; session.abort.abort(); return true; }
  async resolvePermission() { return false; }
  async close() { for (const session of this.sessions.values()) session.abort?.abort(); this.sessions.clear(); this.status.state = 'closed'; }
}
