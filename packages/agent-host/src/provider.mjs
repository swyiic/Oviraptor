// xAI / OpenAI-compatible model provider for the agent host.
// Auth: (1) Grok OAuth device-code via public Grok-CLI client — access+refresh stored
// locally; Bearer access_token calls api.x.ai. (2) Manual API key as advanced fallback.
// Secrets file is 0600; settings/events/errors never return tokens or keys.
import { mkdirSync, readFileSync, writeFileSync, rmSync, existsSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { hostError } from './store.mjs';
import { proxyFetch, networkFetch } from './net-proxy.mjs';
import {
  PLANNING_GUIDANCE,
  DATA_TOOL_HINT,
  answerModeOptions,
  buildQualitySystemPrompt,
  looksLikeFactualClaim,
  looksLikeDataQuestion,
  isDataReadTool,
  isProjectReadQuestion,
  isInventoryCountQuestion,
  certificateTaskIntent,
  scanningTaskIntent,
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
  qualifyUnverifiedEnd,
  publicInnerState,
} from './agent-state.mjs';
import { admitContext, resolveModelCapability } from './workflow-guard.mjs';
import { compressSessionMessages } from './memory.mjs';

export { PLANNING_GUIDANCE, DATA_TOOL_HINT, buildQualitySystemPrompt } from './quality-prompt.mjs';
export { compressMemory, compressSessionMessages } from './memory.mjs';

export const PROVIDERS = Object.freeze({
  xai: { label: 'xAI Grok API', baseUrl: 'https://api.x.ai/v1', model: 'grok-4.7' },
  'openai-compatible': { label: 'OpenAI-compatible endpoint', baseUrl: '', model: '' },
});
export const DEEPSEEK_BASE_URL = 'https://api.deepseek.com';
export function providerHttpError(status) {
  const code = status === 401 || status === 403 ? 'host_provider_unauthorized'
    : status === 402 ? 'host_provider_quota_exhausted'
    : status === 429 ? 'host_provider_rate_limited'
    : status === 400 || status === 422 ? 'host_provider_request_invalid'
    : status === 404 ? 'host_provider_model_or_endpoint_missing'
    : status >= 500 ? 'host_provider_service_unavailable' : 'host_provider_request_failed';
  return hostError(code, 502, { httpStatus: status, outcomeUnknown: false });
}

const isDeepSeekModel = model => /deepseek/i.test(String(model || ''));
const isXaiBase = base => { try { return new URL(base).origin === 'https://api.x.ai'; } catch { return false; } };
const isDeepSeekBase = base => { try { return new URL(base).origin === DEEPSEEK_BASE_URL; } catch { return false; } };
const isXaiOAuthEndpoint = (provider, baseUrl) => provider === 'xai' && String(baseUrl || '').replace(/\/+$/, '') === PROVIDERS.xai.baseUrl;
/** Keep provider, baseUrl and model consistent: a DeepSeek model never rides the xAI endpoint. */
export function normalizeProviderModel(input = {}) {
  const out = { ...input };
  const model = String(out.model || '').trim();
  if (isDeepSeekModel(model)) {
    out.model = model.replace(/^(openai|deepseek)\//i, '');
    if (!out.baseUrl || isXaiBase(out.baseUrl)) {
      out.provider = 'openai-compatible';
      out.baseUrl = DEEPSEEK_BASE_URL;
    }
  } else if (/^(xai\/)?grok/i.test(model)) {
    out.model = model.replace(/^xai\//i, '');
    if (out.provider === 'openai-compatible' && (!out.baseUrl || isDeepSeekBase(out.baseUrl))) {
      out.provider = 'xai';
      out.baseUrl = PROVIDERS.xai.baseUrl;
    }
  }
  return out;
}
const keySlot = (provider, baseUrl) => `${provider}|${String(baseUrl || '').replace(/\/+$/, '')}`;
export const MAX_TOOL_RESULT_BYTES = 16_384;
export const MAX_AGENT_STEPS = 12;

const loopback = host => host === '127.0.0.1' || host === 'localhost' || host === '[::1]';
function connectionReference(value) {
  if (value == null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value) || value.kind !== 'models'
    || !Number.isSafeInteger(value.revision) || value.revision < 1
    || ![value.policyId, value.modelProfileId].every(id => typeof id === 'string' && /^[A-Za-z0-9._:-]{1,128}$/.test(id))) throw hostError('host_provider_connection_ref_invalid');
  return { policyId: value.policyId, revision: value.revision, kind: 'models', modelProfileId: value.modelProfileId };
}
const sameReference = (left, right) => JSON.stringify(left || null) === JSON.stringify(right || null);
export function validateProviderConfig(raw = {}) {
  const input = normalizeProviderModel(raw);
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
  #raw() {
    if (!existsSync(this.file)) return null;
    try { return JSON.parse(readFileSync(this.file, 'utf8')); } catch { return null; }
  }
  #savedCredentials(raw = {}) {
    const keys = { ...(raw?.keys ?? {}) };
    if (raw?.apiKey && raw.authentication !== 'oauth' && raw.provider && raw.baseUrl) keys[keySlot(raw.provider, raw.baseUrl)] ??= raw.apiKey;
    // Legacy files stored OAuth at the top level. Tokens belong to Grok even if
    // an old model switch accidentally attached a different provider endpoint.
    const candidate = raw?.oauth?.accessToken ? raw.oauth : raw?.authentication === 'oauth' && raw.accessToken ? raw : null;
    const oauth = candidate ? {
      provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl,
      model: /^(?:xai\/)?grok/i.test(String(candidate.model || '')) ? String(candidate.model).replace(/^xai\//i, '') : PROVIDERS.xai.model,
      authentication: 'oauth', accessToken: String(candidate.accessToken),
      ...(candidate.refreshToken ? { refreshToken: String(candidate.refreshToken) } : {}),
      ...(candidate.expiresAt ? { expiresAt: Number(candidate.expiresAt) } : {}),
      ...(candidate.tokenType ? { tokenType: String(candidate.tokenType) } : {}),
      ...(candidate.scope ? { scope: String(candidate.scope) } : {}),
    } : null;
    return { keys, oauth };
  }
  read() {
    const raw = this.#raw();
    if (!raw) return null;
    if (raw.authentication === 'oauth') return this.#savedCredentials(raw).oauth;
    const { keys: _keys, oauth: _oauth, accessToken: _access, refreshToken: _refresh, expiresAt: _expiry, tokenType: _tokenType, scope: _scope, ...value } = raw;
    const normalized = normalizeProviderModel(value);
    if (normalized.provider === value.provider && normalized.baseUrl === value.baseUrl) return { ...value, model: normalized.model };
    // Repair a stale mismatched combo (e.g. xai + openai/deepseek-*): use the key saved for that endpoint.
    const slot = keySlot(normalized.provider, normalized.baseUrl);
    const apiKey = raw.keys?.[slot] || this.#envKey(normalized.baseUrl);
    return { ...value, provider: normalized.provider, baseUrl: normalized.baseUrl, model: normalized.model, authentication: apiKey ? 'api-key' : null, apiKey: apiKey || undefined };
  }
  #envKey(baseUrl) {
    try { if (new URL(baseUrl).origin === DEEPSEEK_BASE_URL) return String(process.env.DEEPSEEK_API_KEY || '').trim() || undefined; } catch { /* no ambient credential for other endpoints */ }
    return undefined;
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
    return null;
  }
  save(input = {}) {
    const previous = this.read();
    const custom = previous?.provider === 'openai-compatible' && !isXaiBase(previous.baseUrl) && !isDeepSeekBase(previous.baseUrl);
    const next = validateProviderConfig(custom && input.provider === undefined && input.baseUrl === undefined
      ? { provider: previous.provider, baseUrl: previous.baseUrl, ...input }
      : input);
    if (input.authType !== undefined && !['api-key', 'none'].includes(input.authType)) throw hostError('host_provider_auth_type_invalid');
    if (input.clearApiKey !== undefined && typeof input.clearApiKey !== 'boolean') throw hostError('host_provider_auth_type_invalid');
    if (input.authType === 'none' && (next.apiKey !== undefined || input.clearApiKey !== true)) throw hostError('host_provider_auth_type_invalid');
    const sameEndpoint = previous?.provider === next.provider && previous.baseUrl === next.baseUrl;
    const unchanged = sameEndpoint && previous.model === next.model && next.apiKey === undefined && !input.clearApiKey;
    const connectionRef = input.connectionRef === undefined ? (unchanged ? connectionReference(previous?.connectionRef) : null) : connectionReference(input.connectionRef);
    const { keys, oauth } = this.#savedCredentials(this.#raw());
    if (previous?.apiKey && previous.provider) keys[keySlot(previous.provider, previous.baseUrl)] ??= previous.apiKey;
    if (input.clearApiKey) delete keys[keySlot(next.provider, next.baseUrl)];
    if (input.authType === 'none' || (input.authType === undefined && sameEndpoint && previous.authentication === 'none' && next.apiKey === undefined)) {
      this.#write({ provider: next.provider, baseUrl: next.baseUrl, model: next.model, authentication: 'none', connectionRef, keys, ...(oauth ? { oauth } : {}) });
      return this.publicView();
    }
    // A different catalog connection cannot inherit another connection's key at the same URL.
    if (connectionRef && next.apiKey === undefined && !sameReference(connectionRef, previous?.connectionRef)) throw hostError('host_provider_api_key_required');
    if (next.apiKey === undefined) {
      if (input.authType !== 'api-key' && oauth && isXaiOAuthEndpoint(next.provider, next.baseUrl) && (!sameEndpoint || previous.authentication === 'oauth')) {
        const selected = { ...oauth, model: next.model };
        this.#write({ ...selected, keys, oauth: selected });
        return this.publicView();
      }
      next.apiKey = !input.clearApiKey && sameEndpoint && previous?.apiKey ? previous.apiKey : keys[keySlot(next.provider, next.baseUrl)] || (!connectionRef && !input.clearApiKey ? this.#envKey(next.baseUrl) : undefined);
      if (!next.apiKey) throw hostError('host_provider_api_key_required');
    }
    keys[keySlot(next.provider, next.baseUrl)] = next.apiKey;
    const stored = { provider: next.provider, baseUrl: next.baseUrl, model: next.model, apiKey: next.apiKey, authentication: 'api-key', connectionRef, keys, ...(oauth ? { oauth } : {}) };
    this.#write(stored);
    return this.publicView();
  }
  /** Explicit authentication selection; ordinary model edits keep their auth. */
  selectOAuth() {
    const { keys, oauth } = this.#savedCredentials(this.#raw());
    if (!oauth) throw hostError('host_oauth_not_configured');
    this.#write({ ...oauth, keys, oauth });
    return this.publicView();
  }
  /** Persist OpenCode-compatible OAuth tokens (access + refresh). */
  saveOAuth(tokens, { model, baseUrl, provider = 'xai' } = {}) {
    if (!tokens?.accessToken) throw hostError('host_oauth_token_invalid');
    if (provider !== 'xai' || (baseUrl && !isXaiOAuthEndpoint(provider, baseUrl))) throw hostError('host_oauth_provider_invalid');
    const { keys, oauth } = this.#savedCredentials(this.#raw());
    const selected = validateProviderConfig({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: model || oauth?.model || PROVIDERS.xai.model });
    if (!isXaiOAuthEndpoint(selected.provider, selected.baseUrl)) throw hostError('host_oauth_provider_invalid');
    const stored = {
      ...selected,
      authentication: 'oauth',
      accessToken: String(tokens.accessToken),
      ...(tokens.refreshToken || oauth?.refreshToken ? { refreshToken: String(tokens.refreshToken || oauth.refreshToken) } : {}),
      ...(tokens.expiresAt ? { expiresAt: Number(tokens.expiresAt) } : {}),
      ...(tokens.tokenType ? { tokenType: String(tokens.tokenType) } : {}),
      ...(tokens.scope ? { scope: String(tokens.scope) } : {}),
    };
    this.#write({ ...stored, keys, oauth: stored });
    return this.publicView();
  }
  /** Sign out of Grok without deleting independently configured API providers. */
  clearOAuth() {
    const raw = this.#raw();
    if (!raw) return this.publicView();
    const { keys } = this.#savedCredentials(raw);
    if (raw.authentication !== 'oauth') {
      const { oauth: _oauth, accessToken: _access, refreshToken: _refresh, expiresAt: _expiry, tokenType: _tokenType, scope: _scope, ...rest } = raw;
      this.#write({ ...rest, keys });
    } else {
      const apiKey = keys[keySlot('xai', PROVIDERS.xai.baseUrl)];
      this.#write({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: this.read()?.model || PROVIDERS.xai.model, authentication: apiKey ? 'api-key' : null, ...(apiKey ? { apiKey } : {}), keys });
    }
    return this.publicView();
  }
  clear() { rmSync(this.file, { force: true }); return this.publicView(); }
  publicView() {
    const value = this.read();
    if (!value) {
      return { configured: false, apiKeyConfigured: false, oauthConfigured: false, authentication: null, authType: null, connectionRef: null, oauthAvailable: true };
    }
    const oauth = value.authentication === 'oauth' && Boolean(value.accessToken);
    const apiKey = Boolean(value.apiKey);
    return {
      configured: oauth || apiKey || value.authentication === 'none',
      provider: value.provider,
      label: PROVIDERS[value.provider]?.label,
      baseUrl: value.baseUrl,
      model: value.model,
      apiKeyConfigured: apiKey,
      oauthConfigured: oauth,
      oauthSaved: Boolean(this.#savedCredentials(this.#raw()).oauth?.accessToken),
      authentication: oauth ? 'oauth' : apiKey ? 'api-key' : value.authentication === 'none' ? 'none' : null,
      authType: oauth ? 'oauth' : apiKey ? 'api-key' : value.authentication === 'none' ? 'none' : null,
      connectionRef: oauth ? null : connectionReference(value.connectionRef),
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
  host_task_revision_conflict: 'Read host_task_read again and use its current task revision and each existing step revision. Successful data/file tools also advance the task revision. Do not guess or reuse an earlier revision.',
  host_task_criteria_invalid: 'Re-check criteria inputSchema. Each criterion must use exactly the fields for its kind: file_exists {id,kind,artifactId}; file_contains adds contains; tool_json_equals {id,kind,toolName,path,expected,evidenceId?}; tool_read_equals uses arguments? instead of evidenceId. path is an array of JSON field keys. Artifact IDs must already exist in host_task_read.',
  host_task_artifact_unverifiable: 'Read host_task_read and correct the artifact location to a workspace-relative path (for example summary.json), or an absolute path inside the agent workspace. Do not use file:// URLs. Then read the file through workspace_read before verifying again.',
  host_task_acceptance_required: 'Read host_task_read; finish steps with actual successful evidence, declare criteria from the user goal, then call host_task_verify. Complete only after a successful receipt, using a separate host_task_update with expectedRevision and complete:true.',
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
        const ok = outcome.value?.isError !== true;
        results.set(call.id, { id: call.id, name: call.name, ok, advisory: outcome.value?._meta?.hostEvidence === 'advisory', content: cut.text, truncated: cut.truncated, ...(!ok ? { code: 'host_tool_result_error' } : {}) });
      } else {
        const code = typeof outcome.reason?.code === 'string' ? outcome.reason.code : 'host_tool_failed';
        results.set(call.id, { id: call.id, name: call.name, ok: false, content: JSON.stringify({ error: code, hint: strategyHint(code) }), code });
      }
    });
  }
  return calls.map(call => results.get(call.id));
}

const DSML_BAR = '[|｜]+';
const DSML_OPEN = new RegExp(`<\\s*${DSML_BAR}\\s*DSML`, 'i');
const DSML_BLOCK = new RegExp(`<\\s*${DSML_BAR}\\s*DSML\\s*${DSML_BAR}\\s*(?:tool_)?calls\\s*>[\\s\\S]*?<\\s*/\\s*${DSML_BAR}\\s*DSML\\s*${DSML_BAR}\\s*(?:tool_)?calls\\s*>`, 'gi');

/** DeepSeek sometimes writes tool calls as DSML text instead of tool_calls. */
export function extractDsmlToolCalls(raw) {
  const calls = [];
  const text = String(raw ?? '').replace(DSML_BLOCK, block => {
    const invoke = new RegExp(`<\\s*${DSML_BAR}\\s*DSML\\s*${DSML_BAR}\\s*invoke\\s+name\\s*=\\s*"([^"]+)"\\s*>([\\s\\S]*?)<\\s*/\\s*${DSML_BAR}\\s*DSML\\s*${DSML_BAR}\\s*invoke\\s*>`, 'gi');
    const param = new RegExp(`<\\s*${DSML_BAR}\\s*DSML\\s*${DSML_BAR}\\s*parameter\\s+name\\s*=\\s*"([^"]+)"[^>]*>([\\s\\S]*?)<\\s*/\\s*${DSML_BAR}\\s*DSML\\s*${DSML_BAR}\\s*parameter\\s*>`, 'gi');
    for (const match of block.matchAll(invoke)) {
      const args = {};
      for (const item of match[2].matchAll(param)) args[item[1]] = item[2].trim();
      calls.push({ id: `dsml_${calls.length + 1}`, name: match[1], arguments: JSON.stringify(args) });
    }
    return '';
  });
  return { text: text.replace(/\n{3,}/g, '\n\n').trim(), calls };
}

function executionSignature(call) {
  const canonical = (value, depth = 0) => {
    if (depth > 32) return '[depth-limit]';
    if (Array.isArray(value)) return value.map(item => canonical(item, depth + 1));
    if (value && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key], depth + 1)]));
    return value;
  };
  return `${call.name}:${createHash('sha256').update(JSON.stringify(canonical(call.arguments))).digest('hex')}`;
}
export function providerToolName(name) {
  const value = String(name);
  if (/^[A-Za-z0-9_-]{1,64}$/.test(value)) return value;
  const short = value.split(/::|__/).at(-1).replace(/[^A-Za-z0-9_-]/g, '_').slice(0, 50) || 'tool';
  return `t_${createHash('sha256').update(value).digest('hex').slice(0, 10)}__${short}`;
}
function providerTools(listed) {
  const seen = new Set();
  return listed.map(tool => {
    const name = providerToolName(tool.name);
    if (seen.has(name)) throw hostError('host_provider_tool_names_ambiguous');
    seen.add(name);
    return { type: 'function', _hostName: tool.name, function: { name, description: tool.description ?? '', parameters: tool.inputSchema ?? { type: 'object' } } };
  });
}
function matchListedTool(name, tools) {
  const names = tools.map(tool => tool.function?.name).filter(Boolean);
  if (names.includes(name)) return name;
  const exact = tools.find(tool => tool._hostName === name);
  if (exact) return exact.function.name;
  const hits = names.filter(item => item.endsWith(`__${name}`) || item.endsWith(`::${name}`));
  return hits.length === 1 ? hits[0] : name;
}

/** Parse an OpenAI-compatible chat.completions SSE stream into text deltas and assembled tool calls. */
export async function readChatStream(body, onText) {
  const decoder = new TextDecoder(); let buffer = '', raw = '', shown = '', finish = null, reasoning = '';
  const tools = [];
  let refusal = null;
  const publish = () => {
    const open = raw.search(DSML_OPEN);
    const visible = open < 0 ? raw : raw.slice(0, open);
    const cut = open < 0 ? visible.lastIndexOf('<') : -1;
    const hold = cut >= 0 && visible.length - cut < 32 ? visible.slice(cut) : '';
    const safe = hold ? visible.slice(0, visible.length - hold.length) : visible;
    if (safe.startsWith(shown) && safe.length > shown.length) {
      onText(safe.slice(shown.length));
      shown = safe;
    }
  };
  const handle = data => {
    if (data === '[DONE]') return;
    let chunk; try { chunk = JSON.parse(data); } catch { return; }
    if (chunk.error) throw hostError('host_provider_stream_failed', 502, { outcomeUnknown: true });
    const choice = chunk.choices?.[0]; if (!choice) return;
    const delta = choice.delta ?? {};
    if (delta.refusal) {
      const explanation = typeof delta.refusal === 'string' ? delta.refusal : String(delta.refusal.explanation || delta.refusal.reason || '');
      refusal = { code: 'model_refusal', category: typeof delta.refusal === 'object' && delta.refusal.category ? String(delta.refusal.category).slice(0, 80) : 'model_refusal', explanation: explanation.slice(0, 240) };
      finish = 'refusal';
    }
    if (typeof delta.reasoning_content === 'string') {
      reasoning += delta.reasoning_content;
      if (Buffer.byteLength(reasoning) > 512 * 1024) throw hostError('host_provider_reasoning_too_large', 502);
    }
    if (typeof delta.content === 'string' && delta.content) { raw += delta.content; publish(); }
    for (const part of delta.tool_calls ?? []) {
      const index = part.index ?? tools.length;
      tools[index] ??= { id: '', name: '', arguments: '' };
      if (part.id) tools[index].id = part.id;
      if (part.function?.name) tools[index].name += part.function.name;
      if (part.function?.arguments) tools[index].arguments += part.function.arguments;
    }
    if (choice.finish_reason === 'refusal') finish = 'refusal';
    else if (choice.finish_reason && finish !== 'refusal') finish = choice.finish_reason;
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
  if (!finish) throw hostError('host_provider_stream_incomplete', 502, { outcomeUnknown: true });
  if (!['stop', 'tool_calls', 'length', 'content_filter', 'refusal'].includes(finish)) throw hostError('host_provider_stream_failed', 502, { outcomeUnknown: true });
  const parsed = extractDsmlToolCalls(raw);
  if (!refusal && parsed.text.startsWith(shown) && parsed.text.length > shown.length) onText(parsed.text.slice(shown.length));
  if (finish === 'refusal' || refusal) {
    return { text: refusal?.explanation || parsed.text, toolCalls: [], finishReason: 'refusal', refusal: refusal ?? { code: 'model_refusal', category: 'model_refusal', explanation: '' } };
  }
  const toolCalls = [...tools.filter(Boolean), ...parsed.calls];
  if (finish === 'tool_calls' && !toolCalls.length) throw hostError('host_provider_tool_calls_invalid', 502, { outcomeUnknown: true });
  return { text: parsed.text, toolCalls, finishReason: finish, ...(reasoning ? { reasoningContent: reasoning } : {}), ...(finish === 'content_filter' ? { refusal: { code: 'model_refusal', category: 'content_filter', explanation: '' } } : {}) };
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
  let refusal = null;
  const handleEvent = (eventType, data) => {
    if (data === '[DONE]') return;
    let chunk; try { chunk = JSON.parse(data); } catch { return; }
    const type = eventType || chunk.type || '';
    if (type === 'error' || type === 'response.failed' || chunk.error || chunk.response?.status === 'failed') throw hostError('host_provider_stream_failed', 502, { outcomeUnknown: true });
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
      } else if (item.type === 'refusal') {
        refusal = { code: 'model_refusal', category: 'model_refusal', explanation: String(item.refusal || item.content || '').slice(0, 240) };
        finish = 'refusal';
        outputItems.push(item);
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
      if (chunk.response?.status && chunk.response.status !== 'completed') throw hostError('host_provider_stream_incomplete', 502, { outcomeUnknown: true });
      if (!refusal) finish = 'stop';
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
            if (part.type === 'refusal') {
              refusal = { code: 'model_refusal', category: 'model_refusal', explanation: String(part.refusal || part.text || '').slice(0, 240) };
              finish = 'refusal';
            } else if (part.type === 'output_text' && typeof part.text === 'string' && !text.includes(part.text)) {
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
    if (type === 'response.incomplete') {
      const reason = chunk.response?.incomplete_details?.reason ?? chunk.incomplete_details?.reason;
      if (reason && reason !== 'max_output_tokens') throw hostError('host_provider_stream_incomplete', 502, { outcomeUnknown: true });
      finish = 'length';
    }
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
  if (!finish) throw hostError('host_provider_stream_incomplete', 502, { outcomeUnknown: true });
  if (finish === 'refusal' || refusal) {
    return { text: refusal?.explanation || text, toolCalls: [], finishReason: 'refusal', refusal: refusal ?? { code: 'model_refusal', category: 'model_refusal', explanation: '' }, responseId, outputItems };
  }
  if (!tools.length && toolByIndex.size) {
    for (const [, value] of [...toolByIndex.entries()].sort((a, b) => a[0] - b[0])) tools.push(value);
  }
  if (tools.some(tool => !tool.name)) throw hostError('host_provider_tool_calls_invalid', 502, { outcomeUnknown: true });
  return { text, toolCalls: tools, finishReason: finish, responseId, outputItems };
}

const mcpServerEnv = (servers, name) => servers?.[0]?.env?.find?.(item => item.name === name)?.value;

function awaitWhileActive(promise, signal) {
  return new Promise((resolve, reject) => {
    const abort = () => { cleanup(); reject(hostError('host_reply_cancelled')); };
    const cleanup = () => signal.removeEventListener('abort', abort);
    if (signal.aborted) { abort(); return; }
    signal.addEventListener('abort', abort, { once: true });
    Promise.resolve(promise).then(value => { cleanup(); resolve(value); }, error => { cleanup(); reject(error); });
  });
}

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
    if (!bearer && config?.authentication !== 'none') throw hostError('host_provider_api_key_required');
    if (config.authentication === 'oauth' && !isXaiOAuthEndpoint(config.provider, config.baseUrl)) throw hostError('host_oauth_provider_invalid');
    this.answerMode = normalizeAnswerMode(answerMode);
    const mode = answerModeOptions(this.answerMode);
    this.config = { ...config, apiKey: bearer }; this.fetch = fetchImpl; this.onEvent = onEvent;
    this.maxSteps = maxSteps ?? mode.maxSteps;
    this.stepLimitOverride = maxSteps !== undefined && maxSteps !== mode.maxSteps ? maxSteps : null;
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
  /** After a model switch the in-memory transcript is gone. Fill it once from the saved conversation. */
  seedTranscriptIfEmpty(sessionId, messages = []) {
    const session = this.sessions.get(sessionId);
    if (!session || session.messages?.length) return 0;
    const usable = [];
    for (const message of messages) {
      if (message?.role !== 'user' && message?.role !== 'assistant') continue;
      const content = String(message.content ?? '').trim();
      if (!content) continue;
      usable.push({ role: message.role, content });
    }
    const limit = 40;
    let seeded = usable;
    if (usable.length > limit) {
      const firstUser = usable.find(item => item.role === 'user');
      const tail = usable.slice(-(limit - 1));
      seeded = firstUser && !tail.includes(firstUser) ? [firstUser, ...tail] : usable.slice(-limit);
    }
    session.messages = seeded;
    return seeded.length;
  }
  async #tools(session) {
    if (!this.host || !session.token) return [];
    const listed = await this.host.mcp(session.token, session.conversationId, 'tools/list', {}, { runId: session.runId });
    return providerTools(listed.tools ?? []);
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
    this.maxSteps = this.stepLimitOverride ?? mode.maxSteps;
  }
  async #chatTurn(session, tools, { toolChoice, onText } = {}) {
    const options = session.options;
    const config = session.config;
    const body = {
      model: config.model,
      stream: true,
      temperature: options.temperature,
      max_tokens: options.maxTokens,
      messages: session.messages.map(({ _hostUserText, ...message }) => message),
      ...(tools.length ? { tools: tools.map(({ _hostName, ...tool }) => tool), parallel_tool_calls: true } : {}),
      ...(isDeepSeekModel(config.model) || isDeepSeekBase(config.baseUrl) ? {
        thinking: { type: options.answerMode === 'quality' ? 'enabled' : 'disabled' },
        ...(toolChoice ? { tool_choice: options.answerMode === 'quality' ? 'auto' : toolChoice } : {}),
      } : (toolChoice ? { tool_choice: toolChoice } : {})),
    };
    const response = await networkFetch(this.fetch, `${config.baseUrl}/chat/completions`, {
      method: 'POST', redirect: 'error',
      headers: { ...(config.authentication === 'none' ? {} : { Authorization: `Bearer ${config.apiKey}` }), 'Content-Type': 'application/json', Accept: 'text/event-stream' },
      body: JSON.stringify(body),
      signal: AbortSignal.any([session.abort.signal, AbortSignal.timeout(options.timeoutMs)]),
    });
    if (!response.ok || !response.body) throw providerHttpError(response.status);
    return { api: 'chat', turn: await readChatStream(response.body, onText) };
  }
  async #responsesTurn(session, tools, { toolChoice, onText } = {}) {
    const options = session.options;
    const config = session.config;
    const effort = normalizeReasoningEffort(options.reasoningEffort, options.answerMode);
    const input = [
      ...chatMessagesToResponsesInput(session.messages),
      ...((session.reasoningItems ?? []).filter(item => item?.type === 'reasoning')),
    ];
    const body = {
      model: config.model,
      stream: true,
      store: false,
      max_output_tokens: options.maxTokens,
      reasoning: { effort },
      input,
      ...(tools.length ? { tools: toResponsesTools(tools), parallel_tool_calls: true } : {}),
      ...(toolChoice ? { tool_choice: toolChoice } : {}),
      prompt_cache_key: session.conversationId || session.sessionId || 'oviraptor-agent',
    };
    const response = await networkFetch(this.fetch, `${config.baseUrl}/responses`, {
      method: 'POST', redirect: 'error',
      headers: {
        ...(config.authentication === 'none' ? {} : { Authorization: `Bearer ${config.apiKey}` }),
        'Content-Type': 'application/json',
        Accept: 'text/event-stream',
      },
      body: JSON.stringify(body),
      signal: AbortSignal.any([session.abort.signal, AbortSignal.timeout(options.timeoutMs)]),
    });
    if (!response.ok || !response.body) {
      // Fall back to chat completions once if Responses unavailable on this endpoint.
      if (response.status === 404 || response.status === 405 || response.status === 501) {
        return this.#chatTurn(session, tools, { toolChoice, onText });
      }
      throw providerHttpError(response.status);
    }
    const turn = await readResponsesStream(response.body, onText);
    session.previousResponseId = turn.responseId ?? session.previousResponseId;
    session.reasoningItems = (turn.outputItems ?? []).filter(item => item?.type === 'reasoning');
    return { api: 'responses', turn };
  }
  /** One model HTTP turn; on 401, ask host to refresh OAuth once and retry if no text yet. */
  async #modelTurn(session, tools, { toolChoice, onText } = {}) {
    const attempt = async () => (session.options.useResponsesApi
      ? await this.#responsesTurn(session, tools, { toolChoice, onText })
      : await this.#chatTurn(session, tools, { toolChoice, onText }));
    try {
      return await attempt();
    } catch (error) {
      const unauthorized = error?.code === 'host_provider_unauthorized';
      if (!unauthorized || session._authRetried || !this.host?.ensureOAuthAccess) throw error;
      if (session.config.authentication !== 'oauth' || !isXaiOAuthEndpoint(session.config.provider, session.config.baseUrl)) throw error;
      // Only retry when nothing streamed yet (avoid double-billing partial answers).
      if (session._producedText) throw error;
      session._authRetried = true;
      try {
        const refreshed = await this.host.ensureOAuthAccess({ force: true });
        const selected = this.host.providerStore?.read?.() ?? refreshed;
        if (selected?.authentication !== 'oauth' || !isXaiOAuthEndpoint(selected.provider, selected.baseUrl) || !selected.accessToken) throw error;
        session.config.apiKey = selected.accessToken;
        if (this.config.authentication === 'oauth' && isXaiOAuthEndpoint(this.config.provider, this.config.baseUrl)) this.config.apiKey = selected.accessToken;
      } catch {
        throw error;
      }
      return await attempt();
    }
  }

  async prompt({ sessionId, prompt, userText: rawUserText, taskContext, turnId, answerMode } = {}) {
    const session = this.sessions.get(sessionId); if (!session) throw hostError('host_runtime_session_invalid');
    if (session.abort) throw hostError('host_reply_already_running', 409);
    this.#applyMode(answerMode);
    // Per-turn settings and callbacks must not be overwritten by another session
    // or by an operator changing the default while this request is streaming.
    const options = session.options = Object.fromEntries(['answerMode', 'temperature', 'maxTokens', 'toolHintGuard', 'forceToolOnDataQuestion', 'reasoningEffort', 'useResponsesApi', 'timeoutMs', 'maxSteps'].map(key => [key, this[key]]));
    session.config = { ...this.config };
    session.runId = turnId;
    const started = performance.now(); let firstTextMs = null, firstToolMs = null;
    session.abort = new AbortController();
    session._authRetried = false;
    session._producedText = false;
    const fullPrompt = typeof prompt === 'string' ? prompt : JSON.stringify(prompt);
    // Goal must be the user's raw question — never the whole turn prompt blob.
    let goalText = (typeof rawUserText === 'string' && rawUserText.trim())
      ? rawUserText.trim()
      : fullPrompt;
    // The durable plan stays in taskContext/fullPrompt. The current question owns
    // this turn's verification; an old data question cannot hijack a follow-up.
    session.harness = setGoal({ ...(session.harness ?? emptyAgentState()), openIntents: [] }, goalText);
    const system = buildQualitySystemPrompt({ answerMode: options.answerMode, reasoningEffort: options.reasoningEffort });
    const harnessBlock = formatAgentStatePromptBlock(session.harness);
    if (!session.messages.some(message => message.role === 'system')) {
      session.messages.push({ role: 'system', content: system });
    } else {
      session.messages[0] = { role: 'system', content: system };
    }
    const userContent = harnessBlock ? `${harnessBlock}\n\n${fullPrompt}` : fullPrompt;
    // Internal provenance is consumed by compaction and removed from API input.
    session.messages.push({ role: 'user', content: userContent, _hostUserText: goalText });
    // Compress API messages (same shape as SQLite memory): recent window + fold old tools into harness.
    let sawCompression = false;
    {
      const packed = compressSessionMessages(session.messages);
      session.messages = packed.messages;
      if (packed.compressed) sawCompression = true;
      if (packed.foldedFacts.length || packed.foldedHypotheses.length) {
        const next = { ...session.harness };
        next.confirmedFacts = [...(next.confirmedFacts || []), ...packed.foldedFacts].slice(-12);
        next.hypotheses = [...(next.hypotheses || []), ...packed.foldedHypotheses].slice(-6);
        session.harness = next;
      }
    }
    const emit = event => { try { this.onEvent({ sessionId, turnId, ...event }); } catch { /* observer isolated */ } };
    let holdNote = '';
    const emitInner = (phase, hold = '') => {
      if (phase === 'model') holdNote = hold;
      else if (hold) holdNote = hold;
      emit({ type: 'inner', ...publicInnerState(session.harness, { phase, hold: phase === 'model' ? hold : (hold || holdNote), compressed: sawCompression }) });
    };
    if (this.host?.admitModelRequest && session.conversationId) {
      try { this.host.admitModelRequest(session.conversationId); }
      catch (error) {
        if (error?.code === 'host_budget_hard_stop' || error?.code === 'host_budget_paused') {
          const note = '预算不足，这一轮没有发给模型，也没有升档。';
          emitInner('done', note);
          return { stopReason: 'paused_budget', replaceText: true, assistantText: note, metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, api: options.useResponsesApi ? 'responses' : 'chat', messageCount: session.messages.length }, harness: session.harness };
        }
        throw error;
      }
    }
    if (sawCompression && this.host?.recordSummary && session.conversationId) {
      const sourceIds = session.messages.flatMap(message => [message.tool_call_id, message._hostUserText].filter(Boolean)).slice(0, 32);
      try { this.host.recordSummary(session.projectId, session.conversationId, { sourceIds, body: session.messages.find(message => String(message.content || '').startsWith('上下文已压缩'))?.content || '' }); } catch { /* summary is advisory */ }
    }
    const contextFit = admitContext({ capability: resolveModelCapability(this.config), messages: session.messages, configuredInputBytes: this.config.inputByteCeiling });
    const contextBlocks = (this.config.contextMode === 'strict' && !contextFit.ok) || (Number.isSafeInteger(this.config.contextWindowTokens) && !contextFit.ok);
    if (contextBlocks) {
      const note = contextFit.reason === 'context_insufficient' ? '上下文装不下硬约束，这一轮没有发给模型。' : '模型窗口未知，严格模式下这一轮没有发给模型。';
      emitInner('done', note);
      return { stopReason: contextFit.reason === 'context_insufficient' ? 'context_insufficient' : 'unsupported_capability', replaceText: true, assistantText: note, metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, api: options.useResponsesApi ? 'responses' : 'chat', messageCount: session.messages.length }, harness: session.harness };
    }
    emitInner(sawCompression ? 'compress' : 'model');
    const onText = delta => { if (firstTextMs === null) firstTextMs = performance.now() - started; session._producedText = true; emit({ type: 'text_delta', text: delta }); };
    const consumeSteer = () => {
      const lines = this.host?.takeSteer?.(session.conversationId) ?? [];
      for (const line of lines) {
        session.messages.push({ role: 'user', content: `用户插话：${line}`, _hostUserText: line });
        goalText += `\n用户插话：${line}`;
      }
      if (lines.length) session.harness = setGoal(session.harness, goalText);
      return lines.length > 0;
    };
    let usedToolThisPrompt = false;
    let toolNamesThisPrompt = [];
    let toolResultsThisPrompt = [];
    const failedExecutions = new Map();
    let forcedToolHint = false;
    let forcedEmptyBlock = false;
    let forcedVerifier = false;
    let injectedReview = false;
    let countAnswerHint = false;
    let certificateStatusHint = false;
    let scanStatusHint = false;
    let apiUsed = options.useResponsesApi ? 'responses' : 'chat';
    let stepLimit = options.maxSteps;
    try {
      for (let step = 0; step < stepLimit; step++) {
        if (session.abort.signal.aborted) throw hostError('host_reply_cancelled');
        consumeSteer();
        emitInner('model');
        let tools = await this.#tools(session);
        const listedTools = tools;
        const dataReads = tools.filter(tool => isDataReadTool(tool._hostName));
        const statsReads = dataReads.filter(tool => String(tool._hostName).split(/::|__/).at(-1) === 'inventory_stats');
        const simpleCount = isInventoryCountQuestion(goalText) && statsReads.length > 0;
        const countReady = simpleCount && toolResultsThisPrompt.some(result => result.ok && !result.advisory
          && String(result.name).split(/::|__/).at(-1) === 'inventory_stats'
          && !/\[truncated:/i.test(result.content));
        const hasDataEvidence = toolResultsThisPrompt.some(result => result.ok && !result.advisory && isDataReadTool(result.name));
        const wantForceTool = options.forceToolOnDataQuestion && !hasDataEvidence && dataReads.length > 0 && looksLikeDataQuestion(goalText);
        if (wantForceTool || isProjectReadQuestion(goalText)) tools = dataReads;
        if (simpleCount && toolResultsThisPrompt.length === 0) tools = statsReads;
        if (countReady) {
          tools = [];
          if (!countAnswerHint) {
            countAnswerHint = true;
            session.messages.push({ role: 'user', content: 'Host: 本轮 inventory_stats 已成功返回项目统计。请直接根据该结果回答当前计数问题，不再创建计划、设置验收条件或重复查询。只报告结果实际包含的字段；缺少的字段明确说未提供。' });
          }
        }
        const certificateIntent=certificateTaskIntent(goalText);
        // Explicit job actions are not generic inventory questions or task-plan mutations.
        const availableCertificateTools=listedTools.filter(tool=>String(tool._hostName).split(/::|__/).at(-1)?.startsWith('certificate_task_'));
        const certificateTurn=Boolean(certificateIntent && availableCertificateTools.length);
        if(certificateTurn){
          const short=name=>String(name).split(/::|__/).at(-1);
          const successful=toolResultsThisPrompt.filter(result=>result.ok&&!result.advisory&&!result.truncated);
          const mutations=successful.filter(result=>/^certificate_task_(start|pause|resume)$/.test(short(result.name)));
          const reads=successful.filter(result=>short(result.name)==='certificate_task_status');
          const lastMutationIndex=toolResultsThisPrompt.findLastIndex(result=>mutations.includes(result));
          const verified=mutations.length>0&&toolResultsThisPrompt.some((result,index)=>index>lastMutationIndex&&reads.includes(result));
          let allowed=['certificate_task_status'];
          if(!verified){
            if(certificateIntent.action==='start')allowed.push('certificate_task_start',...(certificateIntent.paused?['certificate_task_pause']:['certificate_task_resume']));
            if(certificateIntent.action==='pause')allowed.push('certificate_task_pause');
            if(certificateIntent.action==='resume')allowed.push('certificate_task_resume');
          }
          tools=availableCertificateTools.filter(tool=>allowed.includes(short(tool._hostName)));
          if(mutations.length && !verified){
            tools=tools.filter(tool=>short(tool._hostName)==='certificate_task_status');
            if(!certificateStatusHint){certificateStatusHint=true;session.messages.push({role:'user',content:'Host: 证书任务控制操作已经返回。请使用返回的 taskId 调用一次 certificate_task_status 核对当前进度，然后直接答复。任务执行器会在后台处理，不要逐条调用模型，也不要循环等待任务全部完成。'});}
          }
          if(verified || certificateIntent.action==='status'&&reads.length || reads.length>=2){tools=[];}
        }
        const scanIntent=scanningTaskIntent(goalText);
        const scanTools=listedTools.filter(tool=>['scan_list','scan_status','scan_start','scan_pause','scan_resume'].includes(String(tool._hostName).split(/::|__/).at(-1)));
        const scanTurn=Boolean(scanIntent&&scanTools.length&&!certificateTurn);
        if(scanTurn){
          const short=name=>String(name).split(/::|__/).at(-1);
          const successful=toolResultsThisPrompt.filter(result=>result.ok&&!result.advisory&&!result.truncated);
          const mutations=successful.filter(result=>/^scan_(start|pause|resume)$/.test(short(result.name)));
          const reads=successful.filter(result=>['scan_status','scan_list'].includes(short(result.name)));
          const last=toolResultsThisPrompt.findLastIndex(result=>mutations.includes(result));
          const verified=mutations.length>0&&toolResultsThisPrompt.some((result,index)=>index>last&&reads.includes(result)&&short(result.name)==='scan_status');
          const attempted=toolResultsThisPrompt.some(result=>/^scan_(start|pause|resume)$/.test(short(result.name)));
          const allowed=['scan_list','scan_status',...(!attempted&&scanIntent.action!=='status'?['scan_'+scanIntent.action]:[])];
          tools=scanTools.filter(tool=>allowed.includes(short(tool._hostName)));
          if(attempted&&!verified){tools=tools.filter(tool=>short(tool._hostName)==='scan_status');if(!scanStatusHint){scanStatusHint=true;session.messages.push({role:'user',content:'Host: 扫描控制操作已返回。请使用 taskId 读取一次 scan_status，然后报告实际状态、进度或缺口。任务在后台运行，不重复启动或恢复，不循环等待完成；结果未知时先核对，不重发。'});}}
          if(verified||scanIntent.action==='status'&&reads.length||reads.length>=2)tools=[];
        }
        const toolChoice = wantForceTool && !certificateTurn && !scanTurn ? 'required' : undefined;

        if (!injectedReview && shouldInjectReviewer(session.harness, { answerMode: options.answerMode, step })) {
          injectedReview = true;
          session.harness = noteReviewDone(session.harness, step);
          const hint = isDeadEnd(session.harness) ? `${REVIEWER_HINT}\n\n${DEEP_THINK_HINT}` : REVIEWER_HINT;
          session.messages.push({ role: 'user', content: hint });
          emitInner('hold', '同一条路反复失败，先回到你的目标');
          emit({ type: 'tool_call', toolCallId: `review-${step}`, title: '方向审查', status: 'in_progress' });
          emit({ type: 'tool_update', toolCallId: `review-${step}`, title: '方向审查', status: 'completed', summary: 'harness reviewer' });
        }

        const result = await this.#modelTurn(session, tools, { toolChoice, onText });
        apiUsed = result.api;
        const turn = result.turn;
        if (session.abort.signal.aborted) throw hostError('host_reply_cancelled');
        const callIds = new Set();
        for (const call of turn.toolCalls) {
          if (typeof call.name !== 'string' || !call.name.trim()) throw hostError('host_provider_tool_calls_invalid', 502, { outcomeUnknown: true });
          call.id ||= `call_${crypto.randomUUID()}`;
          if (callIds.has(call.id)) throw hostError('host_provider_tool_calls_invalid', 502, { outcomeUnknown: true });
          callIds.add(call.id);
          call._requestedName = call.name;
          call.name = providerToolName(matchListedTool(call.name, tools));
        }

        const modelRefusal = turn.finishReason === 'refusal' || turn.finishReason === 'content_filter' || turn.refusal;
        if (modelRefusal) {
          const explanation = String(turn.refusal?.explanation || turn.text || '').slice(0, 240);
          if (explanation && !session._producedText) onText(explanation);
          session.messages.push({ role: 'assistant', content: explanation || null });
          emitInner('done', turn.finishReason === 'content_filter' ? '模型按内容策略停止' : '模型拒绝继续');
          return {
            stopReason: 'refusal',
            replaceText: !session._producedText && Boolean(explanation),
            assistantText: explanation,
            refusal: {
              code: 'model_refusal',
              category: turn.refusal?.category || (turn.finishReason === 'content_filter' ? 'content_filter' : 'model_refusal'),
              explanation,
            },
            metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed, reasoningEffort: options.reasoningEffort, messageCount: session.messages.length },
            harness: session.harness,
          };
        }

        session.messages.push({
          role: 'assistant',
          content: turn.text || null,
          ...(turn.reasoningContent ? { reasoning_content: turn.reasoningContent } : {}),
          ...(turn.toolCalls.length ? { tool_calls: turn.toolCalls.map(call => ({ id: call.id, type: 'function', function: { name: call.name, arguments: call.arguments } })) } : {}),
        });

        if (!turn.toolCalls.length) {
          // A steer accepted while this HTTP response was in flight still belongs
          // to this turn, including when the ordinary request budget just ended.
          if (consumeSteer()) {
            stepLimit = Math.max(stepLimit, step + 2);
            emitInner('hold', '正在处理你刚补充的要求');
            continue;
          }
          if (turn.finishReason === 'length' || turn.finishReason === 'content_filter') {
            return { stopReason: turn.finishReason === 'length' ? 'max_tokens' : 'refusal', harness: session.harness, metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed } };
          }
          const block = shouldBlockEmptyCompletion(session.harness, {
            answerMode: options.answerMode,
            usedToolThisPrompt,
            forcedToolHint: forcedToolHint || forcedEmptyBlock,
            assistantText: turn.text || '',
            userText: goalText,
          });
          if (block.block && !forcedEmptyBlock && tools.length) {
            forcedEmptyBlock = true;
            forcedToolHint = true;
            emitInner('hold', '目标还没完成，这一轮还没有新的工具结果');
            emit({ type: 'tool_call', toolCallId: `goal-${step}`, title: '目标未满足·需补查', status: 'in_progress' });
            emit({ type: 'tool_update', toolCallId: `goal-${step}`, title: '目标未满足·需补查', status: 'completed', summary: block.reason });
            session.messages.push({ role: 'user', content: block.hint });
            continue;
          }
          if (
            options.toolHintGuard
            && !forcedToolHint
            && !usedToolThisPrompt
            && tools.length
            && looksLikeFactualClaim(turn.text || '')
          ) {
            forcedToolHint = true;
            emitInner('hold', '这句话里的数字还没查过');
            emit({ type: 'tool_call', toolCallId: `hint-${step}`, title: '需要先查项目数据', status: 'in_progress' });
            emit({ type: 'tool_update', toolCallId: `hint-${step}`, title: '需要先查项目数据', status: 'completed', summary: 'quality guard: force one tool verify pass' });
            session.messages.push({ role: 'user', content: DATA_TOOL_HINT });
            continue;
          }
          // Lightweight verifier before end_turn (at most once).
          if (!forcedVerifier) {
            const verdict = verifyBeforeEnd(session.harness, {
              answerMode: options.answerMode,
              usedToolThisPrompt,
              assistantText: turn.text || '',
              userText: goalText,
              toolNamesThisPrompt,
              toolResultsThisPrompt,
            });
            if (!verdict.ok && tools.length && !forcedVerifier) {
              forcedVerifier = true;
              forcedToolHint = true;
              emitInner('hold', '证据不够，不能把假设写成事实');
              emit({ type: 'tool_call', toolCallId: `verify-${step}`, title: '结束前验收·证据缺口', status: 'in_progress' });
              emit({ type: 'tool_update', toolCallId: `verify-${step}`, title: '结束前验收·证据缺口', status: 'completed', summary: verdict.reason });
              session.messages.push({ role: 'user', content: verdict.hint });
              continue;
            }
          }
          const endVerdict = verifyBeforeEnd(session.harness, {
            answerMode: options.answerMode,
            usedToolThisPrompt,
            assistantText: turn.text || '',
            userText: goalText,
            toolNamesThisPrompt,
            toolResultsThisPrompt,
          });
          const qualified = qualifyUnverifiedEnd(endVerdict, turn.text || '');
          if (qualified.unqualified) {
            emitInner('done', '没有工具证据，这个数字没有当成事实');
            const last = session.messages.at(-1);
            if (last?.role === 'assistant') last.content = qualified.note;
            onText(turn.text ? `\n\n${qualified.note}` : qualified.note);
            return {
              stopReason: 'end_turn',
              replaceText: true,
              assistantText: qualified.note,
              metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed, reasoningEffort: options.reasoningEffort, messageCount: session.messages.length },
              harness: session.harness,
            };
          }
          emitInner('done');
          return {
            stopReason: turn.finishReason === 'length' ? 'max_tokens' : turn.finishReason === 'content_filter' ? 'refusal' : 'end_turn',
            metrics: {
              firstTextMs, firstToolMs, totalMs: performance.now() - started,
              forcedToolHint, forcedEmptyBlock, forcedVerifier,
              api: apiUsed, reasoningEffort: options.reasoningEffort,
              messageCount: session.messages.length,
            },
            harness: session.harness,
          };
        }

        if (firstToolMs === null) firstToolMs = performance.now() - started;
        const calls = turn.toolCalls.map(call => {
          let args = {}, code = '';
          try {
            args = JSON.parse(call.arguments || '{}');
            if (!args || typeof args !== 'object' || Array.isArray(args)) code = 'host_tool_input_invalid';
          } catch { code = 'host_tool_input_invalid'; }
          const name = matchListedTool(call.name, tools);
          if (!tools.some(tool => tool.function?.name === name)) code = 'host_tool_permission_denied';
          if (turn.finishReason === 'length' || turn.finishReason === 'content_filter') code = 'host_provider_output_incomplete';
          return { id: call.id, name: tools.find(tool => tool.function.name === name)?._hostName ?? name, arguments: args, code };
        });
        const appendResults = results => {
          if (!results.length) return;
          toolResultsThisPrompt.push(...results.filter(result => !result.advisory));
          usedToolThisPrompt ||= results.some(result => result.ok && !result.advisory);
          toolNamesThisPrompt.push(...results.filter(result => result.ok && !result.advisory).map(result => result.name));
          session.harness = absorbToolResults(session.harness, results.filter(result => !result.advisory).map(result => ({ ...result, arguments: calls.find(call => call.id === result.id)?.arguments })));
          for (const result of results) {
            emit({ type: 'tool_update', toolCallId: result.id, title: result.name, status: result.ok ? 'completed' : 'failed', summary: result.content.slice(0, 240) });
            session.messages.push({ role: 'tool', tool_call_id: result.id, content: result.content, name: providerToolName(result.name) });
          }
        };
        const invalidResults = calls.filter(call => call.code).map(call => ({ id: call.id, name: call.name, ok: false, code: call.code, content: JSON.stringify({ error: call.code, hint: strategyHint(call.code) }) }));
        appendResults(invalidResults);
        const executable = calls.filter(call => !call.code);
        if (turn.finishReason === 'length' || turn.finishReason === 'content_filter') {
          return { stopReason: turn.finishReason === 'length' ? 'max_tokens' : 'refusal', harness: session.harness, metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed } };
        }
        if (!executable.length && calls.every(call => call.code === 'host_tool_permission_denied')) {
          const offer = typeof this.host?.permissionOffer === 'function'
            ? this.host.permissionOffer(session.conversationId, turn.toolCalls.map(call => call._requestedName ?? call.name))
            : { offers: [] };
          if (offer.offers?.length && (typeof this.host?.requestAuthorization === 'function' || typeof this.host?.waitForAuthorization === 'function')) {
            emitInner('wait', '这一步要先经过你');
            const request = { projectId: offer.projectId, identity: offer.identity, offers: offer.offers, question: goalText };
            let authorization;
            if (typeof this.host.requestAuthorization === 'function') authorization = this.host.requestAuthorization(session.conversationId, request);
            else {
              authorization = this.host.waitForAuthorization(session.conversationId);
              emit({ type: 'authorization.requested', conversationId: session.conversationId, ...request });
            }
            const accepted = await awaitWhileActive(authorization, session.abort.signal);
            if (session.abort?.signal.aborted) throw Object.assign(Error('host_reply_cancelled'), { code: 'host_reply_cancelled' });
            if (accepted) {
              session.messages.push({ role: 'user', content: '用户已授权读取。请立刻调用已授权的只读工具回答刚才的问题，不要输出 DSML 标记。' });
              stepLimit = Math.max(stepLimit, step + 2);
              continue;
            }
          }
          emitInner('done', offer.offers?.length ? '没有授权，所以没有往下做' : '这一步没有可调用的工具');
          const note = offer.offers?.length
            ? '这次没有授权，所以没有执行请求的工具。'
            : '请求的工具当前不可用或未授权，因此没有执行。请检查应用与数据权限配置。';
          onText(turn.text ? `\n\n${note}` : note);
          session.messages.push({ role: 'assistant', content: note });
          if (consumeSteer()) { stepLimit = Math.max(stepLimit, step + 2); continue; }
          return { stopReason: 'end_turn', metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed, reasoningEffort: options.reasoningEffort, messageCount: session.messages.length }, harness: session.harness };
        }
        for (const call of executable) {
          emit({ type: 'tool_call', toolCallId: call.id, title: call.name, status: 'in_progress' });
        }
        emitInner('tool');
        const blocked = executable.filter(call => (failedExecutions.get(executionSignature(call)) ?? 0) >= 2);
        const ready = executable.filter(call => !blocked.includes(call));
        const results = await awaitWhileActive(runToolCalls(ready, call => this.host.mcp(session.token, session.conversationId, 'tools/call', { name: call.name, arguments: call.arguments, _hostToolCallId: call.id }, { runId: session.runId })), session.abort.signal);
        for (const result of results) {
          const key = executionSignature(ready.find(call => call.id === result.id));
          if (result.ok) failedExecutions.delete(key); else failedExecutions.set(key, (failedExecutions.get(key) ?? 0) + 1);
        }
        const suppressed = blocked.map(call => ({ id: call.id, name: call.name, ok: false, code: 'host_tool_repeat_blocked', content: JSON.stringify({ error: 'host_tool_repeat_blocked', hint: 'This exact operation failed twice. It was not executed again. Use a different applicable read or explain the gap.' }) }));
        appendResults([...results, ...suppressed]);
        if (blocked.length && !ready.length) {
          const note = '相同工具和参数已连续失败两次，已停止重复调用。本轮未取得所需结果；请检查工具返回的错误，或改用其他查询条件。';
          session.messages.push({ role: 'assistant', content: note });
          emitInner('done', '已停止无进展的重复调用');
          return { stopReason: 'tool_loop_stalled', replaceText: true, assistantText: note, harness: session.harness, metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed, suppressedCalls: blocked.length } };
        }
        if (session.abort.signal.aborted) throw hostError('host_reply_cancelled');
        if (isDeadEnd(session.harness) && !injectedReview) {
          injectedReview = true;
          emitInner('hold', '同一条路反复失败，先回到你的目标');
          session.messages.push({ role: 'user', content: `${REVIEWER_HINT}\n\n${DEEP_THINK_HINT}` });
          session.harness = noteReviewDone(session.harness, step);
        }
        // Soft compress after tool waves only when session context is growing large.
        if (session.messages.length > 12 || Buffer.byteLength(JSON.stringify(session.messages)) > 48_000) {
          const packed = compressSessionMessages(session.messages);
          session.messages = packed.messages;
          if (packed.compressed) sawCompression = true;
        }
        emitInner('tool');
        if (step + 1 >= stepLimit && consumeSteer()) stepLimit = step + 2;
      }
      emitInner('done', '这一轮步数到了上限');
      return {
        stopReason: 'max_turn_requests',
        metrics: {
          firstTextMs, firstToolMs, totalMs: performance.now() - started,
          forcedToolHint, forcedEmptyBlock, forcedVerifier,
          api: apiUsed, reasoningEffort: options.reasoningEffort,
          messageCount: session.messages.length,
        },
        harness: session.harness,
      };
    } catch (error) {
      if (session.abort.signal.aborted) {
        emitInner('done', '这一轮已停下');
        return {
          stopReason: 'cancelled',
          metrics: { firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed },
          harness: session.harness,
        };
      }
      throw error;
    } finally { session.abort = null; }
  }

  async cancel(sessionId) { const session = this.sessions.get(sessionId); if (!session?.abort) return false; session.abort.abort(); return true; }
  async resolvePermission() { return false; }
  async close() { for (const session of this.sessions.values()) session.abort?.abort(); this.sessions.clear(); this.status.state = 'closed'; }
}
