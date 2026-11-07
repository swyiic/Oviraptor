// xAI / OpenAI-compatible model provider for the agent host.
// Auth: (1) Grok OAuth device-code via public Grok-CLI client — access+refresh stored
// locally; Bearer access_token calls api.x.ai. (2) Manual API key as advanced fallback.
// Secrets file is 0600; settings/events/errors never return tokens or keys.
import { mkdirSync, readFileSync, writeFileSync, rmSync, existsSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { hostError } from './store.mjs';
import { proxyFetch, networkFetch } from './net-proxy.mjs';
import { toolFailureReceipt } from './tool-errors.mjs';
import { normalizeProviderUsage } from './provider-usage.mjs';
import {
  PLANNING_GUIDANCE,
  DATA_TOOL_HINT,
  answerModeOptions,
  buildQualitySystemPrompt,
  looksLikeFactualClaim,
  looksLikeDataQuestion,
  isDataReadTool,
  isReadOnlyDataRequest,
  isProjectReadQuestion,
  isInventoryCountQuestion,
  certificateTaskIntent,
  discoveryEnrichmentIntent,
  discoveryAssistIntent,
  discoverySeedReadIntent,
  discoveryCleanupIntent,
  exposureTaskIntent,
  scanningTaskIntent,
  scanningHandoffIntent,
  scanPreflightBlock,
  scanExcludedCapabilities,
  scanControlFailure,
  normalizeAnswerMode,
  prefersResponsesApi,
  normalizeReasoningEffort,
  scrubSecrets,
  scrubbedModelText,
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
  finishTurnDecision,
  FORCED_CONTINUE_BUDGET,
  toolSetAnnouncement,
} from './agent-state.mjs';
import { admitContext, resolveModelCapability } from './workflow-guard.mjs';
import { compressSessionMessages } from './memory.mjs';
import { exposureTurnTools, unresolvedExposureControlNote } from './exposure-turn.mjs';
import { cleanupTurnState, unresolvedCleanupControlNote } from './discovery-cleanup-turn.mjs';

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
// How many times a single model HTTP turn retries on a TRANSIENT connection/timeout/429/5xx
// error before giving up (only when nothing streamed yet). Smooths "连接不稳定" under the
// high request volume the autonomy changes produce, without masking a real outage.
export const MODEL_TRANSIENT_RETRIES = 2;
// When every tool call a step proposes is an exact twice-failed repeat, give the model this
// many chances to PIVOT to a different approach (with a strong nudge) before declaring the
// turn stalled. Stops the agent quitting the instant it first repeats itself; still bounded.
export const ALL_BLOCKED_PIVOT_BUDGET = 2;
/** Keep provider, baseUrl and model consistent: a DeepSeek model never rides the xAI endpoint. */
export function normalizeProviderModel(input = {}) {
  const out = { ...input };
  const model = String(out.model || '').trim();
  // A catalog connection already names its provider and endpoint. Match the
  // Rust gateway's wire model exactly; legacy preset inference must not route
  // this connection or its credential to a different provider.
  if (connectionReference(out.connectionRef)) {
    out.model = model.replace(/^openai\//, '').trim();
    return out;
  }
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
  const binding = connectionReference(raw.connectionRef);
  const input = normalizeProviderModel(raw);
  const id = input.provider ?? 'xai';
  if (!Object.hasOwn(PROVIDERS, id)) throw hostError('host_provider_invalid');
  const baseUrl = String(input.baseUrl || PROVIDERS[id].baseUrl).replace(/\/+$/, '');
  let url;
  try { url = new URL(baseUrl); } catch { throw hostError('host_provider_base_url_invalid'); }
  if (url.username || url.password || url.search || url.hash) throw hostError('host_provider_base_url_invalid');
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && (binding || loopback(url.hostname)))) throw hostError('host_provider_base_url_invalid');
  const model = String(input.model || PROVIDERS[id].model).trim();
  if (!/^[A-Za-z0-9._:/-]{1,128}$/.test(model)) throw hostError('host_provider_model_invalid');
  let apiKey;
  if (input.apiKey !== undefined) {
    apiKey = String(input.apiKey).trim();
    if (binding ? !apiKey || /[\u0000-\u001f\u007f-\u009f]/.test(apiKey)
      : apiKey.length < 8 || apiKey.length > 512 || /[\s\0]/.test(apiKey)) throw hostError('host_provider_api_key_invalid');
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
      model: connectionReference(candidate.connectionRef) ? String(candidate.model || '').trim()
        : /^(?:xai\/)?grok/i.test(String(candidate.model || '')) ? String(candidate.model).replace(/^xai\//i, '') : PROVIDERS.xai.model,
      authentication: 'oauth', accessToken: String(candidate.accessToken),
      connectionRef: connectionReference(candidate.connectionRef),
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
    // New catalog writes already hold the Rust-compatible wire id. Reapplying
    // prefix normalization would corrupt a real model named openai/custom.
    const normalized = value.modelWireResolved === true && connectionReference(value.connectionRef)
      ? value : normalizeProviderModel(value);
    if (normalized.provider === value.provider && normalized.baseUrl === value.baseUrl) return { ...value, model: normalized.model };
    // Repair a stale mismatched combo (e.g. xai + openai/deepseek-*): use the key saved for that endpoint.
    const slot = keySlot(normalized.provider, normalized.baseUrl);
    const apiKey = raw.keys?.[slot] || this.#envKey(normalized.baseUrl);
    return { ...value, provider: normalized.provider, baseUrl: normalized.baseUrl, model: normalized.model, authentication: apiKey ? 'api-key' : null, apiKey: apiKey || undefined };
  }
  /** Saved OAuth can serve another workload without changing the chat model. */
  readOAuth() { return this.#savedCredentials(this.#raw()).oauth; }
  /** Refresh only the same saved login; a logout/re-login wins over in-flight refresh. */
  refreshOAuth(tokens, expectedAccessToken) {
    const raw = this.#raw();
    const { keys, oauth } = this.#savedCredentials(raw);
    if (!oauth || oauth.accessToken !== expectedAccessToken) throw hostError('host_oauth_changed', 409);
    const next = { ...oauth, accessToken: String(tokens.accessToken),
      ...(tokens.refreshToken ? { refreshToken: String(tokens.refreshToken) } : {}),
      ...(tokens.expiresAt ? { expiresAt: Number(tokens.expiresAt) } : {}),
      ...(tokens.tokenType ? { tokenType: String(tokens.tokenType) } : {}),
      ...(tokens.scope ? { scope: String(tokens.scope) } : {}) };
    if (!tokens.accessToken) throw hostError('host_oauth_token_invalid');
    this.#write(raw.authentication === 'oauth' ? { ...next, keys, oauth: next } : { ...raw, keys, oauth: next });
    return next;
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
      this.#write({ provider: next.provider, baseUrl: next.baseUrl, model: next.model, authentication: 'none', connectionRef, ...(connectionRef ? { modelWireResolved: true } : {}), keys, ...(oauth ? { oauth } : {}) });
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
    const stored = { provider: next.provider, baseUrl: next.baseUrl, model: next.model, apiKey: next.apiKey, authentication: 'api-key', connectionRef, ...(connectionRef ? { modelWireResolved: true } : {}), keys, ...(oauth ? { oauth } : {}) };
    this.#write(stored);
    return this.publicView();
  }
  /** Explicit authentication selection; ordinary model edits keep their auth. */
  selectOAuth(input = {}) {
    const { keys, oauth } = this.#savedCredentials(this.#raw());
    if (!oauth) throw hostError('host_oauth_not_configured');
    const selected = validateProviderConfig({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: input.model || oauth.model, connectionRef: input.connectionRef ?? oauth.connectionRef });
    if (!isXaiOAuthEndpoint(selected.provider, selected.baseUrl)) throw hostError('host_oauth_provider_invalid');
    const next = { ...oauth, model: selected.model,
      connectionRef: input.connectionRef === undefined ? oauth.connectionRef : connectionReference(input.connectionRef) };
    this.#write({ ...next, keys, oauth: next });
    return this.publicView();
  }
  /** Persist OpenCode-compatible OAuth tokens (access + refresh). */
  saveOAuth(tokens, { model, baseUrl, provider = 'xai', select = true } = {}) {
    if (!tokens?.accessToken) throw hostError('host_oauth_token_invalid');
    if (provider !== 'xai' || (baseUrl && !isXaiOAuthEndpoint(provider, baseUrl))) throw hostError('host_oauth_provider_invalid');
    const raw = this.#raw();
    const { keys, oauth } = this.#savedCredentials(raw);
    const selected = !model && oauth?.connectionRef
      ? { provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: oauth.model }
      : validateProviderConfig({ provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: model || oauth?.model || PROVIDERS.xai.model, connectionRef: oauth?.connectionRef });
    if (!isXaiOAuthEndpoint(selected.provider, selected.baseUrl)) throw hostError('host_oauth_provider_invalid');
    const stored = {
      ...selected,
      authentication: 'oauth',
      connectionRef: oauth?.connectionRef ?? null,
      accessToken: String(tokens.accessToken),
      ...(tokens.refreshToken || oauth?.refreshToken ? { refreshToken: String(tokens.refreshToken || oauth.refreshToken) } : {}),
      ...(tokens.expiresAt ? { expiresAt: Number(tokens.expiresAt) } : {}),
      ...(tokens.tokenType ? { tokenType: String(tokens.tokenType) } : {}),
      ...(tokens.scope ? { scope: String(tokens.scope) } : {}),
    };
    this.#write(select || raw?.authentication === 'oauth' ? { ...stored, keys, oauth: stored }
      : { ...(raw ?? { provider: 'xai', baseUrl: PROVIDERS.xai.baseUrl, model: selected.model, authentication: null }), keys, oauth: stored });
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
      oauthSaved: Boolean(this.readOAuth()?.accessToken),
      oauthModel: this.readOAuth()?.model,
      authentication: oauth ? 'oauth' : apiKey ? 'api-key' : value.authentication === 'none' ? 'none' : null,
      authType: oauth ? 'oauth' : apiKey ? 'api-key' : value.authentication === 'none' ? 'none' : null,
      connectionRef: connectionReference(value.connectionRef),
      oauthAvailable: true,
      ...(oauth && value.expiresAt ? { oauthExpiresAt: value.expiresAt } : {}),
    };
  }
}

/** Bound an authorized tool result (head + tail) without rewriting its values.
 * Permission checks and owner-selected field projections happen before this channel. */
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
  host_operation_unknown: '同参数的原操作结果仍未知，本次没有重发。使用错误中的 operationId 或原 taskId 读取原 owner 的持久状态回执；普通最新任务或无关读取不能消除未知。证书启动可用 certificate_task_status(operationId)。',
  host_exposure_task_rejected: '先区分 phase=not_started 和 outcomeUnknown。未知时用原 operationId 调 exposure_task_status 对账，不重发采集，不改写范围或预算。',
  host_exposure_task_outcome_unknown: '采集请求结果未知，不要重发。用错误中的原 operationId 调 exposure_task_status；查不到也不能断言未执行。',
  host_certificate_task_rejected: '依据 reason 区分明确未开始和结果未知。phase=not_started 时先修正范围或参数，再显式继续；未知启动先用原 operationId 调 certificate_task_status 对账，不重发启动。',
  host_certificate_task_outcome_unknown: '启动响应未知，不要重发。用本错误的 operationId 调 certificate_task_status 只读查原持久任务；查不到也不代表未执行。暂停/恢复响应未知时先读原 taskId 状态，不自动重发。',
  host_discovery_task_rejected: '补证执行器拒绝了参数或当前状态，不代表已有权限丢失。读取最近队列时调用 discovery_enrichment_status {}，使用回执中的 run.id 进行暂停/恢复；不要猜测 UUID 或重复未知写操作。范围变化需按新范围重新启动。',
  host_scan_financial_continuation_required: '普通恢复在创建执行尝试前被拒绝：这份任务已有原生模型费用，需要原 owner 的明确剩余额度接续合同。保留原账单、用量和预算；这不是应用读取或执行授权丢失。不要重复恢复、清零预算或用新任务替代该任务的恢复。',
  host_scan_financial_reconciliation_required: '普通恢复在创建执行尝试前被拒绝：原生模型费用尚未确定，需要先通过原 owner 核对。不要重发、退款、清零预算或重新申请已有应用权限。',
  host_scan_financial_history_unavailable: '无法可靠核对原任务的费用历史，普通恢复在创建执行尝试前被拒绝。保留原记录并检查原生诊断；不要重发或用新任务绕过恢复。',

  host_scan_record_unavailable: 'The record is unavailable in this task and visibility policy. Do not guess IDs or switch projects. Read scan_results findings for the same taskId to obtain an actually visible record ID, or report that this specific record cannot be read.',
  host_task_revision_conflict: 'Read host_task_read again and use its current task revision and each existing step revision. Successful data/file tools also advance the task revision. Do not guess or reuse an earlier revision.',
  host_task_criteria_invalid: 'Re-check criteria inputSchema. Each criterion must use exactly the fields for its kind: file_exists {id,kind,artifactId}; file_contains adds contains; tool_json_equals {id,kind,toolName,path,expected,evidenceId?}; tool_read_equals uses arguments? instead of evidenceId; execution_success {id,kind,evidenceId} requires an actual current-run successful workspace_run receipt. path is an array of JSON field keys. Artifact IDs must already exist in host_task_read.',
  host_task_artifact_unverifiable: 'Read host_task_read and correct the artifact location to a workspace-relative path (for example summary.json), or an absolute path inside the agent workspace. Do not use file:// URLs. Then read the file through workspace_read before verifying again.',
  host_task_acceptance_required: 'Read the Host reason and host_task_read. Every completed step needs nonempty evidenceRefs from successful business tools, not host_task_verify or other advisory calls. Keep the step title and dependencies unchanged; set in_progress BEFORE the fresh operation, then complete it with that new receipt. Do not add a circular step requiring verification to verify itself. Declare finite criteria, call host_task_verify, then separately host_task_update with expectedRevision and complete:true.',
  host_task_evidence_stale: 'The step requires evidence produced AFTER its attempt started. Keep its id/title/dependencies unchanged, set in_progress, perform one relevant safe operation, then complete with the new successful host receipt. Do not attach older evidence or repeatedly change the title.',
  host_task_criterion_failed: 'Read the failed criterion IDs in reason and host_task_read.acceptance.checks. Check the actual file or JSON field. Fix a mistaken literal/path to match the real user requirement; do not claim success or blindly repeat verification.',
  host_tool_permission_denied: 'Not authorized: do not retry this tool; tell the user which permission to grant or use another granted tool. Scope: permission missing ≠ data absent.',
  host_tool_not_offered: 'This tool name is not in the current offered tool list; no operation was sent. Select a legitimate current tool for the remaining request or explain this specific capability gap. A tool-selection error does not mean project permissions were lost. Do not repeat a control already sent or with an unknown result.',
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
        let payload = outcome.value;
        // MCP commonly repeats its JSON in content.text and structuredContent.
        // Remove only a proven mirror; preserve other text/media, errors and
        // Host evidence metadata. The durable raw receipt remains unchanged.
        if (payload?.structuredContent && typeof payload.structuredContent === 'object'
          && Array.isArray(payload.content) && payload.content.length === 1
          && payload.content[0]?.type === 'text' && typeof payload.content[0].text === 'string') {
          try {
            if (isDeepStrictEqual(JSON.parse(payload.content[0].text), payload.structuredContent)) {
              const { content: _mirror, ...unique } = payload; payload = unique;
            }
          } catch { /* non-JSON text may contain distinct information */ }
        }
        const cut = truncateToolResult(payload, maxBytes);
        const ok = outcome.value?.isError !== true;
        results.set(call.id, { id: call.id, name: call.name, ok, advisory: outcome.value?._meta?.hostEvidence === 'advisory', content: cut.text, truncated: cut.truncated, ...(!ok ? { code: 'host_tool_result_error' } : {}) });
      } else {
        const receipt = toolFailureReceipt(outcome.reason, outcome.reason?.details?.operationId);
        const code = receipt.error;
        const cut = truncateToolResult({ ...receipt, hint: strategyHint(code) }, maxBytes);
        results.set(call.id, { id: call.id, name: call.name, ok: false, content: cut.text, truncated: cut.truncated, code });
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

function permanentToolErrorCode(result) {
  try { const err = JSON.parse(result?.content ?? '{}')?.error; return ['workspace_path_denied','workspace_fetch_host_denied','host_tool_permission_denied','host_tool_not_offered'].includes(err) ? err : null; } catch { return null; }
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
  const exact = tools.find(tool => String(tool._hostName).replace('::','__') === String(name).replace('::','__'));
  if (exact) return exact.function.name;
  const hits = names.filter(item => item.endsWith(`__${name}`) || item.endsWith(`::${name}`));
  return hits.length === 1 ? hits[0] : name;
}

const REFERENCE_READ_TOOLS = new Set(['workspace_list','workspace_read','workspace_grep','workspace_glob','skills_list','skills_read','skills_list_resources','skills_read_resource']);
const shortToolName = name => String(name ?? '').split(/::|__/).at(-1);
const isProjectReadTool = name => isDataReadTool(name) || REFERENCE_READ_TOOLS.has(shortToolName(name));
// A Host grant uses :: while MCP uses __ and provider names may be hashed.
// These aliases only constrain a turn; they never confer tool authority.
const toolNameAliases = name => {
  const literal = String(name ?? ''), mcp = literal.replace('::','__');
  return new Set([literal,mcp,providerToolName(literal),providerToolName(mcp)]);
};
function matchesToolRestriction(name, restrictions) {
  const aliases = toolNameAliases(name);
  return [...restrictions].some(restricted => [...toolNameAliases(restricted)].some(alias => aliases.has(alias)));
}
function offeredPermissionTools(offer) {
  return (offer?.offers ?? []).flatMap(app => (app.permissions ?? []).flatMap(permission =>
    (permission.tools ?? []).map(name => `${app.appId}::${name}`)));
}
function matchesPermissionTool(name, names) {
  if(matchesToolRestriction(name,new Set(names)))return true;
  // Resolve an unqualified model name only against the exact Host offer. It
  // cannot hide or authorize another app's already listed same-named tool.
  return /^[A-Za-z0-9_-]+$/.test(name) && !String(name).includes('__')
    && names.filter(candidate=>shortToolName(candidate)===name).length===1;
}
function remainingPermissionOffer(offer, unavailable, attempted) {
  return {...offer,offers:(offer?.offers??[]).flatMap(app=>{
    if(!Array.isArray(app.permissions))return[app];
    const permissions=app.permissions.flatMap(permission=>{
      const tools=(permission.tools??[]).filter(name=>!matchesToolRestriction(`${app.appId}::${name}`,unavailable)
        && !matchesToolRestriction(`${app.appId}::${name}`,attempted));
      return tools.length?[{...permission,tools}]:[];
    });
    return permissions.length?[{...app,permissions}]:[];
  })};
}

/** Full refusal text is scrubbed, then clipped. A raw 240-character cut can keep a split secret. */
function refusalExplanation(value) {
  const full = typeof value === 'string' ? value : String(value ?? '');
  return scrubbedModelText(full).slice(0, 240);
}

// Longest bounded match in scrubSecrets. `{8,}` and `{12,}` have no upper bound, so they do not raise this.
// sk-/xai-/Bearer, assignment, auth-header, and secret-pair values are held until their own terminators.
function modelSecretHold() {
  let max = 0;
  for (const match of String(scrubSecrets).matchAll(/\{(\d+)(?:,(\d*))?\}/g)) {
    const bound = match[2] ? Number(match[2]) : Number(match[1]);
    if (bound > max) max = bound;
  }
  return max;
}

const MODEL_SECRET_HOLD = modelSecretHold();

// Bounded matches are ASCII. A trailing code point above ASCII cannot extend them.
function secretTailCanGrow(text) {
  return text.charCodeAt(text.length - 1) <= 0x7f;
}

// scrubSecrets: /\b(https?:\/\/)([^/\s@:]+:[^/\s@]+)@/i → '$1[redacted]@'
// Hold only a suffix that can still reach that `@`. Replacement stays in scrubSecrets.
const URL_USERINFO_USER = /[^/\s@:]/;
const URL_USERINFO_PASS = /[^/\s@]/;
const URL_USERINFO_SCHEMES = ['https://', 'http://'];

function isAsciiWord(code) {
  return (code >= 48 && code <= 57) || (code >= 65 && code <= 90) || (code >= 97 && code <= 122) || code === 95;
}

function wordBoundaryBefore(text, index) {
  if (!isAsciiWord(text.charCodeAt(index))) return false;
  if (index === 0) return true;
  return !isAsciiWord(text.charCodeAt(index - 1));
}

function schemeLengthAt(text, index) {
  const match = /^https?:\/\//i.exec(text.slice(index, index + 8));
  return match ? match[0].length : 0;
}

function openUserinfoReachesEnd(text, start) {
  let seenColon = false;
  let username = 0;
  for (let i = start; i < text.length; i += 1) {
    const ch = text[i];
    if (!seenColon) {
      if (ch === ':') {
        if (username < 1) return false;
        seenColon = true;
        continue;
      }
      if (!URL_USERINFO_USER.test(ch)) return false;
      username += 1;
      continue;
    }
    if (!URL_USERINFO_PASS.test(ch)) return false;
  }
  return true;
}

function trailingSchemePrefixLength(text) {
  const max = Math.min(text.length, URL_USERINFO_SCHEMES[0].length);
  let best = 0;
  for (let len = 1; len <= max; len += 1) {
    const suffix = text.slice(text.length - len);
    const folded = suffix.toLowerCase();
    if (folded.length !== len || !URL_USERINFO_SCHEMES.some(scheme => scheme.startsWith(folded))) continue;
    if (!wordBoundaryBefore(text, text.length - len)) continue;
    best = len;
  }
  return best;
}

function urlUserinfoHoldLength(text) {
  if (!text) return 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code !== 72 && code !== 104) continue;
    if (!wordBoundaryBefore(text, i)) continue;
    const scheme = schemeLengthAt(text, i);
    if (!scheme) continue;
    if (openUserinfoReachesEnd(text, i + scheme)) return text.length - i;
  }
  return trailingSchemePrefixLength(text);
}

// scrubSecrets: /\b(access_token|refresh_token|api[_-]?key|client_secret)\s*[:=]\s*["']?[^\s"',;]{8,}/gi
// → '$1=[redacted]'. Hold only a suffix that can still reach that terminator.
// The value class has no upper bound and includes non-ASCII. Replacement stays in scrubSecrets.
const ASSIGNMENT_KEYS = ['access_token', 'refresh_token', 'apikey', 'api_key', 'api-key', 'client_secret'];

function foldedKeyAt(text, index, key) {
  if (index + key.length > text.length) return false;
  for (let i = 0; i < key.length; i += 1) {
    let code = text.charCodeAt(index + i);
    if (code >= 65 && code <= 90) code += 32;
    if (code !== key.charCodeAt(i)) return false;
  }
  return true;
}

function openAssignmentReachesEnd(text, start) {
  let matched = '';
  for (const key of ASSIGNMENT_KEYS) {
    if (key.length > matched.length && foldedKeyAt(text, start, key)) matched = key;
  }
  let i = start;
  if (!matched) {
    const rest = text.length - start;
    return rest > 0 && ASSIGNMENT_KEYS.some(key => rest < key.length && foldedKeyAt(text, start, key.slice(0, rest)));
  }
  i = start + matched.length;
  while (i < text.length && /\s/.test(text[i])) i += 1;
  if (i >= text.length) return true;
  if (text[i] !== ':' && text[i] !== '=') return false;
  i += 1;
  while (i < text.length && /\s/.test(text[i])) i += 1;
  if (i >= text.length) return true;
  if (text[i] === '"' || text[i] === "'") {
    i += 1;
    if (i >= text.length) return true;
  }
  for (; i < text.length; i += 1) {
    if (/[\s"',;]/.test(text[i])) return false;
  }
  return true;
}

function assignmentHoldLength(text) {
  if (!text) return 0;
  for (let i = 0; i < text.length; i += 1) {
    let code = text.charCodeAt(i);
    if (code >= 65 && code <= 90) code += 32;
    if (code !== 97 && code !== 99 && code !== 114) continue;
    if (!wordBoundaryBefore(text, i)) continue;
    if (!openAssignmentReachesEnd(text, i)) continue;
    return text.length - i;
  }
  return 0;
}

// scrubSecrets → scrubHeaderValues. Needles are cookie:, set-cookie:, authorization:,
// proxy-authorization:. One optional space or tab follows the colon. A quoted value
// runs until the next `"`. An unquoted value runs until `\r`, `\n`, or `"`.
// The value has no maximum and may be non-ASCII. Hold only a suffix that already has
// the colon and has not reached that terminator. Incomplete names stay on the 64-character hold.
const HEADER_NAMES = ['proxy-authorization', 'set-cookie', 'authorization', 'cookie'];

function openHeaderReachesEnd(text, start) {
  let matched = '';
  for (const name of HEADER_NAMES) {
    if (name.length > matched.length && foldedKeyAt(text, start, name)) matched = name;
  }
  if (!matched) return false;
  let i = start + matched.length;
  if (i >= text.length || text[i] !== ':') return false;
  i += 1;
  if (i >= text.length) return true;
  if (text[i] === ' ' || text[i] === '\t') {
    i += 1;
    if (i >= text.length) return true;
  }
  if (text[i] === '"') {
    for (let j = i + 1; j < text.length; j += 1) {
      if (text[j] === '"') return false;
    }
    return true;
  }
  for (; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === '\r' || ch === '\n' || ch === '"') return false;
  }
  return true;
}

function headerHoldLength(text) {
  if (!text) return 0;
  for (let i = 0; i < text.length; i += 1) {
    let code = text.charCodeAt(i);
    if (code >= 65 && code <= 90) code += 32;
    if (code !== 97 && code !== 99 && code !== 112 && code !== 115) continue;
    if (!openHeaderReachesEnd(text, i)) continue;
    return text.length - i;
  }
  return 0;
}

// scrubSecrets → scrubSecretPairs. The value has no maximum and may be non-ASCII.
// Quoted values run until `"`. Other values run until `&`, `;`, newline, space, comma,
// quote, or brace. JSON containers are not pair values. Hold only a suffix whose key
// is already a secret name, whose separator is present, and whose value is still open.
const PAIR_KEY_SUFFIXES = ['apikey', 'apisecret', 'token', 'secret', 'password', 'passwd', 'pwd', 'cookie', 'credential', 'privatekey', 'sessionid', 'session', 'csrf', 'authorization', 'jwt'];

function isPairKeyChar(ch) {
  const code = ch.charCodeAt(0);
  return (code >= 48 && code <= 57) || (code >= 65 && code <= 90) || (code >= 97 && code <= 122) || code === 95 || code === 46 || code === 45;
}

function isPairTerminator(ch) {
  return ch === '&' || ch === ';' || ch === '\n' || ch === '\r' || ch === ' ' || ch === ',' || ch === '"' || ch === "'" || ch === '{' || ch === '}';
}

function isPairSecretName(name) {
  const normalized = name.replace(/[_\-\s.]/g, '').toLowerCase();
  if (!normalized) return false;
  return PAIR_KEY_SUFFIXES.some(suffix => normalized === suffix || normalized.endsWith(suffix));
}

function pairKeyAt(text, sep) {
  let end = sep;
  if (end > 0 && (text[end - 1] === '"' || text[end - 1] === "'")) end -= 1;
  let start = end;
  while (start > 0 && isPairKeyChar(text[start - 1])) start -= 1;
  if (start === end) return null;
  return { start, key: text.slice(start, end) };
}

function pairJsonContainer(text, from) {
  const value = text.slice(from).trimStart();
  const opener = value[0];
  const next = value.slice(1).trimStart()[0];
  return (opener === '{' && (next === '"' || next === '}')) || (opener === '[' && (next === '"' || next === '{' || next === '[' || next === ']'));
}

function openPairHoldLength(text, sep) {
  const found = pairKeyAt(text, sep);
  if (!found || !isPairSecretName(found.key)) return 0;
  if (pairJsonContainer(text, sep + 1)) return 0;
  let at = sep + 1;
  while (at < text.length && (text[at] === ' ' || text[at] === '\t')) at += 1;
  if (at >= text.length) return text.length - found.start;
  if (text[at] === '"') {
    for (let i = at + 1; i < text.length; i += 1) {
      if (text[i] === '"') return 0;
    }
    return text.length - found.start;
  }
  for (let i = at; i < text.length; i += 1) {
    if (isPairTerminator(text[i])) return 0;
  }
  return text.length - found.start;
}

function secretPairHoldLength(text) {
  if (!text) return 0;
  let hold = 0;
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] !== '=' && text[i] !== ':') continue;
    const pairHold = openPairHoldLength(text, i);
    if (!(pairHold > hold)) continue;
    hold = pairHold;
    break;
  }
  return hold;
}

// scrubSecrets: /\b(sk-|xai-)[A-Za-z0-9_-]{8,}\b/g → '[redacted-key]'
// scrubSecrets: /\bBearer\s+[A-Za-z0-9._~+/=-]{12,}/gi → 'Bearer [redacted]'
// Neither value has an upper bound. A word boundary at the end of a chunk is not the
// terminator: the class can still continue, and `-` is inside the sk-/xai- class.
// Hold the open suffix until a character outside that class. Replacement stays in scrubSecrets.
function prefixedKeyChar(code) {
  return (code >= 48 && code <= 57) || (code >= 65 && code <= 90) || (code >= 97 && code <= 122) || code === 95 || code === 45;
}

function bearerValueChar(code) {
  return prefixedKeyChar(code) || code === 46 || code === 126 || code === 43 || code === 47 || code === 61;
}

function prefixedKeyLength(text, start) {
  if (text.startsWith('xai-', start)) return 4;
  if (text.startsWith('sk-', start)) return 3;
  return 0;
}

function openPrefixedKeyReachesEnd(text, start) {
  const prefix = prefixedKeyLength(text, start);
  if (!prefix) return false;
  for (let i = start + prefix; i < text.length; i += 1) {
    if (!prefixedKeyChar(text.charCodeAt(i))) return false;
  }
  return true;
}

function prefixedKeyHoldLength(text) {
  if (!text) return 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code !== 115 && code !== 120) continue;
    if (!wordBoundaryBefore(text, i)) continue;
    if (!openPrefixedKeyReachesEnd(text, i)) continue;
    return text.length - i;
  }
  return 0;
}

function openBearerReachesEnd(text, start) {
  let i = start + 6;
  if (i < text.length && isAsciiWord(text.charCodeAt(i))) return false;
  if (i >= text.length) return true;
  if (!/\s/.test(text[i])) return false;
  while (i < text.length && /\s/.test(text[i])) i += 1;
  for (; i < text.length; i += 1) {
    if (!bearerValueChar(text.charCodeAt(i))) return false;
  }
  return true;
}

function bearerHoldLength(text) {
  if (!text) return 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code !== 66 && code !== 98) continue;
    if (!wordBoundaryBefore(text, i)) continue;
    if (!foldedKeyAt(text, i, 'bearer')) continue;
    if (!openBearerReachesEnd(text, i)) continue;
    return text.length - i;
  }
  return 0;
}

function unboundedValueHoldLength(text) {
  return Math.max(assignmentHoldLength(text), headerHoldLength(text), secretPairHoldLength(text), prefixedKeyHoldLength(text), bearerHoldLength(text));
}

function visibleModelDelta(scrubbed, raw) {
  const text = scrubbed && String(scrubbed).trim()
    ? scrubbed
    : (!String(raw ?? '').trim() ? String(raw ?? '') : '');
  return text;
}

// Prefix that can no longer grow. Bounded ASCII matches still hold MODEL_SECRET_HOLD.
// URL userinfo holds its open suffix, which may be longer and non-ASCII, until `@`.
// sk-/xai- and Bearer values hold their open suffix until a character outside the
// class scrubSecrets already uses. Assignment, auth-header, and secret-pair values
// hold their open suffix until the terminator scrubSecrets already uses. A tail that
// can no longer be one of those is scrubbed.
function splitHeldModelText(raw) {
  if (!raw) return { emit: null, rest: '' };
  let held = '';
  let body = raw;
  while (body) {
    const hold = unboundedValueHoldLength(body);
    if (!(hold > 0)) break;
    if (hold >= body.length) return { emit: null, rest: raw };
    held = `${body.slice(body.length - hold)}${held}`;
    body = body.slice(0, body.length - hold);
  }
  const bounded = splitHeldBoundedText(body);
  if (bounded.emit == null) return { emit: null, rest: raw };
  return { emit: bounded.emit, emitRaw: bounded.emitRaw, rest: `${bounded.rest}${held}` };
}

function splitHeldBoundedText(raw) {
  if (!raw) return { emit: null, rest: '' };
  const asciiHold = MODEL_SECRET_HOLD > 0 && secretTailCanGrow(raw) ? MODEL_SECRET_HOLD : 0;
  const hold = Math.max(asciiHold, urlUserinfoHoldLength(raw));
  if (!(hold > 0)) {
    return { emit: raw.trim() ? scrubbedModelText(raw) : '', emitRaw: raw, rest: '' };
  }
  if (raw.length <= hold) return { emit: null, rest: raw };
  if (!raw.trim()) return { emit: '', emitRaw: raw.slice(0, -hold), rest: raw.slice(-hold) };
  const scrubbed = scrubbedModelText(raw);
  if (!scrubbed) return { emit: '', emitRaw: raw, rest: '' };
  if (scrubbed === raw) return { emit: raw.slice(0, -hold), emitRaw: raw.slice(0, -hold), rest: raw.slice(-hold) };
  let same = 0;
  const cap = Math.min(hold, raw.length, scrubbed.length);
  while (same < cap && raw.charCodeAt(raw.length - 1 - same) === scrubbed.charCodeAt(scrubbed.length - 1 - same)) same += 1;
  if (same >= raw.length) return { emit: scrubbed, emitRaw: raw, rest: '' };
  return {
    emit: scrubbed.slice(0, scrubbed.length - same),
    emitRaw: raw.slice(0, raw.length - same),
    rest: raw.slice(raw.length - same),
  };
}

/** Parse an OpenAI-compatible chat.completions SSE stream into text deltas and assembled tool calls. */
export async function readChatStream(body, onText, onUsage = () => {}) {
  const decoder = new TextDecoder(); let buffer = '', raw = '', shown = '', finish = null, reasoning = '';
  const tools = [];
  let refusal = null;
  let usage = null;
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
    const observed = normalizeProviderUsage(chunk.response?.usage ?? chunk.usage);
    if (observed) { usage = observed; onUsage(observed); }
    if (chunk.error) throw hostError('host_provider_stream_failed', 502, { outcomeUnknown: true });
    const choice = chunk.choices?.[0]; if (!choice) return;
    const delta = choice.delta ?? {};
    if (delta.refusal) {
      const explanation = typeof delta.refusal === 'string' ? delta.refusal : String(delta.refusal.explanation || delta.refusal.reason || '');
      refusal = { code: 'model_refusal', category: typeof delta.refusal === 'object' && delta.refusal.category ? String(delta.refusal.category).slice(0, 80) : 'model_refusal', explanation: refusalExplanation(explanation) };
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
    return { text: refusal?.explanation || parsed.text, toolCalls: [], ...(usage ? { usage } : {}), finishReason: 'refusal', refusal: refusal ?? { code: 'model_refusal', category: 'model_refusal', explanation: '' } };
  }
  const toolCalls = [...tools.filter(Boolean), ...parsed.calls];
  if (finish === 'tool_calls' && !toolCalls.length) throw hostError('host_provider_tool_calls_invalid', 502, { outcomeUnknown: true });
  return { text: parsed.text, toolCalls, ...(usage ? { usage } : {}), finishReason: finish, ...(reasoning ? { reasoningContent: reasoning } : {}), ...(finish === 'content_filter' ? { refusal: { code: 'model_refusal', category: 'content_filter', explanation: '' } } : {}) };
}

/**
 * Parse xAI / OpenAI Responses API SSE stream.
 * Emits user-visible text via onText; keeps reasoning encrypted items for next-turn input.
 */
export async function readResponsesStream(body, onText, onUsage = () => {}) {
  const decoder = new TextDecoder(); let buffer = '';
  let text = '';
  const tools = [];
  const toolByIndex = new Map();
  const outputItems = [];
  let responseId = null;
  let finish = null;
  let refusal = null;
  let usage = null;
  const handleEvent = (eventType, data) => {
    if (data === '[DONE]') return;
    let chunk; try { chunk = JSON.parse(data); } catch { return; }
    const observed = normalizeProviderUsage(chunk.response?.usage ?? chunk.usage);
    if (observed) { usage = observed; onUsage(observed); }
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
        refusal = { code: 'model_refusal', category: 'model_refusal', explanation: refusalExplanation(item.refusal || item.content || '') };
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
              refusal = { code: 'model_refusal', category: 'model_refusal', explanation: refusalExplanation(part.refusal || part.text || '') };
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
    return { text: refusal?.explanation || text, toolCalls: [], ...(usage ? { usage } : {}), finishReason: 'refusal', refusal: refusal ?? { code: 'model_refusal', category: 'model_refusal', explanation: '' }, responseId, outputItems };
  }
  if (!tools.length && toolByIndex.size) {
    for (const [, value] of [...toolByIndex.entries()].sort((a, b) => a[0] - b[0])) tools.push(value);
  }
  if (tools.some(tool => !tool.name)) throw hostError('host_provider_tool_calls_invalid', 502, { outcomeUnknown: true });
  return { text, toolCalls: tools, ...(usage ? { usage } : {}), finishReason: finish, responseId, outputItems };
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
 * Ids and type fields stay. Other strings use scrubbedModelText.
 * An empty scrub is omitted. An item left with only that dropped secret is not replayed.
 */
function scrubReasoningNode(value, key) {
  if (value == null || typeof value === 'number' || typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    if (key === 'id' || key === 'type') return value;
    const scrubbed = scrubbedModelText(value);
    return scrubbed ? scrubbed : undefined;
  }
  if (Array.isArray(value)) {
    const next = [];
    for (const entry of value) {
      const scrubbed = scrubReasoningNode(entry);
      if (scrubbed !== undefined) next.push(scrubbed);
    }
    return next.length || !value.length ? next : undefined;
  }
  if (typeof value === 'object') {
    const next = {};
    for (const [childKey, child] of Object.entries(value)) {
      const scrubbed = scrubReasoningNode(child, childKey);
      if (scrubbed === undefined) continue;
      next[childKey] = scrubbed;
    }
    const structural = Object.keys(next).every(childKey => childKey === 'id' || childKey === 'type');
    if (structural && Object.keys(value).some(childKey => childKey !== 'id' && childKey !== 'type')) return undefined;
    return next;
  }
  return undefined;
}

function scrubReasoningItemsForModel(items) {
  const next = [];
  for (const item of items ?? []) {
    if (item?.type !== 'reasoning') continue;
    const scrubbed = scrubReasoningNode(item);
    if (!scrubbed || scrubbed.type !== 'reasoning') continue;
    next.push(scrubbed);
  }
  return next;
}

/** Scrub assistant prose/reasoning only; replay tool-call arguments verbatim. */
function scrubAssistantMessagesForModel(messages) {
  for (const message of messages) {
    if (message?.role !== 'assistant') continue;
    if (typeof message.content === 'string') {
      const scrubbed = scrubbedModelText(message.content);
      message.content = scrubbed || null;
    }
    if (typeof message.reasoning_content === 'string') {
      const scrubbed = scrubbedModelText(message.reasoning_content);
      if (scrubbed) message.reasoning_content = scrubbed;
      else delete message.reasoning_content;
    }
  }
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
      const raw = String(message.content ?? '');
      // User rows were scrubbed when saved. Scrub only assistant text; an empty scrub is not replayed.
      const content = message.role === 'assistant' ? scrubbedModelText(raw).trim() : raw.trim();
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
    scrubAssistantMessagesForModel(session.messages);
    const options = session.options;
    const config = session.config;
    // DeepSeek thinking mode (enabled below only for quality turns) rejects any
    // request whose assistant history has a turn without reasoning_content — and
    // host-injected notes, or turns the scrub emptied, legitimately lack it. When
    // thinking is on, backfill a minimal placeholder on those so the "must be
    // passed back" contract holds; turns that still carry real reasoning keep it
    // verbatim, and non-thinking requests are untouched.
    if ((isDeepSeekModel(config.model) || isDeepSeekBase(config.baseUrl)) && options.answerMode === 'quality') {
      for (const message of session.messages) {
        if (message.role === 'assistant'
          && !(typeof message.reasoning_content === 'string' && message.reasoning_content.length > 0)) {
          message.reasoning_content = ' ';
        }
      }
    }
    const body = {
      model: config.model,
      stream: true,
      stream_options: { include_usage: true },
      temperature: options.temperature,
      max_tokens: options.maxTokens,
      messages: session.messages.map(({ _hostUserText, _hostAuthorizationContext, ...message }) => message),
      ...(tools.length ? { tools: tools.map(({ _hostName, ...tool }) => tool), parallel_tool_calls: !session.serialToolCalls } : {}),
      ...(isDeepSeekModel(config.model) || isDeepSeekBase(config.baseUrl) ? {
        thinking: { type: options.answerMode === 'quality' ? 'enabled' : 'disabled' },
        ...(toolChoice ? { tool_choice: options.answerMode === 'quality' ? 'auto' : toolChoice } : {}),
      } : (toolChoice ? { tool_choice: toolChoice } : {})),
    };
    const requestId = this.#admitHttpRequest(session, 'chat');
    const response = await networkFetch(this.fetch, `${config.baseUrl}/chat/completions`, {
      method: 'POST', redirect: 'error',
      headers: { ...(config.authentication === 'none' ? {} : { Authorization: `Bearer ${config.apiKey}` }), 'Content-Type': 'application/json', Accept: 'text/event-stream' },
      body: JSON.stringify(body),
      signal: AbortSignal.any([session.abort.signal, AbortSignal.timeout(options.timeoutMs)]),
    });
    if (!response.ok || !response.body) throw providerHttpError(response.status);
    return { api: 'chat', turn: await readChatStream(response.body, onText, usage => this.host?.recordModelUsage?.(session.conversationId, requestId, usage)) };
  }
  async #responsesTurn(session, tools, { toolChoice, onText } = {}) {
    scrubAssistantMessagesForModel(session.messages);
    const options = session.options;
    const config = session.config;
    const effort = normalizeReasoningEffort(options.reasoningEffort, options.answerMode);
    const reasoningItems = scrubReasoningItemsForModel(session.reasoningItems);
    session.reasoningItems = reasoningItems;
    const input = [
      ...chatMessagesToResponsesInput(session.messages),
      ...reasoningItems,
    ];
    const body = {
      model: config.model,
      stream: true,
      store: false,
      max_output_tokens: options.maxTokens,
      reasoning: { effort },
      input,
      ...(tools.length ? { tools: toResponsesTools(tools), parallel_tool_calls: !session.serialToolCalls } : {}),
      ...(toolChoice ? { tool_choice: toolChoice } : {}),
      prompt_cache_key: session.conversationId || session.sessionId || 'oviraptor-agent',
    };
    const requestId = this.#admitHttpRequest(session, 'responses');
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
    const turn = await readResponsesStream(response.body, onText, usage => this.host?.recordModelUsage?.(session.conversationId, requestId, usage));
    session.previousResponseId = turn.responseId ?? session.previousResponseId;
    session.reasoningItems = (turn.outputItems ?? []).filter(item => item?.type === 'reasoning');
    return { api: 'responses', turn };
  }
  #admitHttpRequest(session, api) {
    if (session.abort.signal.aborted) throw hostError('host_reply_cancelled');
    if (session.conversationId) this.host?.admitModelRequest?.(session.conversationId);
    session.httpAttempts = (session.httpAttempts ?? 0) + 1;
    const requestId = randomUUID();
    if (session.conversationId) this.host?.beginModelUsage?.(session.conversationId, { requestId, runId: session.runId, api, model: session.config.model });
    return requestId;
  }
  /**
   * One model HTTP turn. On 401 (xAI OAuth) refresh once and retry. On a TRANSIENT
   * connection/timeout/429/5xx error — the cause of "连接不稳定" when the agent now makes
   * many sequential requests — retry with bounded backoff, but ONLY when nothing streamed
   * yet (no double-billing a partial answer) and the user did not cancel. A blip no longer
   * drops the whole task.
   */
  async #modelTurn(session, tools, { toolChoice, onText } = {}) {
    const attempt = async () => (session.options.useResponsesApi
      ? await this.#responsesTurn(session, tools, { toolChoice, onText })
      : await this.#chatTurn(session, tools, { toolChoice, onText }));
    const isTransient = (error) => {
      if (!error) return false;
      const name = error.name || '';
      // A timeout from AbortSignal.timeout counts; a user cancel (session.abort) does not.
      if ((name === 'AbortError' || name === 'TimeoutError') && !session.abort?.signal?.aborted) return true;
      const code = String(error.code || error.cause?.code || '');
      if (['ECONNRESET', 'ETIMEDOUT', 'ECONNREFUSED', 'EPIPE', 'ENETUNREACH', 'ENETDOWN', 'EAI_AGAIN', 'UND_ERR_CONNECT_TIMEOUT', 'UND_ERR_SOCKET', 'UND_ERR_HEADERS_TIMEOUT', 'UND_ERR_BODY_TIMEOUT'].includes(code)) return true;
      const msg = `${error.message || ''} ${error.cause?.message || ''}`;
      if (/fetch failed|socket hang up|network error|timed? ?out|terminated|other side closed|connection (?:reset|closed|refused|aborted)|read ECONNRESET/i.test(msg)) return true;
      const status = Number(error.httpStatus ?? error.status ?? error.statusCode ?? 0);
      return [429, 500, 502, 503, 504].includes(status);
    };
    let transientRetries = 0;
    for (;;) {
      try {
        return await attempt();
      } catch (error) {
        // 401 (xAI OAuth): refresh once and retry, only if nothing streamed yet.
        const unauthorized = error?.code === 'host_provider_unauthorized';
        if (unauthorized && !session._authRetried && this.host?.ensureOAuthAccess
          && session.config.authentication === 'oauth' && isXaiOAuthEndpoint(session.config.provider, session.config.baseUrl)
          && !session._producedText) {
          session._authRetried = true;
          try {
            const refreshed = await this.host.ensureOAuthAccess({ force: true });
            const selected = this.host.providerStore?.read?.() ?? refreshed;
            if (selected?.authentication !== 'oauth' || !isXaiOAuthEndpoint(selected.provider, selected.baseUrl) || !selected.accessToken) throw error;
            session.config.apiKey = selected.accessToken;
            if (this.config.authentication === 'oauth' && isXaiOAuthEndpoint(this.config.provider, this.config.baseUrl)) this.config.apiKey = selected.accessToken;
          } catch { throw error; }
          continue;
        }
        // Transient connection/timeout/429/5xx: bounded backoff retry (no partial stream, not cancelled).
        if (!session._producedText && transientRetries < MODEL_TRANSIENT_RETRIES && !session.abort?.signal?.aborted && isTransient(error)) {
          transientRetries += 1;
          const backoff = Math.min(4000, 500 * Math.pow(3, transientRetries - 1)) + Math.floor(Math.random() * 250);
          try { this.onEvent?.({ type: 'model.transient_retry', attempt: transientRetries, max: MODEL_TRANSIENT_RETRIES, reason: String(error?.message || error?.code || 'transient').slice(0, 120) }); } catch { /* best effort */ }
          await new Promise(resolve => setTimeout(resolve, backoff));
          if (session.abort?.signal?.aborted) throw hostError('host_reply_cancelled');
          continue;
        }
        throw error;
      }
    }
  }

  async prompt({ sessionId, prompt, userText: rawUserText, taskContext, turnId, answerMode, authorizationContext } = {}) {
    const session = this.sessions.get(sessionId); if (!session) throw hostError('host_runtime_session_invalid');
    if (session.abort) throw hostError('host_reply_already_running', 409);
    // Permission decisions and tool-selection corrections are turn-local Host
    // context. A later user request must be checked against fresh grants.
    session.messages=session.messages.filter(message=>!message._hostAuthorizationContext);
    this.#applyMode(answerMode);
    // Per-turn settings and callbacks must not be overwritten by another session
    // or by an operator changing the default while this request is streaming.
    const options = session.options = Object.fromEntries(['answerMode', 'temperature', 'maxTokens', 'toolHintGuard', 'forceToolOnDataQuestion', 'reasoningEffort', 'useResponsesApi', 'timeoutMs', 'maxSteps'].map(key => [key, this[key]]));
    session.config = { ...this.config };
    session.runId = turnId;
    const started = performance.now(); let firstTextMs = null, firstToolMs = null;
    const toolSelection=[];
    session.abort = new AbortController();
    session._authRetried = false;
    session._producedText = false;
    session._allBlockedPivots = 0;
    session.httpAttempts = 0;
    const fullPrompt = typeof prompt === 'string' ? prompt : JSON.stringify(prompt);
    // Goal must be the user's raw question — never the whole turn prompt blob.
    // The model copy of that question is scrubbed once, below; the envelope is not scrubbed again.
    const hasExplicitTurn = typeof rawUserText === 'string';
    const suppliedTurn = hasExplicitTurn && rawUserText.trim() ? rawUserText : '';
    let modelPrompt = fullPrompt;
    let loggedTurn = '';
    if (suppliedTurn) {
      const scrubbedTurn = scrubbedModelText(suppliedTurn);
      loggedTurn = scrubbedTurn.trim();
      const at = fullPrompt.lastIndexOf(suppliedTurn);
      if (at >= 0) {
        modelPrompt = `${fullPrompt.slice(0, at)}${loggedTurn ? scrubbedTurn : ''}${fullPrompt.slice(at + suppliedTurn.length)}`;
        if (!loggedTurn) modelPrompt = modelPrompt.replace(/\n+$/, '');
      }
    } else if (!hasExplicitTurn) {
      modelPrompt = scrubbedModelText(fullPrompt);
      loggedTurn = modelPrompt.trim() ? modelPrompt : '';
    }
    let goalText = suppliedTurn ? suppliedTurn.trim() : fullPrompt;
    const unavailableThisTurn = new Set((Array.isArray(authorizationContext?.unavailableTools) ? authorizationContext.unavailableTools : [])
      .filter(name => typeof name === 'string' && name.length > 0 && name.length <= 256).slice(0,128));
    const authorizationAttempted = new Set();
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
    const userContent = harnessBlock ? `${harnessBlock}\n\n${modelPrompt}` : modelPrompt;
    // Internal provenance is consumed by compaction and removed from API input.
    session.messages.push({ role: 'user', content: userContent, _hostUserText: suppliedTurn || !hasExplicitTurn ? loggedTurn : goalText });
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
    if (unavailableThisTurn.size) session.messages.push({ role:'user', _hostAuthorizationContext:true, content:`Host: 本轮权限核对后仍不可用的具体工具：${JSON.stringify([...unavailableThisTurn])}。${authorizationContext?.authorizationDeclined ? '用户未批准这些新权限，本轮不得再次询问或调用。' : '授权核对尚未使这些工具可用，本轮不得重复询问。'}这不影响当前 tools/list 中其余已获准的读取与操作。先完成可做的部分，再准确说明这些工具对应的缺口；不能声称全项目无权限，也不能冒充执行。` });
    const emit = event => { try { this.onEvent({ sessionId, turnId, ...event }); } catch { /* observer isolated */ } };
    let holdNote = '';
    let innerSeq = 0;
    const emitInner = (phase, hold = '') => {
      if (phase === 'model') holdNote = hold;
      else if (hold) holdNote = hold;
      // seq + at let the UI order 'thinking' snapshots precisely between tool steps.
      emit({ type: 'inner', seq: ++innerSeq, at: Date.now(), ...publicInnerState(session.harness, { phase, hold: phase === 'model' ? hold : (hold || holdNote), compressed: sawCompression }) });
    };
    if (sawCompression && this.host?.recordSummary && session.conversationId) {
      const sourceIds = session.messages.flatMap(message => [message.tool_call_id, message._hostUserText].filter(Boolean)).slice(0, 32);
      try { this.host.recordSummary(session.projectId, session.conversationId, { sourceIds, body: session.messages.find(message => String(message.content || '').startsWith('上下文已压缩'))?.content || '' }); } catch { /* summary is advisory */ }
    }
    const contextFit = admitContext({ capability: resolveModelCapability(this.config), messages: session.messages, configuredInputBytes: this.config.inputByteCeiling });
    const contextBlocks = (this.config.contextMode === 'strict' && !contextFit.ok) || (Number.isSafeInteger(this.config.contextWindowTokens) && !contextFit.ok);
    if (contextBlocks) {
      const note = contextFit.reason === 'context_insufficient' ? '上下文装不下硬约束，这一轮没有发给模型。' : '模型窗口未知，严格模式下这一轮没有发给模型。';
      emitInner('done', note);
      session.abort = null;
      return { stopReason: contextFit.reason === 'context_insufficient' ? 'context_insufficient' : 'unsupported_capability', replaceText: true, assistantText: note, metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, api: options.useResponsesApi ? 'responses' : 'chat', messageCount: session.messages.length }, harness: session.harness };
    }
    emitInner(sawCompression ? 'compress' : 'model');
    // A secret split across text_delta chunks is not visible to a per-chunk scrub,
    // and a later content-policy stop does not rewrite deltas already sent.
    // Hold only the tail that can still grow into a scrubSecrets match.
    let pending = '';
    let pendingPieces = [];
    const markText = () => {
      if (firstTextMs === null) firstTextMs = performance.now() - started;
      session._producedText = true;
    };
    const emitVisible = (scrubbed, raw) => {
      const text = visibleModelDelta(scrubbed, raw);
      if (!text) return;
      emit({ type: 'text_delta', text });
    };
    const flushPending = () => {
      if (!pending) return;
      const raw = pending;
      const pieces = pendingPieces;
      pending = '';
      pendingPieces = [];
      if (!raw.trim()) {
        emitVisible('', raw);
        return;
      }
      const scrubbed = scrubbedModelText(raw);
      if (!scrubbed) return;
      if (scrubbed === raw && pieces.join('') === raw) {
        for (const piece of pieces) emitVisible(scrubbedModelText(piece), piece);
        return;
      }
      emitVisible(scrubbed, raw);
    };
    const releasePending = () => {
      const split = splitHeldModelText(pending);
      if (split.emit == null) return;
      emitVisible(split.emit, split.emitRaw);
      pending = split.rest;
      pendingPieces = pending ? [pending] : [];
    };
    const onStreamText = delta => {
      markText();
      const raw = typeof delta === 'string' ? delta : '';
      if (!raw) return;
      pending += raw;
      pendingPieces.push(raw);
      releasePending();
    };
    const onText = delta => {
      flushPending();
      const raw = typeof delta === 'string' ? delta : '';
      if (!raw) return;
      markText();
      emitVisible(scrubbedModelText(raw), raw);
    };
    const consumeSteer = () => {
      const lines = this.host?.takeSteer?.(session.conversationId) ?? [];
      let kept = 0;
      for (const line of lines) {
        // Scrub only this interruption. The current user message was already scrubbed once.
        const scrubbed = scrubbedModelText(line);
        if (!scrubbed) continue;
        session.messages.push({ role: 'user', content: `用户插话：${scrubbed}`, _hostUserText: scrubbed });
        goalText += `\n用户插话：${scrubbed}`;
        kept += 1;
      }
      if (kept) session.harness = setGoal(session.harness, goalText);
      return kept > 0;
    };
    let usedToolThisPrompt = false;
    let toolNamesThisPrompt = [];
    let toolResultsThisPrompt = [];
    const failedExecutions = new Map();
    let forcedToolHint = false;
    let forcedEmptyBlock = false;
    let lengthContinued = false;
    let forcedVerifier = false;
    let injectedReview = false;
    let countAnswerHint = false;
    let certificateStatusHint = false;
    let scanStatusHint = false, scanBlockedHint = false;
    let scanResultsHint = false;
    let handoffFinishedHint = false;
    let readOnlyHint = false;
    let assistFinishedHint = false;
    let assistIntentHint = false;
    let toolSelectionCorrection = false;
    let partialReadHint = false;
    const exposureToolHints = new Set();
    let cleanupHint = false;
    let apiUsed = options.useResponsesApi ? 'responses' : 'chat';
    let stepLimit = options.maxSteps;
    // Pi-inspired loop mechanics (grant no authority; see agent-state.mjs):
    // finishTurn force-continue budget, and the tool-set the model was last told.
    let forcedContinues = 0;
    let forcedContinueReason = null;
    let toolSetAnnouncements = 0;
    let announcedToolNames = null;
    try {
      for (let step = 0; step < stepLimit; step++) {
        if (session.abort.signal.aborted) throw hostError('host_reply_cancelled');
        consumeSteer();
        emitInner('model');
        const grantedTools = await this.#tools(session);
        // App capabilities are freshly permission-filtered by Host. A real
        // policy/grant change during this turn supersedes an earlier gap. Core
        // workspace schemas are always advertised and are not grant witnesses.
        const restoredAppTools=grantedTools.filter(tool=>/::|__/.test(tool._hostName)
          && matchesToolRestriction(tool._hostName,unavailableThisTurn));
        if(restoredAppTools.length){
          for(const name of [...unavailableThisTurn])if(restoredAppTools.some(tool=>matchesToolRestriction(tool._hostName,new Set([name]))))unavailableThisTurn.delete(name);
          session.messages=session.messages.filter(message=>!message._hostAuthorizationContext);
          session.messages.push({role:'user',_hostAuthorizationContext:true,content:`Host: 最新 tools/list 已实际提供这些业务工具：${JSON.stringify(restoredAppTools.map(tool=>tool._hostName))}。原先相应权限缺口已由 Host 核验更新，不要再把旧拒绝当作当前限制；只用当前 schema，实际执行仍由 Host 和业务 owner 复核，不重发已发送或结果未知的控制。${unavailableThisTurn.size?`仍不可用且本轮不重复询问：${JSON.stringify([...unavailableThisTurn])}。`:''}`});
        }
        // Task 2: announce a mid-conversation tool-set change so the model stops
        // falsely concluding it has no execution tools after a grant unlocks them.
        // Diff the raw granted list (real authority), not the turn-narrowed list.
        const currentToolNames = grantedTools.map(tool => tool._hostName);
        if (announcedToolNames !== null) {
          const announcement = toolSetAnnouncement(announcedToolNames, currentToolNames, { exclude: restoredAppTools.map(tool => tool._hostName) });
          if (announcement) {
            session.messages.push({ role: 'user', _hostAuthorizationContext: true, content: announcement.message });
            toolSetAnnouncements += 1;
            emitInner('model', '工具集有变化，已告知模型');
          }
        }
        announcedToolNames = currentToolNames;
        let tools = grantedTools.filter(tool => !matchesToolRestriction(tool._hostName, unavailableThisTurn));
        const readOnlyTurn=isReadOnlyDataRequest(goalText);
        if(readOnlyTurn){
          tools=tools.filter(tool=>isProjectReadTool(tool._hostName));
          if(!readOnlyHint){readOnlyHint=true;session.messages.push({role:'user',content:'Host: 本轮是只读数据核对。直接查询本次请求涉及模块的已保存数据并回答，不创建或读写持久计划、验收条件、子任务，不执行扫描控制。记录数不是确认漏洞数，未读完分页请明确说明。'});}
          if(toolResultsThisPrompt.filter(result=>result.ok&&!result.advisory&&isDataReadTool(result.name)&&!(discoverySeedReadIntent(goalText)&&['discovery_seed_list','discovery_seed_detail'].includes(String(result.name).split(/::|__/).at(-1)))).length>=8){tools=[];session.messages.push({role:'user',content:'Host: 本轮只读调用达到上限。请直接依据已返回的 summary 和记录回答，说明未读游标，不再查询或创建验收。'});}
        }
        const listedTools = tools;
        const dataReads = tools.filter(tool => isDataReadTool(tool._hostName));
        const statsReads = dataReads.filter(tool => String(tool._hostName).split(/::|__/).at(-1) === 'inventory_stats');
        const referenceReadRequested = /文件|报告|文档|工作区|工作空间|\b(?:workspace|readme|skill|files?|reports?|documents?)\b|\.(?:md|txt|json|csv)\b/i.test(goalText);
        const inventoryCountOnly = isInventoryCountQuestion(goalText) && !referenceReadRequested;
        const simpleCount = inventoryCountOnly && statsReads.length > 0;
        const countReady = simpleCount && toolResultsThisPrompt.some(result => result.ok && !result.advisory
          && String(result.name).split(/::|__/).at(-1) === 'inventory_stats'
          && !/\[truncated:/i.test(result.content));
        const hasDataEvidence = toolResultsThisPrompt.some(result => result.ok && !result.advisory && isDataReadTool(result.name));
        const wantForceTool = options.forceToolOnDataQuestion && !hasDataEvidence && dataReads.length > 0 && looksLikeDataQuestion(goalText);
        // Requiring evidence must not hide granted mutations from an explicit
        // read-create-read request. A model may otherwise conclude at the first
        // inventory read that creation is unauthorized and never rediscover it.
        // Actual read-only turns are filtered above; owner checks still govern calls.
        if (isProjectReadQuestion(goalText)) tools = tools.filter(tool=>isProjectReadTool(tool._hostName));
        // A workspace file cannot establish the live inventory total. Explicit
        // file/report reads keep their reference tools even if they ask a count.
        if (inventoryCountOnly && !statsReads.length) tools=dataReads;
        if (simpleCount && toolResultsThisPrompt.length === 0) tools = statsReads;
        if (countReady) {
          tools = [];
          if (!countAnswerHint) {
            countAnswerHint = true;
            session.messages.push({ role: 'user', content: 'Host: 本轮 inventory_stats 已成功返回项目统计。请直接根据该结果回答当前计数问题，不再创建计划、设置验收条件或重复查询。只报告结果实际包含的字段；缺少的字段明确说未提供。' });
          }
        }
        const cleanupIntent=discoveryCleanupIntent(goalText);
        const cleanupTurn=Boolean(cleanupIntent&&listedTools.some(tool=>String(tool._hostName).split(/::|__/).at(-1)?.startsWith('discovery_cleanup_')));
        let cleanupState;
        if(cleanupTurn){
          cleanupState=cleanupTurnState(cleanupIntent,listedTools,toolResultsThisPrompt);tools=cleanupState.tools;
          if(!cleanupHint){cleanupHint=true;session.messages.push({role:'user',content:'Host: 本轮处理用户明确请求的项目种子清洗，后台持久任务会全量读取并继续补证。只提交一次明确控制，然后读取一次状态回答；不要换成旧 AI 范围发现、逐条人工确认、物理删除或 Hunter。暂停/恢复/撤销必须先从本项目状态读取真实 runId，结果未知不重发。'});}
        }
        const assistIntent=discoveryAssistIntent(goalText);
        const assistTurn=Boolean(assistIntent&&!(readOnlyTurn&&discoverySeedReadIntent(goalText))&&listedTools.some(tool=>String(tool._hostName).split(/::|__/).at(-1)?.startsWith('discovery_assist_')));
        if(assistTurn){
          const short=name=>String(name).split(/::|__/).at(-1);
          const action=readOnlyTurn?'status':assistIntent.action;
          const control='discovery_assist_'+action;
          const last=toolResultsThisPrompt.findLastIndex(result=>short(result.name)===control);
          const checked=toolResultsThisPrompt.some((result,index)=>index>last&&short(result.name)==='discovery_assist_status');
          const allowed=action==='status'?(last>=0?[]:['discovery_assist_status']):(last<0?[control]:checked?[]:['discovery_assist_status']);
          tools=listedTools.filter(tool=>allowed.includes(short(tool._hostName)));
          if(!assistIntentHint&&tools.length){
            assistIntentHint=true;
            session.messages.push({role:'user',content:`Host: 只处理本次用户明确要求的全项目 AI 发现动作 ${action}，历史任务目标不是本轮限制或授权。本轮第一步仅可调用当前 tools 中的 ${tools[0].function.name}，使用本次明确目标及参数；不得猜旧工具名、创建计划或自行确认范围。收到控制回执后只核对一次当前状态，未知控制不重放。`});
          }
          if(!allowed.length&&!assistFinishedHint){
            assistFinishedHint=true;
            session.messages.push({role:'user',content:'Host: 本次全项目 AI 发现控制和一次状态读取已结束。直接根据真实回执报告状态或错误；后台会继续运行，完整范围建议由聊天面板自动更新并由用户一次确认。不要循环轮询、逐条审阅、重复控制，不调用 Hunter、不自行确认范围，也不把受理视为分析完成。'});
          }
        }
        const enrichmentIntent=discoveryEnrichmentIntent(goalText);
        const enrichmentTurn=Boolean(enrichmentIntent&&listedTools.some(tool=>String(tool._hostName).split(/::|__/).at(-1)?.startsWith('discovery_enrichment_')));
        let enrichmentBinding;
        if(enrichmentTurn){
          const short=name=>String(name).split(/::|__/).at(-1);
          const relevant=toolResultsThisPrompt.filter(result=>short(result.name)?.startsWith('discovery_enrichment_'));
          const attempted=relevant.some(result=>/^discovery_enrichment_(start|pause|resume)$/.test(short(result.name)));
          const last=toolResultsThisPrompt.findLastIndex(result=>/^discovery_enrichment_(start|pause|resume)$/.test(short(result.name)));
          const receipt=result=>{if(!result.ok||result.truncated)return null;try{const v=JSON.parse(result.content);return v.structuredContent??v;}catch{return null;}};
          const reads=relevant.filter(result=>short(result.name)==='discovery_enrichment_status'&&receipt(result));
          const known=[...relevant].reverse().map(receipt).find(v=>typeof v?.run?.id==='string')?.run?.id;
          enrichmentBinding=known??goalText.match(/\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b/i)?.[0];
          const verified=attempted&&toolResultsThisPrompt.some((result,index)=>index>last&&reads.includes(result));
          let allowed=[];
          if(!verified&&relevant.filter(result=>!result.ok).length<2){
            if(attempted)allowed=['discovery_enrichment_status'];
            else if(enrichmentIntent.action==='status')allowed=reads.length?[]:['discovery_enrichment_status'];
            else if(enrichmentIntent.action==='start')allowed=['discovery_enrichment_start'];
            else if(known)allowed=['discovery_enrichment_'+enrichmentIntent.action];
            else if(!reads.length)allowed=['discovery_enrichment_status'];
          }
          tools=listedTools.filter(tool=>allowed.includes(short(tool._hostName))).map(tool=>{
            if(!known||!/^discovery_enrichment_(pause|resume|status)$/.test(short(tool._hostName)))return tool;
            return {...tool,function:{...tool.function,parameters:{...tool.function.parameters,properties:{...tool.function.parameters.properties,taskId:{type:'string',enum:[known]}}}}};
          });
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
        let handoffPhase=null;
        const handoffIntent=scanningHandoffIntent(goalText);
        if(handoffIntent){
          const short=name=>String(name).split(/::|__/).at(-1);
          const receipt=result=>{if(!result.ok||result.advisory||result.truncated)return null;try{const value=JSON.parse(result.content);const body=value.structuredContent??value;return body.outcomeUnknown?null:body;}catch{return null;}};
          const created=toolResultsThisPrompt.find(result=>short(result.name)==='scan_from_assets'&&typeof receipt(result)?.task?.id==='string');
          const taskId=created&&receipt(created).task.id;
          const creationIndex=toolResultsThisPrompt.indexOf(created);
          const creationAttempted=toolResultsThisPrompt.some(result=>short(result.name)==='scan_from_assets');
          const subsequent=toolResultsThisPrompt.slice(creationIndex+1);
          const matched=result=>receipt(result)?.task?.id===taskId&&result.arguments?.taskId===taskId;
          const inspected=taskId&&subsequent.some(result=>short(result.name)===(handoffIntent.preflight?'scan_preflight':'scan_status')&&matched(result));
          const startAttempted=subsequent.some(result=>short(result.name)==='scan_start');
          const startIndex=subsequent.findLastIndex(result=>short(result.name)==='scan_start');
          const executionChecked=startAttempted&&subsequent.slice(startIndex+1).some(result=>short(result.name)==='scan_status'&&matched(result));
          handoffPhase={created:Boolean(taskId),creationAttempted,inspected:Boolean(inspected),startAttempted,executionChecked};
          let allowed=['inventory_query','inventory_search','inventory_schema','scan_list','scan_status','scan_preflight','scan_from_assets'];
          if(creationAttempted&&!taskId){const attempt=toolResultsThisPrompt.findLastIndex(result=>short(result.name)==='scan_from_assets');allowed=toolResultsThisPrompt.slice(attempt+1).some(result=>['scan_list','scan_status'].includes(short(result.name)))?[]:['scan_list','scan_status'];} // Unknown writes are reconciled once, never replayed.
          if(taskId){
            const blockedStart=handoffIntent.start&&scanPreflightBlock(toolResultsThisPrompt,taskId);
            allowed=inspected?(handoffIntent.start?(startAttempted||blockedStart?['scan_status']:['scan_start']):[]):[handoffIntent.preflight?'scan_preflight':'scan_status'];
            if(blockedStart&&!scanBlockedHint){scanBlockedHint=true;session.messages.push({role:'user',content:`Host: 预检已阻断启动（${blockedStart}）。报告该原因，不要调用 scan_start，不要新建任务。`});}
          }
          if(executionChecked)allowed=[];
          tools=listedTools.filter(tool=>allowed.includes(short(tool._hostName)));
          if(taskId&&!tools.length&&!handoffFinishedHint){handoffFinishedHint=true;session.messages.push({role:'user',content:'Host: 本次资产交接及所请求的后续读取已经有同一 taskId 的成功回执。直接报告该任务的真实状态、保存模式与上限，不再重复预检或创建；创建草稿和预检都不是启动执行。'});}
        }
        let scanFollowupBinding;
        const scanIntent=scanningTaskIntent(goalText);
        // The destination is always this conversation; the Host checks that the old one is gone.
        const namedBindingDestination=/(?:重新关联|改绑|换绑|reassociate)/i.test(goalText)&&/(?:会话|conversation|这里|当前)/i.test(goalText);
        const scanTools=listedTools.filter(tool=>['scan_list','scan_status','scan_preflight','scan_recovery','scan_results','scan_fingerprint_search','scan_fingerprint_read','scan_start','scan_pause','scan_resume','scan_continue_targets','scan_bind','scan_reassociate_binding','scan_command','scan_command_ack','scan_review','scan_verify','scan_retry_delivery'].includes(String(tool._hostName).split(/::|__/).at(-1)));
        const scanTurn=Boolean(scanIntent&&scanTools.length&&!certificateTurn&&!enrichmentTurn&&!assistTurn&&!handoffIntent);
        if(scanTurn){
          const short=name=>String(name).split(/::|__/).at(-1);
          const successful=toolResultsThisPrompt.filter(result=>result.ok&&!result.advisory&&!result.truncated);
          const mutations=successful.filter(result=>/^scan_(start|pause|resume)$/.test(short(result.name)));
          const reads=successful.filter(result=>['scan_status','scan_list','scan_preflight','scan_recovery','scan_results'].includes(short(result.name)));
          const saved=result=>{try{const v=JSON.parse(result?.content);return v.structuredContent??v;}catch{return null;}};
          const last=toolResultsThisPrompt.findLastIndex(result=>/^scan_(start|pause|resume)$/.test(short(result.name)));
          const target=last>=0?(saved(toolResultsThisPrompt[last])?.task?.id??toolResultsThisPrompt[last].arguments?.taskId):undefined;
          const verified=last>=0&&toolResultsThisPrompt.some((result,index)=>index>last&&reads.includes(result)&&['scan_status','scan_recovery'].includes(short(result.name))&&(!target||saved(result)?.task?.id===target));
          const attempted=toolResultsThisPrompt.some(result=>/^scan_(start|pause|resume)$/.test(short(result.name)));
          const financial=toolResultsThisPrompt.some(result=>{if(short(result.name)!=='scan_resume')return false;try{const v=JSON.parse(result.content);const b=v.structuredContent??v;return b.recovery||/host_scan_financial_/.test(b.error??'')||/^scan_financial_/.test(b.reasonCode??'');}catch{return false;}});
          const followup=financial?'scan_recovery':'scan_status';
          const allowed=['scan_list','scan_status',...(financial?['scan_recovery']:[]),...(!attempted&&scanIntent.action!=='status'?['scan_'+scanIntent.action]:[]),...(!attempted&&['start','resume'].includes(scanIntent.action)?['scan_preflight']:[])];
          const blockedStart=allowed.includes('scan_start')?scanPreflightBlock(toolResultsThisPrompt,typeof target==='string'?target:undefined):null;
          if(blockedStart)allowed.splice(allowed.indexOf('scan_start'),1);
          const excludedPlan=scanExcludedCapabilities(toolResultsThisPrompt,typeof target==='string'?target:undefined);
          const askedExcluded=[['browser',/浏览器/],['uploadRole',/上传/],['concurrencyRole',/并发/]].filter(([id,pattern])=>excludedPlan.includes(id)&&pattern.test(goalText)).map(([id])=>id);
          if(askedExcluded.length){for(const name of ['scan_start','scan_resume','scan_continue_targets']){const index=allowed.indexOf(name);if(index>=0)allowed.splice(index,1);}}
          const controlFailure=scanControlFailure(toolResultsThisPrompt,typeof target==='string'?target:undefined);
          if(controlFailure){for(const name of ['scan_start','scan_resume','scan_continue_targets']){const index=allowed.indexOf(name);if(index>=0)allowed.splice(index,1);}if(!allowed.includes('scan_status'))allowed.push('scan_status');}
          if(namedBindingDestination&&!['start','resume'].includes(scanIntent.action))allowed.push('scan_reassociate_binding');
          tools=scanTools.filter(tool=>allowed.includes(short(tool._hostName)));
          if(blockedStart&&!scanBlockedHint){scanBlockedHint=true;session.messages.push({role:'user',content:`Host: 预检已阻断启动（${blockedStart}）。报告该原因，不要调用 scan_start，不要新建任务。`});}
          if(askedExcluded.length){session.messages.push({role:'user',content:`Host: 预检已把 ${askedExcluded.join('、')} 排除出可执行计划。不要启动、恢复或续跑这些能力，也不要把 HTTP 读取说成它们已经完成。`});}
          if(controlFailure){session.messages.push({role:'user',content:`Host: 启动或恢复已失败（${controlFailure}）。先前预检通过不能当成执行成功。读取 scan_status 报告这次失败，不要再次启动。`});}
          if(attempted&&!verified){tools=tools.filter(tool=>short(tool._hostName)===followup);if(!scanStatusHint){scanStatusHint=true;session.messages.push({role:'user',content:`Host: 扫描控制操作已返回。请使用 taskId 读取一次 ${followup}，然后报告实际状态、进度或恢复缺口。保留原账本，不新建预算、不重复启动或恢复，不循环等待完成；结果未知时先核对，不重发。`});}}
          if(attempted&&typeof target==='string'&&target.length<=128){scanFollowupBinding={taskId:target,tool:followup};tools=tools.map(tool=>short(tool._hostName)===followup?{...tool,function:{...tool.function,parameters:{...tool.function.parameters,required:['taskId'],properties:{taskId:{type:'string',enum:[target]}}}}}:tool);}
          const readLimit=scanIntent.action==='results'?8:3;
          if(verified||scanIntent.action==='status'&&reads.length||['preflight','recovery'].includes(scanIntent.action)&&reads.some(result=>short(result.name)==='scan_'+scanIntent.action)||reads.length>=readLimit)tools=[];
          // Keep only reference tools already present in the current granted/leased
          // list. A later short follow-up need not repeat a Skill keyword. The
          // mandatory post-control status/recovery read and global call caps remain.
          if(!attempted){
            const references=new Set(['scan_fingerprint_search','scan_fingerprint_read','skills_list','skills_read','skills_list_resources','skills_read_resource']);
            const present=new Set(tools.map(tool=>tool._hostName));
            tools.push(...listedTools.filter(tool=>references.has(short(tool._hostName))&&!present.has(tool._hostName)));
          }
          if(scanIntent.action==='results'&&reads.length>=readLimit&&!scanResultsHint){scanResultsHint=true;session.messages.push({role:'user',content:'Host: 本轮结果读取已达到分页调用上限。请依据已返回的完整 summary 与实际读过的页回答，明确 nextAfterId 和未读范围，不宣称已经读取全部记录；不为补读结果启动扫描。'});}
        }
        const exposureIntent=exposureTaskIntent(goalText);
        const exposureTurn=Boolean(exposureIntent&&listedTools.some(t=>String(t._hostName).split(/::|__/).at(-1)?.startsWith('exposure_')));
        if(exposureTurn){tools=exposureTurnTools(exposureIntent,goalText,listedTools,toolResultsThisPrompt);if(step===0)session.messages.push({role:'user',content:'Host: 本轮仅处理本项目公开暴露面请求。只读复核不写计划或结论；明确控制只提交一次并核对一次状态，后台继续执行。未知请求按原 operationId 核对，禁止重放，ID 必须实际读取。不要用资产发现、扫描或证书工具代替暴露面工具。'});}
        // A dedicated workflow may select only its requested control. If that
        // control was declined, do not also hide the independently granted
        // status/inventory reads needed to answer the rest of this request.
        const deniedBusinessControl=[...unavailableThisTurn].some(name=>/^(?:certificate_task|discovery_(?:cleanup|assist|enrichment)|exposure_task|scan)_(?:start|pause|resume|restore|cancel)$/.test(shortToolName(name)));
        if(!tools.length&&deniedBusinessControl){
          tools=listedTools.filter(tool=>isProjectReadTool(tool._hostName)&&!toolResultsThisPrompt.some(result=>result.ok&&!result.advisory&&result.name===tool._hostName));
          if(tools.length&&!partialReadHint){partialReadHint=true;session.messages.push({role:'user',_hostAuthorizationContext:true,content:'Host: 本次新增控制没有取得授权，仍有独立的已授权只读工具。先查询本项目已有状态或记录，再说明该控制未执行；不重新申请、不将读取称作启动或完成，也不扩大范围。'});}
        }
        const toolChoice = cleanupTurn && tools.length===1 ? 'required' : exposureTurn && tools.length===1 ? 'required' : assistTurn && tools.length === 1 ? 'required'
          : wantForceTool && !certificateTurn && !scanTurn && !enrichmentTurn && !assistTurn && !cleanupTurn && !exposureTurn ? 'required' : undefined;

        if (!readOnlyTurn && !handoffIntent && !enrichmentTurn && !assistTurn && !cleanupTurn && !exposureTurn && !injectedReview && shouldInjectReviewer(session.harness, { answerMode: options.answerMode, step })) {
          injectedReview = true;
          session.harness = noteReviewDone(session.harness, step);
          const hint = isDeadEnd(session.harness) ? `${REVIEWER_HINT}\n\n${DEEP_THINK_HINT}` : REVIEWER_HINT;
          session.messages.push({ role: 'user', content: hint });
          emitInner('hold', '同一条路反复失败，先回到你的目标');
          emit({ type: 'tool_call', toolCallId: `review-${step}`, title: '方向审查', status: 'in_progress' });
          emit({ type: 'tool_update', toolCallId: `review-${step}`, title: '方向审查', status: 'completed', summary: 'harness reviewer' });
        }

        if(!namedBindingDestination)tools=tools.filter(tool=>String(tool._hostName).split(/::|__/).at(-1)!=='scan_reassociate_binding');
        toolSelection.push({step,readOnlyTurn,handoffPhase,listedCount:listedTools.length,selectedCount:tools.length,selectedNames:tools.slice(0,16).map(tool=>tool._hostName),namesTruncated:tools.length>16});
        session.serialToolCalls=enrichmentTurn||assistTurn||cleanupTurn||exposureTurn;
        let result;
        try {
          result = await this.#modelTurn(session, tools, { toolChoice, onText: onStreamText });
        } finally {
          flushPending();
        }
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
          // Refusal explanations are already scrubbed, then clipped. A content-policy
          // stop leaves explanation empty and falls back to turn.text: scrub that full
          // text before the 240-character cut. A raw prefix can keep a split secret.
          const suppliedExplanation = typeof turn.refusal?.explanation === 'string' ? turn.refusal.explanation : '';
          const policyStop = !suppliedExplanation && (turn.finishReason === 'content_filter' || turn.refusal?.category === 'content_filter');
          const explanation = policyStop
            ? refusalExplanation(turn.text)
            : String(suppliedExplanation || turn.text || '').slice(0, 240);
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
            metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed, reasoningEffort: options.reasoningEffort, messageCount: session.messages.length },
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
          if (turn.finishReason === 'length' && !usedToolThisPrompt && !lengthContinued) {
            lengthContinued = true;
            options.maxTokens = Math.max(options.maxTokens || 0, 16384);
            emitInner('hold', '思考被长度截断，改为直接抓取');
            session.messages.push({ role: 'user', content: 'Host: 上一轮输出被长度截断，没有工具结果，用户还看不到分析。停止长篇思考。用户给了网址就立刻调用 workspace_fetch；给了本地文件就调用 workspace_read_path。抓到或读到之后再写解密脚本。' });
            continue;
          }
          if (turn.finishReason === 'length' || turn.finishReason === 'content_filter') {
            return { stopReason: turn.finishReason === 'length' ? 'max_tokens' : 'refusal', harness: session.harness, metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed } };
          }
          if((exposureTurn||cleanupTurn)&&tools.length===1){
            const selected=tools[0].function;
            if(!exposureToolHints.has(selected.name)){
              exposureToolHints.add(selected.name);
              session.messages.push({role:'user',content:`Host: 当前步骤实际可用且已通过权限筛选的工具是 ${selected.name}，参数以当前 schema 为准。刚才没有执行该步骤；上一轮只提供状态工具，不代表本轮没有控制工具。请按用户本轮明确请求调用当前工具，不要求重复授权，不重复已有或结果未知的控制操作。`});
              continue;
            }
            const note=(cleanupTurn?unresolvedCleanupControlNote(toolResultsThisPrompt):unresolvedExposureControlNote(toolResultsThisPrompt))||'当前步骤具备已授权的工具，但模型没有完成该步骤调用，尚无对应执行回执。请查看当前任务状态；这不表示权限丢失，也不能据此宣称任务已完成。';
            return{stopReason:'end_turn',replaceText:true,assistantText:note,harness:session.harness,metrics:{toolSelection,firstTextMs,firstToolMs,totalMs:performance.now()-started,api:apiUsed}};
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
              metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed, reasoningEffort: options.reasoningEffort, messageCount: session.messages.length },
              harness: session.harness,
            };
          }
          // Task 1: finishTurn completion policy. On a clean end-of-turn, force
          // one more model request (bounded by FORCED_CONTINUE_BUDGET) when an
          // in-scope autonomous analysis task did real work but produced no
          // deliverable and did not end by asking the user or hitting a scope
          // boundary. Cancellation and every existing stop still take precedence:
          // this runs only after the no-tool-calls guards above have declined.
          if (!session.abort.signal.aborted) {
            const decision = finishTurnDecision({
              answerMode: options.answerMode,
              goalText,
              usedToolThisPrompt,
              toolResultsThisPrompt,
              assistantText: turn.text || '',
              readOnlyTurn,
              scopeBoundary: unavailableThisTurn.size > 0 || authorizationAttempted.size > 0,
              forcedContinuesUsed: forcedContinues,
              budget: FORCED_CONTINUE_BUDGET,
              cancelled: session.abort.signal.aborted,
            });
            if (decision.continue) {
              forcedContinues += 1;
              forcedContinueReason = decision.reason;
              stepLimit = Math.max(stepLimit, step + 2);
              emitInner('hold', '任务还没产出成果，再推进一步');
              emit({ type: 'tool_call', toolCallId: `finish-${step}`, title: '继续完成任务', status: 'in_progress' });
              emit({ type: 'tool_update', toolCallId: `finish-${step}`, title: '继续完成任务', status: 'completed', summary: decision.reason });
              session.messages.push({ role: 'user', content: decision.hint });
              continue;
            }
          }
          emitInner('done');
          return {
            stopReason: turn.finishReason === 'length' ? 'max_tokens' : turn.finishReason === 'content_filter' ? 'refusal' : 'end_turn',
            // Tool-wave progress was streamed, but the completed message is
            // the final answer rather than a concatenation of every preface.
            ...((cleanupTurn&&unresolvedCleanupControlNote(toolResultsThisPrompt))||(exposureTurn&&unresolvedExposureControlNote(toolResultsThisPrompt))
              ? {replaceText:true,assistantText:cleanupTurn?unresolvedCleanupControlNote(toolResultsThisPrompt):unresolvedExposureControlNote(toolResultsThisPrompt)}
              : turn.text?.trim() ? { replaceText: true, assistantText: turn.text } : {}),
            metrics: {
              toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started,
              forcedToolHint, forcedEmptyBlock, forcedVerifier,
              forcedContinues, forcedContinueReason, toolSetAnnouncements,
              api: apiUsed, reasoningEffort: options.reasoningEffort,
              messageCount: session.messages.length,
            },
            harness: session.harness,
          };
        }

        if (firstToolMs === null) firstToolMs = performance.now() - started;
        const assistWaveActions=new Set();
        const calls = turn.toolCalls.map(call => {
          let args = {}, code = '';
          try {
            args = JSON.parse(call.arguments || '{}');
            if (!args || typeof args !== 'object' || Array.isArray(args)) code = 'host_tool_input_invalid';
          } catch { code = 'host_tool_input_invalid'; }
          const name = matchListedTool(call.name, tools);
          // The scoped executor owns identity. A model must not guess task IDs,
          // project fields, envelopes or translated parameter names for control.
          const ownerName=tools.find(tool=>tool.function.name===name)?._hostName;
          const discoveryAction=String(ownerName??'').split(/::|__/).at(-1);
          if(scanFollowupBinding&&discoveryAction===scanFollowupBinding.tool&&!code)args={taskId:scanFollowupBinding.taskId};
          if(cleanupTurn&&cleanupState?.bound&&/^discovery_cleanup_(pause|resume|restore)$/.test(discoveryAction)&&!code)args={...args,runId:cleanupState.bound};
          if(cleanupTurn&&discoveryAction==='discovery_cleanup_status'&&!code){if(cleanupState?.operation)args={operationId:cleanupState.operation};else if(cleanupState?.bound)args={...args,runId:cleanupState.bound};}
          if(cleanupTurn&&discoveryAction?.startsWith('discovery_cleanup_')){if(assistWaveActions.has(discoveryAction))code='host_tool_repeat_blocked';else assistWaveActions.add(discoveryAction);}
          if(assistTurn&&['discovery_assist_pause','discovery_assist_status'].includes(discoveryAction)&&!code)args={};
          if(assistTurn&&discoveryAction?.startsWith('discovery_assist_')){
            if(assistWaveActions.has(discoveryAction))code='host_tool_repeat_blocked';
            else assistWaveActions.add(discoveryAction);
          }
          if(enrichmentTurn&&discoveryAction==='discovery_enrichment_status'){
            args=enrichmentBinding?{taskId:enrichmentBinding}:{};
            code='';
          }else if(enrichmentTurn&&/^discovery_enrichment_(pause|resume)$/.test(discoveryAction)&&enrichmentBinding){
            args={taskId:enrichmentBinding};code='';
          }
          if (!tools.some(tool => tool.function?.name === name)) code = matchesToolRestriction(call._requestedName ?? name,unavailableThisTurn)
            ? 'host_tool_permission_denied' : 'host_tool_not_offered';
          if (turn.finishReason === 'length' || turn.finishReason === 'content_filter') code = 'host_provider_output_incomplete';
          return { id: call.id, name: tools.find(tool => tool.function.name === name)?._hostName ?? name, requestedName:call._requestedName ?? call.name, arguments: args, code };
        });
        // Tool visibility can also be narrowed by a read-only turn or a
        // control's once-only phase. Neither is a missing permission. Only the
        // Host's actual offer can establish a new grantable scope.
        const permissionCandidates=calls.filter(call=>call.code==='host_tool_not_offered'
          && !matchesToolRestriction(call.requestedName,authorizationAttempted)
          && !grantedTools.some(tool=>tool.function.name===matchListedTool(call.requestedName,grantedTools))
          && (!readOnlyTurn || isProjectReadTool(call.requestedName)));
        const rawPermissionOffer=permissionCandidates.length&&typeof this.host?.permissionOffer==='function'
          ? this.host.permissionOffer(session.conversationId,permissionCandidates.map(call=>call.requestedName)) : {offers:[]};
        const rawPermissionNames=offeredPermissionTools(rawPermissionOffer);
        const permissionOffer=remainingPermissionOffer(rawPermissionOffer,unavailableThisTurn,authorizationAttempted);
        const permissionNames=offeredPermissionTools(permissionOffer);
        for(const call of permissionCandidates){
          if(rawPermissionOffer.offers?.length&&(!rawPermissionNames.length||matchesPermissionTool(call.requestedName,rawPermissionNames)))call.code='host_tool_permission_denied';
        }
        const appendResults = results => {
          if (!results.length) return;
          toolResultsThisPrompt.push(...results.filter(result => !result.advisory).map(result=>({...result,arguments:calls.find(call=>call.id===result.id)?.arguments})));
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
        if (turn.finishReason === 'length' && !executable.length && !lengthContinued) {
          lengthContinued = true;
          options.maxTokens = Math.max(options.maxTokens || 0, 16384);
          emitInner('hold', '工具调用被截断，重新抓取');
          session.messages.push({ role: 'user', content: 'Host: 上一轮工具调用不完整。不要继续思考。立刻重新调用 workspace_fetch 或 workspace_read_path，参数必须是完整 JSON。' });
          continue;
        }
        if (turn.finishReason === 'length' || turn.finishReason === 'content_filter') {
          return { stopReason: turn.finishReason === 'length' ? 'max_tokens' : 'refusal', harness: session.harness, metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed } };
        }
        if (!executable.length && calls.every(call => ['host_tool_permission_denied','host_tool_not_offered'].includes(call.code))) {
          const offer=permissionOffer;
          if (offer.offers?.length && (typeof this.host?.requestAuthorization === 'function' || typeof this.host?.waitForAuthorization === 'function')) {
            for(const name of [...permissionCandidates.map(call=>call.requestedName),...permissionNames])authorizationAttempted.add(name);
            emitInner('wait', '这一步要先经过你');
            const request = { projectId: offer.projectId, identity: offer.identity, offers: offer.offers, question: suppliedTurn || !hasExplicitTurn ? loggedTurn : goalText };
            let authorization;
            if (typeof this.host.requestAuthorization === 'function') authorization = this.host.requestAuthorization(session.conversationId, request);
            else {
              authorization = this.host.waitForAuthorization(session.conversationId);
              emit({ type: 'authorization.requested', conversationId: session.conversationId, ...request });
            }
            const accepted = await awaitWhileActive(authorization, session.abort.signal);
            if (session.abort?.signal.aborted) throw Object.assign(Error('host_reply_cancelled'), { code: 'host_reply_cancelled' });
            if (accepted) {
              session.messages.push({ role: 'user', _hostAuthorizationContext:true, content: 'Host: 用户已完成这次权限确认。下一步只使用重新读取的当前 tools/list；沿本轮请求继续可做的部分，不重复申请同一权限，不输出 DSML 标记，不重发已发送或结果未知的控制。' });
              stepLimit = Math.max(stepLimit, step + 2);
              continue;
            }
            for(const name of [...permissionCandidates.map(call=>call.requestedName),...permissionNames])unavailableThisTurn.add(name);
            session.messages.push({role:'user',_hostAuthorizationContext:true,content:`Host: 用户未批准本次新增权限，以下工具本轮不得再次询问或执行：${JSON.stringify([...permissionCandidates.map(call=>call.requestedName),...permissionNames])}。继续其余已有权限覆盖的工作，仅说明这些具体工具的缺口。`});
          }
          if(tools.length&&!toolSelectionCorrection){
            toolSelectionCorrection=true;
            session.messages.push({role:'user',_hostAuthorizationContext:true,content:`Host: 刚才的工具选择没有执行，这不代表已有项目权限丢失。当前仍可使用的合法名称是 ${tools.map(tool=>tool.function.name).join('、')}。请纠正未执行的工具选择，完成请求中可做的部分；本轮被拒工具不再询问，不沿用旧名称，不重复已发送或结果未知的控制。此选择纠正仅一次。`});
            continue;
          }
          const permissionGap=calls.some(call=>call.code==='host_tool_permission_denied');
          emitInner('done', permissionGap ? '这一项未获准，其余已保存结果保留' : '模型未选中本轮可用工具');
          const controlNote = cleanupTurn&&unresolvedCleanupControlNote(toolResultsThisPrompt)
            || exposureTurn&&unresolvedExposureControlNote(toolResultsThisPrompt)
            || exposureTurn&&toolResultsThisPrompt.some(r=>r.ok&&/^exposure_task_(start|resume|cancel)$/.test(String(r.name).split(/::|__/).at(-1)))&&'暴露面控制请求已返回回执，但后续状态未能核对。请查看任务记录与当前进度，不要重复提交控制请求。'
          const note = controlNote || (permissionGap
            ? `本轮这些具体工具未获准，未执行且没有重复申请：${calls.filter(call=>call.code==='host_tool_permission_denied').map(call=>call.requestedName).join('、')}。其余已获准能力和已保存结果不受此缺口影响。`
            : `模型请求的工具没有出现在本轮可用列表，本次没有执行：${calls.map(call=>call.requestedName).join('、')}。${tools.length?'当前仍有其他可用工具，但本轮未正确选用；已有权限保持不变。':'当前没有对应的可用工具，这属于本次工具能力或选择缺口。'}`);
          onText(turn.text ? `\n\n${note}` : note);
          session.messages.push({ role: 'assistant', content: note });
          if (consumeSteer()) { stepLimit = Math.max(stepLimit, step + 2); continue; }
          return { stopReason: 'end_turn', replaceText:true, assistantText:note, metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed, reasoningEffort: options.reasoningEffort, messageCount: session.messages.length }, harness: session.harness };
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
          if (result.ok) failedExecutions.delete(key); else failedExecutions.set(key, permanentToolErrorCode(result) ? 2 : (failedExecutions.get(key) ?? 0) + 1);
        }
        const suppressed = blocked.map(call => ({ id: call.id, name: call.name, ok: false, code: 'host_tool_repeat_blocked', content: JSON.stringify({ error: 'host_tool_repeat_blocked', hint: 'This exact operation failed twice. It was not executed again. Use a different applicable read or explain the gap.' }) }));
        appendResults([...results, ...suppressed]);
        if (blocked.length && !ready.length) {
          // Don't quit the instant the model first repeats itself. Give it a bounded number
          // of chances to PIVOT to a genuinely different approach; only stall if it still has
          // nothing new. The suppressed results above already told it these ops failed twice.
          session._allBlockedPivots = (session._allBlockedPivots ?? 0) + 1;
          if (session._allBlockedPivots <= ALL_BLOCKED_PIVOT_BUDGET) {
            emitInner('hold', '同一条路走不通，换个做法');
            session.messages.push({ role: 'user', content: 'Host: 你这一步请求的工具+参数全是已经连续失败两次的重复操作，已被拦下、没有执行——别再重复它们。任务还没完成，不要就此停下：换一个真正不同的做法（换工具、换参数/路径、仔细读上面返回的错误再据此改、或改写脚本解决根因）。只有当你确实再没有任何可行的新办法时，才用一句话写清具体卡在哪、还差什么外部条件。' });
            continue;
          }
          const note = '相同工具和参数已连续失败两次，且已给过换方法的机会仍无新进展，已停止重复调用。本轮未取得所需结果；请检查工具返回的错误，或改用其他查询条件。';
          session.messages.push({ role: 'assistant', content: note });
          emitInner('done', '已停止无进展的重复调用');
          return { stopReason: 'tool_loop_stalled', replaceText: true, assistantText: note, harness: session.harness, metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed, suppressedCalls: blocked.length } };
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
          modelHttpAttempts: session.httpAttempts,
          firstTextMs, firstToolMs, totalMs: performance.now() - started,
          forcedToolHint, forcedEmptyBlock, forcedVerifier,
          forcedContinues, forcedContinueReason, toolSetAnnouncements,
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
          metrics: { toolSelection, firstTextMs, firstToolMs, totalMs: performance.now() - started, forcedToolHint, forcedEmptyBlock, forcedVerifier, api: apiUsed },
          harness: session.harness,
        };
      }
      if (['host_budget_hard_stop', 'host_budget_paused'].includes(error?.code)) {
        const note = '模型请求额度已用尽，本轮不再发送模型请求；已返回的工具回执仍保留，后台业务任务需另行核对状态。';
        emitInner('done', note);
        return { stopReason: 'paused_budget', replaceText: true, assistantText: note,
          metrics: { modelHttpAttempts: session.httpAttempts, firstTextMs, firstToolMs, totalMs: performance.now() - started, api: apiUsed, messageCount: session.messages.length }, harness: session.harness };
      }
      throw error;
    } finally { session.abort = null; }
  }

  async cancel(sessionId) { const session = this.sessions.get(sessionId); if (!session?.abort) return false; session.abort.abort(); return true; }
  async resolvePermission() { return false; }
  async close() { for (const session of this.sessions.values()) session.abort?.abort(); this.sessions.clear(); this.status.state = 'closed'; }
}
