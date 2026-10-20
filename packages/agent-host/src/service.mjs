import { proxyFetch } from './net-proxy.mjs';
import { HostNetworkSettings } from './network-settings.mjs';
import { createHash, randomUUID, randomBytes } from 'node:crypto';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdir } from 'node:fs/promises';
import { existsSync, writeFileSync } from 'node:fs';
import { HostStore, hostError, boundedText, RETRYABLE_HOST_CODES, deriveConversationTitle, isPlaceholderConversationTitle } from './store.mjs';
import { desktopAssetDbPath, isDesktopAssetRead, syncDesktopAssets } from './desktop-assets.mjs';
import { WORKSPACE_TOOL_SPECS, runWorkspaceTool, workspacePermission } from './workspace-tools.mjs';
import { AppManager } from './apps.mjs';
import { ensureBundledApps, bundledAppsStatus } from './bundled-apps.mjs';
import { shellCatalog as buildShellCatalog } from './shell-apps.mjs';
import { buildDiscoverySummary, formatDiscoveryPromptBlock } from './discovery.mjs';
import { formatMemoryPromptBlock, readConversationMemory, recordTurn, summarizeToolEvent, pinFact } from './memory.mjs';
import { ProviderStore, OpenAiCompatibleRuntime, truncateMcpResult } from './provider.mjs';
import { buildTurnPrompt, looksLikeDataQuestion, isProjectReadQuestion, isReadOnlyDataRequest, certificateTaskIntent, scanningTaskIntent, humanizeToolTitle, normalizeAnswerMode, answerModeOptions, prefersResponsesApi, scrubSecrets } from './quality-prompt.mjs';
import {
  requestDeviceCode, pollDeviceCodeOnce, pollDeviceCodeUntilDone, normalizeOAuthTokens,
  publicDevicePending, refreshAccessToken, accessTokenIsExpiring, XAI_OAUTH,
} from './xai-oauth.mjs';
import { buildPairedSkillsPreview, importPairedSkills as importPairedSkillsAction } from './paired-skills.mjs';
import { InsightLedger, openInsightInventoryDb, INSIGHT_INVENTORY_FILENAME, formatInsightDiffSummary } from './insight.mjs';
import { ASSETS_APP_ID } from './legacy-bridge.mjs';
import { ReadOnlySubagents, SUBAGENT_TOOL_SPECS } from './subagents.mjs';
import { VERIFIER_TOOL_SPECS, setTaskCriteria, verifyTask, assertAcceptance, acceptanceValid } from './task-verifier.mjs';
import { TASK_TOOL_SPECS, argumentDigest, bindReconciliation, formatTaskPrompt, startTaskRun, finishTaskRun, startTaskEvidence, finishTaskEvidence, updateTask, readOnlyAnswerPresentation, readOnlyContinuationQuestion } from './tasks.mjs';
import { buildTurnStopReport } from './turn-stop-report.mjs';
import { requestedBusinessTools } from './task-intent-permissions.mjs';
import { previewIntent, intentReply, renderTaskReport, renderTaskReportHtml, createBudgetAllocation, bindExecutionPreset } from './workflow-guard.mjs';

const RUNTIME_IDS = Object.freeze(['none', 'grok-acp', 'xai-api']);

function scrubbedToolBody(result) {
  const raw = JSON.stringify(result ?? null);
  return scrubSecrets(raw).slice(0, 32768);
}

function reconciliationCandidate(result) {
  let body = result?.structuredContent;
  if (!body && typeof result?.content?.[0]?.text === 'string') {
    try { body = JSON.parse(result.content[0].text); } catch { body = null; }
  }
  return body?.reconciliation;
}
const AUTHENTICATION_REQUIRED_MESSAGE = 'Official Grok authentication is required. Sign in separately using your approved private Grok profile, then explicitly retry. No login or message resend was started.';
const authenticationRequired = (code, details) => code === 'ACP_CACHED_AUTH_UNAVAILABLE' && details?.authenticationRequired === true && details?.outcomeUnknown === false;
const authenticationNotice = state => state === 'authentication_required' ? { authenticationRequired: true, message: AUTHENTICATION_REQUIRED_MESSAGE } : {};

/** Keepalive while a turn/tool is active so the UI does not look frozen. */
export const RUNTIME_HEARTBEAT_MS = 5_000;

/** Conversation orchestration only. Model intelligence stays in the selected runtime. */
export class AgentHost {
  constructor({ dataDir, runtime = null, runtimeFactory = null, runtimeSelection, store, apps, onEvent = () => {}, providerFetch, heartbeatMs, networkEnvironment } = {}) {
    if (runtimeSelection !== undefined && !RUNTIME_IDS.includes(runtimeSelection)) throw hostError('host_runtime_invalid');
    if (runtimeSelection === 'grok-acp' && !runtime && !runtimeFactory) throw hostError('host_runtime_unavailable');
    this.dataDir = dataDir; this.store = store ?? new HostStore(join(dataDir, 'host.sqlite'));
    this.apps = apps ?? new AppManager(this.store, join(dataDir, 'applications'));
    this.runtimeFactory = runtimeFactory; this.providerStore = new ProviderStore(dataDir);
    this.networkStore = new HostNetworkSettings(dataDir, { ...(networkEnvironment ? { env: networkEnvironment } : {}) });
    this.networkTransportInjected = Boolean(providerFetch);
    // An explicitly injected test transport keeps its old contract. Production
    // provider and OAuth requests share this host-owned routing policy.
    this.providerFetch = providerFetch ?? this.networkStore.createFetch();
    this.oauthFetch = this.providerFetch;
    this.oauthPending = null; this.oauthPoll = null; this.oauthAbort = null;
    const savedRuntime = this.store.db.prepare("SELECT value FROM host_settings WHERE key='runtime'").get()?.value;
    const wanted = runtimeSelection ?? (runtime ? 'grok-acp' : savedRuntime);
    if (wanted === 'xai-api' && !runtime) { this.runtime = this.#providerRuntime(); this.runtimeId = this.runtime ? 'xai-api' : 'none'; }
    else {
      this.runtime = runtimeSelection === 'none' ? null : runtimeSelection === 'grok-acp' ? (runtime ?? runtimeFactory())
        : runtime ?? (savedRuntime === 'grok-acp' && runtimeFactory ? runtimeFactory() : null);
      this.runtimeId = this.runtime ? 'grok-acp' : 'none';
    }
    this.runtime?.bindHost?.(this);
    this.onEvent = onEvent; this.runs = new Map(); this.permissions = new Map(); this.mcpScopes = new Map(); this.loadedSessions = new Map(); this.inflightEffects = new Map(); this.activeMcpCalls = new Set();
    this.heartbeatMs = Number.isFinite(heartbeatMs) && heartbeatMs > 0 ? heartbeatMs : RUNTIME_HEARTBEAT_MS;
    this.apps.onActivity = event => this.emit({ type: 'application.activity', appId: event.appId, ...(typeof event.busy === 'boolean' ? { busy: event.busy } : {}) });
    this.runtimeRevision = Number(this.store.db.prepare("SELECT value FROM host_settings WHERE key='runtime_revision'").get()?.value ?? 1);
    this.subagents = new ReadOnlySubagents(this);
    this.runtimeState = this.runtime ? 'configured' : 'unconfigured';
    this.runtimeEvidence = this.runtime?.evidence ?? (this.runtime?.command === 'grok' ? 'official-cli-unverified' : this.runtime ? 'synthetic-or-custom' : 'none');
  }
  runtimeIdentity() { return `${this.runtime ? this.runtimeId : 'none'}:${this.runtimeRevision}`; }
  #providerRuntime() {
    const config = this.providerStore.read();
    const bearer = this.providerStore.bearer();
    if (!config || (!bearer && config.authentication !== 'none')) return null;
    const answerMode = this.answerMode();
    const mode = answerModeOptions(answerMode);
    return new OpenAiCompatibleRuntime({
      config: { ...config, apiKey: bearer },
      ...(this.providerFetch ? { fetch: this.providerFetch } : {}),
      onEvent: event => this.runtimeEvent(event),
      answerMode,
      maxSteps: mode.maxSteps,
      timeoutMs: mode.timeoutMs,
    });
  }
  answerMode() {
    const saved = this.store.db.prepare("SELECT value FROM host_settings WHERE key='answer_mode'").get()?.value;
    return normalizeAnswerMode(saved);
  }
  setAnswerMode(mode) {
    const next = normalizeAnswerMode(mode);
    this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('answer_mode',?)").run(next);
    if (this.runtimeId === 'xai-api' && this.runtime && typeof this.runtime === 'object') {
      this.runtime.answerMode = next;
      const options = answerModeOptions(next);
      this.runtime.temperature = options.temperature;
      this.runtime.maxTokens = options.maxTokens;
      this.runtime.toolHintGuard = options.toolHintGuard;
      this.runtime.maxSteps = options.maxSteps;
      this.runtime.forceToolOnDataQuestion = options.forceToolOnDataQuestion;
      this.runtime.reasoningEffort = options.reasoningEffort;
      this.runtime.timeoutMs = options.timeoutMs;
      this.runtime.useResponsesApi = options.useResponsesApi && prefersResponsesApi(this.runtime.config ?? {});
    }
    return this.settings();
  }
  /** Product switch: omit projectId on POST /api/conversations → auto Scratch project. Default on. */
  scratchConversationsEnabled() {
    const saved = this.store.db.prepare("SELECT value FROM host_settings WHERE key='scratch_conversations'").get()?.value;
    if (saved === '0' || saved === 'false') return false;
    return true;
  }
  setScratchConversations(enabled) {
    const next = enabled === false || enabled === 0 || enabled === '0' || enabled === 'false' ? '0' : '1';
    this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('scratch_conversations',?)").run(next);
    return this.settings();
  }
  /**
   * Create a conversation. With projectId: existing project-bound path.
   * Without projectId: scratch when `scratch: true` or product switch is on.
   */
  createConversationFromRequest(input = {}) {
    const title = input.title;
    if (input.projectId != null && input.projectId !== '') {
      return this.store.createConversation(input.projectId, title);
    }
    const wantScratch = input.scratch === true || this.scratchConversationsEnabled();
    if (!wantScratch) throw hostError('host_project_required');
    const project = this.store.createScratchProject(typeof input.scratchName === 'string' ? input.scratchName : undefined);
    const conversation = this.store.createConversation(project.id, title);
    this.emit({ type: 'project.created', projectId: project.id, kind: project.kind, name: project.name });
    return conversation;
  }
  promoteProject(projectId, name) {
    const project = this.store.promoteProject(projectId, name);
    this.emit({ type: 'project.updated', projectId: project.id, kind: project.kind, name: project.name });
    return project;
  }
  /** Resolve assets inventory.sqlite3 path (installed app data dir, else conventional host path). */
  #assetsInventoryDbPath() {
    try {
      const app = this.store.applications().find(row => row.id === ASSETS_APP_ID);
      if (app) {
        const dataDir = this.apps.dataDirectoryFor(app);
        if (dataDir) return join(dataDir, INSIGHT_INVENTORY_FILENAME);
      }
    } catch {
      // App may be absent in early host tests; fall through to conventional path.
    }
    return join(this.apps.root ?? join(this.dataDir, 'applications'), 'data', ASSETS_APP_ID, INSIGHT_INVENTORY_FILENAME);
  }
  #withInsightLedger(fn) {
    const dbPath = this.#assetsInventoryDbPath();
    const db = openInsightInventoryDb(dbPath);
    const ledger = new InsightLedger({ db });
    try {
      return fn(ledger);
    } finally {
      db.close();
    }
  }
  /** Wave3 insight_propose: durable proposal in assets inventory.sqlite3 (model tool on assets). */
  proposeInsight(projectId, input = {}) {
    this.store.project(projectId);
    const proposal = this.#withInsightLedger(ledger => ledger.propose({
      projectId: Number(projectId),
      inventoryRecordId: input.inventoryRecordId,
      fields: input.fields,
      baseVersion: input.baseVersion,
      operationId: input.operationId ?? randomUUID(),
    }));
    this.#emitInsightProposal(proposal, { projectId: Number(projectId) });
    return proposal;
  }
  /** Notify agent-ui to show Confirm/Cancel for insight_commit (userConfirmed). */
  #emitInsightProposal(proposal, { projectId, conversationId } = {}) {
    if (!proposal?.proposalId) return;
    this.emit({
      type: 'insight.proposal',
      projectId: Number.isSafeInteger(projectId) ? projectId : proposal.projectId,
      ...(conversationId != null ? { conversationId } : {}),
      proposalId: proposal.proposalId,
      operationId: proposal.operationId,
      inventoryRecordId: proposal.inventoryRecordId,
      fields: proposal.fields,
      baseVersion: proposal.baseVersion,
      diffSummary: proposal.diffSummary || formatInsightDiffSummary(proposal),
      expiresAt: proposal.expiresAt,
    });
  }
  /** Wave3 insight_commit: requires userConfirmed === true; writes inventory_insights. */
  commitInsight(projectId, input = {}) {
    this.store.project(projectId);
    return this.#withInsightLedger(ledger => ledger.commit({
      projectId: Number(projectId),
      proposalId: input.proposalId,
      operationId: input.operationId,
      userConfirmed: input.userConfirmed,
      baseVersion: input.baseVersion,
    }));
  }
  async setProvider(input) {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    const view = this.providerStore.save(input ?? {});
    // Explicit connection selection cancels an outstanding login before it can overwrite this selection.
    this.#cancelOAuthPoll(); this.oauthPending = null;
    if (this.runtimeId === 'xai-api') await this.setRuntime('xai-api', { force: true });
    return { ...this.settings(), apiProvider: view };
  }
  async selectProviderOAuth() {
    if (this.runs.size || this.runtimeChanging || this.oauthPending?.status === 'pending') throw hostError('host_runtime_busy', 409);
    this.providerStore.selectOAuth();
    if (this.runtimeId === 'xai-api') await this.setRuntime('xai-api', { force: true });
    return this.settings();
  }
  networkSettings() {
    return this.networkStore.publicView({
      targets: { provider: this.providerStore.publicView().baseUrl || 'https://api.x.ai/v1', oauth: XAI_OAUTH.DEVICE_URL },
      transportInjected: this.networkTransportInjected,
    });
  }
  setNetwork(input) {
    if (this.runs.size || this.runtimeChanging || this.oauthPending?.status === 'pending') throw hostError('host_runtime_busy', 409);
    this.networkStore.save(input);
    const network = this.networkSettings();
    this.emit({ type: 'network.settings', network });
    return this.settings();
  }
  async clearProvider() {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    this.#cancelOAuthPoll();
    if (this.runtimeId === 'xai-api') await this.setRuntime('none');
    this.providerStore.clear(); return this.settings();
  }
  #cancelOAuthPoll() {
    this.oauthGeneration = (this.oauthGeneration ?? 0) + 1;
    this.oauthStarting = false;
    this.oauthAbort?.abort();
    this.oauthAbort = null;
    this.oauthPoll = null;
  }
  /** Start RFC 8628 device-code login; host background-polls and stores tokens on success. */
  async startProviderOAuth({ referrer = XAI_OAUTH.REFERRER, background = true } = {}) {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    this.#cancelOAuthPoll(); this.oauthPending = null;
    const generation = this.oauthGeneration;
    this.oauthStarting = true;
    const fetchImpl = this.oauthFetch ?? proxyFetch;
    let device;
    try { device = await requestDeviceCode({ fetch: fetchImpl, referrer }); }
    finally { if (generation === this.oauthGeneration) this.oauthStarting = false; }
    if (generation !== this.oauthGeneration) throw hostError('host_oauth_cancelled', 409);
    const expiresAt = Date.now() + (Number(device.expires_in) || 300) * 1000;
    this.oauthPending = {
      ...device,
      status: 'pending',
      expires_at: expiresAt,
      started_at: Date.now(),
      interval_ms: Math.max((Number(device.interval) || 5) * 1000, XAI_OAUTH.MIN_INTERVAL_MS),
    };
    if (background !== false) {
      this.oauthAbort = new AbortController();
      const signal = this.oauthAbort.signal;
      this.oauthPoll = this.#runOAuthPoll(device, signal).catch(error => {
        if (signal.aborted) return;
        if (this.oauthPending) {
          const code = error?.code || 'host_oauth_failed';
          this.oauthPending = {
            ...this.oauthPending,
            status: code === 'host_oauth_denied' ? 'denied' : code === 'host_oauth_expired' || code === 'host_oauth_timeout' ? 'expired' : 'error',
            error: code,
          };
          this.emit({ type: 'provider.oauth', status: this.oauthPending.status, error: code });
        }
      });
    }
    this.emit({ type: 'provider.oauth', status: 'pending', userCode: device.user_code });
    return {
      status: 'pending',
      userCode: device.user_code,
      verificationUri: device.verification_uri,
      verificationUriComplete: device.verification_uri_complete,
      expiresIn: device.expires_in ?? 300,
      expiresAt,
      oauth: publicDevicePending(this.oauthPending),
      ...this.settings(),
    };
  }
  async #runOAuthPoll(device, signal) {
    const fetchImpl = this.oauthFetch ?? proxyFetch;
    const tokens = await pollDeviceCodeUntilDone(device, { fetch: fetchImpl, signal });
    if (signal.aborted) return;
    const normalized = normalizeOAuthTokens(tokens);
    // Never log or emit raw tokens.
    this.providerStore.saveOAuth(normalized);
    this.oauthPending = { ...this.oauthPending, status: 'authorized', user_code: this.oauthPending?.user_code, verification_uri: this.oauthPending?.verification_uri, verification_uri_complete: this.oauthPending?.verification_uri_complete, expires_at: this.oauthPending?.expires_at };
    this.emit({ type: 'provider.oauth', status: 'authorized' });
    await this.setRuntime('xai-api', { force: true });
    // Clear pending after success (keep short window for UI poll).
    this.oauthPending = { status: 'authorized', user_code: device.user_code, verification_uri: device.verification_uri, verification_uri_complete: device.verification_uri_complete, expires_at: Date.now() + 30_000 };
  }
  /** Client poll / status. Advances one poll if host background poll is not running. */
  async pollProviderOAuth() {
    const pending = this.oauthPending;
    if (!pending) {
      const view = this.providerStore.publicView();
      return { status: view.oauthConfigured ? 'authorized' : 'idle', ...this.settings() };
    }
    if (pending.status === 'authorized') return { status: 'authorized', ...this.settings() };
    if (pending.status !== 'pending') return { status: pending.status, error: pending.error, ...this.settings() };
    if (pending.expires_at && Date.now() > pending.expires_at) {
      this.oauthPending = { ...pending, status: 'expired', error: 'host_oauth_expired' };
      return { status: 'expired', error: 'host_oauth_expired', ...this.settings() };
    }
    // If background poll is alive, just report status.
    if (this.oauthPoll) return { status: 'pending', oauth: publicDevicePending(this.oauthPending), ...this.settings() };
    // Manual single-step poll (tests / recovery).
    const fetchImpl = this.oauthFetch ?? proxyFetch;
    const generation = this.oauthGeneration;
    const result = await pollDeviceCodeOnce(pending.device_code, { fetch: fetchImpl });
    if (generation !== this.oauthGeneration || this.oauthPending !== pending) throw hostError('host_oauth_cancelled', 409);
    if (result.status === 'authorized') {
      const normalized = normalizeOAuthTokens(result.tokens);
      this.providerStore.saveOAuth(normalized);
      this.oauthPending = { status: 'authorized', user_code: pending.user_code, verification_uri: pending.verification_uri, verification_uri_complete: pending.verification_uri_complete, expires_at: Date.now() + 30_000 };
      this.emit({ type: 'provider.oauth', status: 'authorized' });
      await this.setRuntime('xai-api', { force: true });
      return { status: 'authorized', ...this.settings() };
    }
    if (result.status === 'pending') {
      if (result.slowDown) this.oauthPending = { ...pending, interval_ms: (pending.interval_ms || 5000) + XAI_OAUTH.SLOW_DOWN_INCREMENT_MS };
      return { status: 'pending', oauth: publicDevicePending(this.oauthPending), ...this.settings() };
    }
    this.oauthPending = { ...pending, status: result.status, error: result.error || result.status };
    return { status: result.status, error: result.error, ...this.settings() };
  }
  async logoutProviderOAuth() {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    this.#cancelOAuthPoll();
    this.oauthPending = null;
    const usingOAuth = this.providerStore.read()?.authentication === 'oauth';
    if (usingOAuth && this.runtimeId === 'xai-api') await this.setRuntime('none');
    this.providerStore.clearOAuth();
    this.emit({ type: 'provider.oauth', status: 'logged_out' });
    return this.settings();
  }
  /** Refresh OAuth access token when near expiry (or force=true after 401). Syncs runtime bearer. */
  async ensureOAuthAccess({ force = false } = {}) {
    const value = this.providerStore.read();
    if (!value || value.authentication !== 'oauth' || !value.refreshToken) return value;
    const expiring = (value.expiresAt && value.expiresAt - Date.now() <= XAI_OAUTH.ACCESS_REFRESH_SKEW_MS)
      || accessTokenIsExpiring(value.accessToken);
    if (!force && !expiring) return value;
    const fetchImpl = this.oauthFetch ?? proxyFetch;
    const tokens = await refreshAccessToken(value.refreshToken, { fetch: fetchImpl });
    this.providerStore.saveOAuth(normalizeOAuthTokens(tokens));
    const next = this.providerStore.read();
    const bearer = this.providerStore.bearer();
    if (bearer && this.runtimeId === 'xai-api' && this.runtime?.config) {
      this.runtime.config.apiKey = bearer;
    }
    return next;
  }
  async ensureSkills() {
    if (this.skillAccess) return this.skillAccess;
    if (this.skillsStarting) return this.skillsStarting;
    this.skillsStarting = (async () => {
      const [{ SkillLibrary }, { ConversationSkills }] = await Promise.all([import('../../skills-library/src/index.mjs'), import('./skills.mjs')]);
      this.library = new SkillLibrary({ dataDir: join(this.dataDir, 'skill-library') });
      this.skillAccess = new ConversationSkills(this.store, this.library, { runtimeIdentity: () => this.runtimeIdentity() });
      return this.skillAccess;
    })();
    try { return await this.skillsStarting; }
    finally { this.skillsStarting = null; }
  }
  bundledSkillsPath() {
    const packaged = fileURLToPath(new URL('../../skills-library/resources/compound-engineering.json.gz', import.meta.url));
    return existsSync(packaged) ? packaged : fileURLToPath(new URL('../../../src-tauri/resources/skill-packages/compound-engineering.json.gz', import.meta.url));
  }
  async skillsChanged() {
    if (!this.skillAccess) return;
    for (const id of this.skillAccess.affectedConversations()) if (this.runs.has(id)) { this.runs.get(id).accessInvalidated = true; await this.cancel(id); }
    this.emit({ type: 'applications.changed' });
  }
  async pairedSkillsPreview(appId) {
    await this.ensureSkills();
    return buildPairedSkillsPreview({ app: this.store.application(appId), library: this.library });
  }
  async importPairedSkills(appId, input = {}) {
    await this.ensureSkills();
    const result = await importPairedSkillsAction({
      app: this.store.application(appId),
      library: this.library,
      skillAccess: this.skillAccess,
      names: input.names,
      conversationId: input.conversationId ?? null,
      includeInSkillsRead: Boolean(input.includeInSkillsRead),
      runtimeIdentity: () => this.runtimeIdentity(),
      stagingRoot: join(this.dataDir, 'paired-skill-staging'),
    });
    await this.skillsChanged();
    return result;
  }
  async withPairedSkillsPreview(view) {
    if (!view?.id || !Array.isArray(view.pairedSkills) || !view.pairedSkills.length) return view;
    try {
      const pairedSkillsPreview = await this.pairedSkillsPreview(view.id);
      return { ...view, pairedSkillsPreview };
    } catch {
      return view;
    }
  }
  async ensureBundledApps(options = {}) {
    return ensureBundledApps(this, options);
  }
  bundledApps() { return bundledAppsStatus(this); }
  shellApps() { return buildShellCatalog(this.apps.list()); }
  settings() {
    const id = this.runtime ? this.runtimeId : 'none';
    const apiProvider = this.providerStore.publicView();
    const xaiLabel = apiProvider.oauthConfigured ? 'xAI Grok (OAuth)' : apiProvider.apiKeyConfigured ? 'xAI Grok API (API key)' : 'xAI Grok API';
    const answerMode = this.answerMode();
    const mode = answerModeOptions(answerMode);
    return {
      runtime: {
        id,
        state: this.runtimeState,
        label: id === 'grok-acp' ? 'Grok CLI · ACP (advanced)' : id === 'xai-api' ? `${xaiLabel} · primary Q&A` : 'No runtime configured',
        identity: this.runtimeIdentity(),
        ...authenticationNotice(this.runtimeState),
      },
      modelProvider: { source: 'legacy-desktop', availableHere: false },
      apiProvider,
      network: this.networkSettings(),
      oauth: publicDevicePending(this.oauthPending),
      runtimes: RUNTIME_IDS,
      answerMode,
      answerModes: ['quality', 'speed'],
      answerModeOptions: {
        temperature: mode.temperature,
        maxSteps: mode.maxSteps,
        maxTokens: mode.maxTokens,
        toolHintGuard: mode.toolHintGuard,
        reasoningEffort: mode.reasoningEffort,
        useResponsesApi: mode.useResponsesApi,
        forceToolOnDataQuestion: mode.forceToolOnDataQuestion,
      },
      preferredRuntime: 'xai-api',
      runtimeGuidance: '问答主路径：xAI Grok（OAuth）。Grok CLI/ACP 为高级/CLI 选项，不走 host 质量护栏。',
      scratchConversations: this.scratchConversationsEnabled(),
    };
  }
  async setRuntime(id, { force = false } = {}) {
    if (!RUNTIME_IDS.includes(id)) throw hostError('host_runtime_invalid');
    if (this.runs.size) throw hostError('host_runtime_busy', 409);
    if (this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    if (!force && (this.oauthStarting || this.oauthPending?.status === 'pending')) {
      this.#cancelOAuthPoll(); this.oauthPending = null;
    }
    if (!force && (this.runtime ? this.runtimeId : 'none') === id) return this.settings();
    if (id === 'grok-acp' && !this.runtimeFactory) throw hostError('host_runtime_unavailable');
    if (id === 'xai-api' && !this.providerStore.publicView().configured) throw hostError('host_provider_api_key_required');
    this.runtimeChanging = true;
    try {
    const next = id === 'grok-acp' ? this.runtimeFactory() : id === 'xai-api' ? this.#providerRuntime() : null;
    next?.bindHost?.(this);
    await this.runtime?.close(); this.runtime = next; this.runtimeId = next ? id : 'none'; this.runtimeRevision++;
    this.runtimeState = next ? 'configured' : 'unconfigured';
    this.runtimeEvidence = next?.evidence ?? (next?.command === 'grok' ? 'official-cli-unverified' : next ? 'synthetic-or-custom' : 'none');
    this.mcpScopes.clear(); this.loadedSessions.clear();
    this.store.db.exec('BEGIN IMMEDIATE');
    try {
      this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('runtime',?)").run(id);
      this.store.db.prepare("INSERT OR REPLACE INTO host_settings(key,value) VALUES('runtime_revision',?)").run(String(this.runtimeRevision));
      this.store.db.exec('COMMIT');
    } catch (error) { this.store.db.exec('ROLLBACK'); throw error; }
    this.emit({ type: 'runtime.status', state: this.runtimeState, identity: this.runtimeIdentity() });
    return this.settings();
    } finally { this.runtimeChanging = false; }
  }
  emit(event) { if (event.type === 'applications.changed') this.subagents?.revalidate(); try { this.onEvent({ ...event, timestamp: new Date().toISOString() }); } catch { this.observerErrors = (this.observerErrors ?? 0) + 1; } }
  #queueAuthorization(conversationId, details, legacy = false) {
    const id = String(conversationId), run = this.runs.get(id);
    if (!run || run.cancelled || run.state === 'cancelling') return Promise.resolve(false);
    if (!this.authWaits) this.authWaits = new Map();
    const queue = this.authWaits.get(id) ?? [];
    let resolve;
    const promise = new Promise(done => { resolve = done; });
    const pending = { requestId: randomUUID(), runId: run.id, runtimeIdentity: this.runtimeIdentity(), promise, resolve, legacy, shown: false, details };
    queue.push(pending); this.authWaits.set(id, queue);
    this.#publishAuthorization(id);
    return promise;
  }
  #publishAuthorization(conversationId) {
    const pending = this.authWaits?.get(conversationId)?.[0];
    if (!pending || pending.shown) return;
    const run = this.runs.get(conversationId);
    if (!run || run.id !== pending.runId || run.cancelled || run.state === 'cancelling') {
      this.#cancelAuthorizations(conversationId, pending.runId);
      return;
    }
    pending.shown = true;
    if (pending.details) this.emit({ ...pending.details, type: 'authorization.requested', conversationId, runId: pending.runId, requestId: pending.requestId });
  }
  #cancelAuthorizations(conversationId, runId) {
    const id = String(conversationId), queue = this.authWaits?.get(id);
    if (!queue) return;
    const cancelled = queue.filter(pending => runId === undefined || pending.runId === runId);
    const remaining = queue.filter(pending => runId !== undefined && pending.runId !== runId);
    if (remaining.length) this.authWaits.set(id, remaining); else this.authWaits.delete(id);
    for (const pending of cancelled) {
      pending.resolve(false);
      if (pending.shown) this.emit({ type: 'authorization.cancelled', conversationId: id, runId: pending.runId, requestId: pending.requestId });
    }
    if (remaining.length) this.#publishAuthorization(id);
  }
  requestAuthorization(conversationId, details) {
    return this.#queueAuthorization(conversationId, details);
  }
  waitForAuthorization(conversationId) {
    const id = String(conversationId);
    const run = this.runs.get(id);
    const pending = this.authWaits?.get(id)?.find(item => item.legacy && item.runId === run?.id);
    return pending?.promise ?? this.#queueAuthorization(id, null, true);
  }
  resolveAuthorization(conversationId, accepted, { requestId, runId, remember = false } = {}) {
    const id = String(conversationId), queue = this.authWaits?.get(id), pending = queue?.[0];
    if (!pending) return false;
    // New requests must name the exact displayed approval. Conversation alone is
    // insufficient: a late response could otherwise authorize a different run.
    if ((!pending.legacy || requestId !== undefined) && requestId !== pending.requestId) return false;
    if (runId !== undefined && runId !== pending.runId) return false;
    const run = this.runs.get(id);
    if (!run || run.id !== pending.runId || run.cancelled || run.state === 'cancelling' || pending.runtimeIdentity !== this.runtimeIdentity()) {
      this.#cancelAuthorizations(id, pending.runId);
      return false;
    }
    if (accepted === true && remember === true) {
      const kinds = (pending.details?.offers ?? []).filter(offer => offer.appId === 'workspace')
        .flatMap(offer => offer.permissions.map(item => item.permission))
        .filter(kind => ['workspace.read', 'workspace.write'].includes(kind));
      this.store.db.exec('BEGIN IMMEDIATE');
      try {
        const save = this.store.db.prepare('INSERT OR IGNORE INTO conversation_workspace_grants VALUES(?,?,?)');
        for (const kind of kinds) save.run(id, pending.runtimeIdentity, kind);
        this.store.db.exec('COMMIT');
      } catch (error) { this.store.db.exec('ROLLBACK'); throw error; }
    }
    queue.shift();
    if (!queue.length) this.authWaits.delete(id);
    pending.resolve(accepted === true);
    this.#publishAuthorization(id);
    return true;
  }
  workspaceGrants(conversationId, projectId) {
    this.store.conversation(conversationId, projectId);
    return { permissions: this.store.db.prepare('SELECT permission FROM conversation_workspace_grants WHERE conversation_id=? AND runtime_identity=? ORDER BY permission').all(conversationId, this.runtimeIdentity()).map(row => row.permission) };
  }
  clearWorkspaceGrants(conversationId, projectId) {
    this.store.conversation(conversationId, projectId);
    this.store.db.prepare('DELETE FROM conversation_workspace_grants WHERE conversation_id=?').run(conversationId);
    const run = this.runs.get(String(conversationId));
    run?.workspaceAllow?.clear();
    if (run) this.#cancelAuthorizations(String(conversationId), run.id);
    this.subagents.revalidate();
    return { cleared: true };
  }
  steer(conversationId, text) {
    const run = this.runs.get(String(conversationId));
    if (!run || run.state === 'cancelling') throw hostError('host_reply_unavailable', 404);
    const line = String(text ?? '').trim().slice(0, 2000);
    if (!line) throw hostError('host_request_failed');
    if (!run.steer) run.steer = [];
    run.steer.push(line);
    return { queued: true };
  }
  takeSteer(conversationId) {
    const run = this.runs.get(String(conversationId));
    const lines = run?.steer ?? [];
    if (run) run.steer = [];
    return lines;
  }
  /** Tools the model tried to call that exist but are not granted for this conversation's project. */
  permissionOffer(conversationId, requestedNames) {
    const conversation = this.store.conversation(conversationId);
    const names = [...new Set((requestedNames ?? []).map(name => String(name)))];
    const offers = [];
    for (const app of this.store.applications()) {
      if (!app.enabled) continue;
      const grant = this.store.grant(app.id, conversation.projectId);
      const missing = new Map();
      for (const tool of app.manifest?.tools ?? []) {
        const wanted = names.some(name => name === tool.name || name === `${app.id}__${tool.name}` || name === `${app.id}::${tool.name}`);
        if (!wanted || (grant.permissions.includes(tool.permission) && grant.disclosureRuntime === this.runtimeIdentity())) continue;
        const row = missing.get(tool.permission) ?? { permission: tool.permission, tools: [] };
        row.tools.push(tool.name);
        missing.set(tool.permission, row);
      }
      if (!missing.size) continue;
      offers.push({ appId: app.id, revision: grant.revision, permissions: [...missing.values()] });
    }
    return { projectId: conversation.projectId, identity: this.runtimeIdentity(), offers };
  }
  status() { return { protocolVersion: 1, runtime: { state: this.runtimeState, id: this.runtime ? this.runtimeId : null, identity: this.runtimeIdentity(), evidence: this.runtimeEvidence, ...authenticationNotice(this.runtimeState) },
    activeRuns: [...this.runs.values()].map(({ id, conversationId, state }) => ({ id, conversationId, state })), permissions: [...this.permissions.values()],
    authorizations: [...(this.authWaits?.entries() ?? [])].flatMap(([conversationId, queue]) => queue[0]?.details ? [{ ...queue[0].details, type: 'authorization.requested', conversationId, runId: queue[0].runId, requestId: queue[0].requestId }] : []) }; }
  setOrigin(origin) { this.origin = origin; }
  isCurrentRun(run) {
    if (run.parentRunId) { try { this.subagents.parentActive(run); return this.subagents.runs.get(run.id) === run; } catch { return false; } }
    return this.runs.get(run.conversationId) === run && !run.cancelled && !['cancelling', 'finishing'].includes(run.state);
  }
  #revokeRunScope(run) {
    for (const [token, scope] of this.mcpScopes) if (scope.conversationId === run.conversationId && (scope.runId === run.id || token === run.scopeToken)) this.mcpScopes.delete(token);
  }
  task(conversationId, { projectId, taskId } = {}) {
    const task = this.store.task(conversationId, taskId, projectId);
    return task ? { ...task, acceptanceValid: acceptanceValid(task) } : null;
  }
  taskView(conversationId, options = {}) {
    const task = this.task(conversationId, options);
    return task ? { ...task, readOnlyAnswer: readOnlyAnswerPresentation(task, this.store.turnMessages(conversationId, task.runs?.at(-1)?.userMessageId, task.projectId)) } : null;
  }
  async cancelChild(conversationId, input = {}) {
    if (!input || typeof input !== 'object' || Array.isArray(input) || Object.keys(input).some(key => !['projectId', 'taskId', 'childTaskId', 'parentRunId'].includes(key)) || input.projectId === undefined || typeof input.taskId !== 'string' || typeof input.childTaskId !== 'string' || typeof input.parentRunId !== 'string') throw hostError('host_child_input_invalid');
    const parent = this.store.task(conversationId, input.taskId, input.projectId);
    const child = this.store.task(conversationId, input.childTaskId, input.projectId);
    if (!input.taskId || !input.childTaskId || !input.parentRunId || parent.parentTaskId || child.parentTaskId !== parent.id || child.parentRunId !== input.parentRunId) throw hostError('host_child_scope_unavailable', 409);
    const run = this.runs.get(conversationId), active = this.subagents.runs.get(child.runs.at(-1)?.id);
    if (active && (!run || run.taskId !== parent.id || run.id !== input.parentRunId)) throw hostError('host_child_scope_unavailable', 409);
    if (active) { await this.subagents.stop(active, 'host_child_cancelled'); await active.completion; }
    return { task: this.task(conversationId, { taskId: parent.id, projectId: input.projectId }), childTask: this.task(conversationId, { taskId: child.id, projectId: input.projectId }) };
  }
  deleteConversation(conversationId, projectId) {
    if (this.runs.has(conversationId) || [...this.inflightEffects.values()].some(effect => effect.conversationId === conversationId)) throw hostError('host_reply_already_running', 409);
    return this.store.deleteConversation(conversationId, projectId);
  }
  #changeTask(run, change) {
    if (!run.taskId) return null;
    const current = this.store.task(run.conversationId, run.taskId);
    const next = this.store.saveTask(change(current));
    this.emit({ type: run.parentRunId ? 'task.child.updated' : 'task.updated', conversationId: run.conversationId, taskId: next.id, runId: run.id, ...(run.parentRunId ? { parentTaskId: run.parentTaskId, parentRunId: run.parentRunId } : {}), task: { ...next, acceptanceValid: acceptanceValid(next) } });
    return next;
  }
  setBudgetAllocation(projectId, input) {
    const saved = this.store.saveAllocation(projectId, { ...createBudgetAllocation(input), softPause: input.softPause ?? 0 });
    this.#writeAllocationReceipt(saved);
    return saved;
  }
  #writeAllocationReceipt(allocation) {
    const receipt = {
      schemaVersion: 1,
      hostCap: allocation.host,
      nativeCap: allocation.native,
      pending: allocation.pending,
      nativeSpent: allocation.nativeSpent,
      presetId: 'standard',
      limitsSource: 'runtime_config',
      price: null,
    };
    writeFileSync(join(this.dataDir, 'owner-allocation.json'), JSON.stringify(receipt));
  }
  reserveNativeBudget(projectId, amount) {
    return this.store.reserveNative(projectId, amount);
  }
  prepareTask(conversationId, { projectId, choice, scope }) {
    const conversation = this.store.conversation(conversationId, projectId);
    const budgetRevision = this.store.allocation(conversation.projectId)?.revision ?? 0;
    const scopeHash = createHash('sha256').update(JSON.stringify({ projectId: conversation.projectId, choice, scope: scope ?? '', budgetRevision })).digest('hex');
    return this.store.prepareApproval({ id: randomUUID(), projectId: conversation.projectId, conversationId, scopeHash, choice: String(choice || 'view_link'), budgetRevision });
  }
  bindPreset(conversationId, { projectId, presetId, actor = 'user' }) {
    const task = this.task(conversationId, { projectId });
    const choice = bindExecutionPreset(task.executionPreset?.presetRef?.id, presetId, actor);
    return this.store.saveTask({ ...task, executionPreset: choice });
  }
  recallSummaries(projectId) {
    this.store.project(projectId);
    return { items: this.store.querySummaries(projectId) };
  }
  admitModelRequest(conversationId) {
    const conversation = this.store.conversation(conversationId);
    return this.store.admitModelRequest(conversation.projectId);
  }
  confirmTask(conversationId, { projectId, approvalId, scopeHash }) {
    const conversation = this.store.conversation(conversationId, projectId);
    const confirmed = this.store.confirmApproval({ projectId, approvalId, scopeHash });
    if (!this.store.task(conversationId, undefined, conversation.projectId)) {
      this.store.createTask(conversationId, '已确认的本地范围。确认本身不启动扫描。');
    }
    return { ...confirmed, taskId: this.store.task(conversationId, undefined, conversation.projectId).id };
  }
  queryHistory(projectId, options) {
    this.store.project(projectId);
    return { items: this.store.queryHistory(projectId, options) };
  }
  recordSummary(projectId, conversationId, { sourceIds = [], body = '' } = {}) {
    const project = projectId ?? this.store.conversation(conversationId).projectId;
    return this.store.saveSummaryVersion({ id: randomUUID(), projectId: project, conversationId, sourceIds, body: String(body).slice(0, 4000) });
  }
  addHumanReview(conversationId, { projectId, taskId, reportVersion, note }) {
    const task = this.task(conversationId, { projectId, taskId });
    return this.store.addHumanReview({ id: randomUUID(), projectId: task.projectId, taskId: task.id, reportVersion, note });
  }
  taskReport(conversationId, { projectId, taskId, format = 'markdown' } = {}) {
    const task = this.task(conversationId, { projectId, taskId });
    const kind = format === 'html' ? 'html' : 'markdown';
    const body = kind === 'html' ? renderTaskReportHtml(task) : renderTaskReport(task);
    const snapshotHash = createHash('sha256').update(JSON.stringify({ id: task.id, revision: task.revision, evidence: task.evidence, status: task.status })).digest('hex');
    return this.store.saveReportVersion({ id: randomUUID(), projectId: task.projectId, taskId: task.id, format: kind, snapshotHash, body });
  }
  async continueTask(conversationId, input = {}) {
    const task = this.store.task(conversationId, input.taskId, input.projectId);
    if (!task) throw hostError('host_task_unavailable', 404);
    if (!this.runtime) throw hostError('host_runtime_unconfigured', 409);
    const readQuestion = !input.text && readOnlyContinuationQuestion(task, this.store.turnMessages(conversationId, task.runs.at(-1)?.userMessageId, task.projectId));
    return this.send(conversationId, { projectId: task.projectId, taskId: task.id, text: input.text || readQuestion || '继续原任务。先核实中断操作的实际结果，再推进尚未完成的步骤；不要盲目重放写操作。' });
  }
  async send(conversationId, input) {
    if (this.oauthStarting || this.oauthPending?.status === 'pending') throw hostError('host_runtime_busy', 409);
    if (this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    const conversation = this.store.conversation(conversationId, input.projectId);
    if (this.runs.has(conversationId)) throw hostError('host_reply_already_running', 409);
    if ([...this.inflightEffects.values()].some(effect => effect.conversationId === conversationId)) throw hostError('host_task_effect_pending', 409);
    const text = boundedText(input.text);
    const intent = previewIntent(text);
    if (intent) {
      const userMessage = this.store.addMessage(conversationId, 'user', text);
      const note = intentReply(intent, input.confirmedIntent);
      const assistant = this.store.addMessage(conversationId, 'assistant', note, 'completed');
      this.store.finishMessage(assistant.id, note, 'completed', { stopReason: 'intent_preview', outcomeUnknown: false });
      this.emit({ type: 'message.completed', conversationId, messageId: assistant.id, status: 'completed', content: note, stopReason: 'intent_preview' });
      return { userMessage, intent, savedOnly: true };
    }
    let task = this.store.task(conversationId, input.taskId, conversation.projectId);
    if (task?.parentTaskId) throw hostError('host_child_scope_unavailable', 403);
    if (!task || (task.status === 'completed' && input.taskId === undefined)) task = this.store.createTask(conversationId, text);
    if (isPlaceholderConversationTitle(conversation.title)) {
      const titled = this.store.updateConversationTitle(conversation.id, deriveConversationTitle(text));
      conversation.title = titled.title;
      this.emit({ type: 'conversation.updated', conversationId: conversation.id, projectId: conversation.projectId, title: titled.title });
    }
    const userMessage = this.store.addMessage(conversationId, 'user', text);
    const runUserMessageId = userMessage.id;
    if (!this.runtime) return { userMessage, task, runtime: { state: 'unconfigured' }, savedOnly: true };
    const run = { id: randomUUID(), conversationId, taskId: task.id, sessionId: null, userMessageId: runUserMessageId, state: 'starting', text: '', started: performance.now() };
    run.message = this.store.addMessage(conversationId, 'assistant', '', 'running', run.id);
    task = this.store.saveTask(startTaskRun(task, run));
    this.runs.set(conversationId, run);
    this.emit({ type: 'task.updated', conversationId, taskId: task.id, runId: run.id, task });
    this.emit({ type: 'applications.changed', projectId: conversation.projectId });
    // The HTTP response does not claim the model turn completed.
    run.completion = this.#run(conversation, text, run).catch(() => {});
    return { userMessage, task, run: { id: run.id, conversationId, taskId: task.id, state: run.state } };
  }
  async #run(conversation, text, run) {
    let status = 'failed';
    run.lastEventAt = new Date().toISOString();
    const heartbeat = setInterval(() => {
      if (!this.isCurrentRun(run)) return;
      const elapsedMs = Math.round(performance.now() - run.started);
      const step = run.plan?.steps?.find(item => item.status === 'in_progress') || run.plan?.steps?.at(-1);
      this.emit({
        type: 'runtime.heartbeat',
        conversationId: conversation.id,
        runId: run.id,
        elapsedMs,
        lastEventAt: run.lastEventAt,
        ...(step ? { stepId: step.id, stepTitle: step.title, stepStatus: step.status } : {}),
      });
    }, this.heartbeatMs);
    try {
      const skills = await this.ensureSkills();
      if (run.cancelled) throw hostError('host_reply_cancelled');
      run.skillScope = skills.capture(conversation.id);
      await this.runtime.start(); this.runtimeState = 'ready';
      if (run.cancelled) throw hostError('host_reply_cancelled');
      skills.validate(conversation.id, run.skillScope);
      const cwd = join(this.dataDir, 'workspaces', String(conversation.projectId));
      await mkdir(cwd, { recursive: true, mode: 0o700 });
      // A retained bridge is not authority for a later turn. Rebind the session
      // with a fresh run capability and permanently revoke all older tokens.
      for (const [token, scope] of this.mcpScopes) if (scope.conversationId === conversation.id) this.mcpScopes.delete(token);
      const scopeToken = run.scopeToken = randomBytes(32).toString('hex');
      this.mcpScopes.set(scopeToken, { conversationId: conversation.id, runtimePid: this.runtime.status.pid, runId: run.id });
      const mcpServers = this.origin ? [{ name: 'oviraptor-apps', command: process.execPath,
        args: [fileURLToPath(new URL('./mcp-bridge.mjs', import.meta.url))], env: [
          { name: 'OVIRAPTOR_HOST_ORIGIN', value: this.origin }, { name: 'OVIRAPTOR_MCP_TOKEN', value: scopeToken },
          { name: 'OVIRAPTOR_CONVERSATION', value: conversation.id },
          { name: 'OVIRAPTOR_RUN_ID', value: run.id },
        ] }] : [];
      let resumeCold = false;
      if (conversation.sessionId) {
        resumeCold = this.loadedSessions.get(conversation.sessionId) !== this.runtime.status.pid;
        await this.runtime.loadSession({ sessionId: conversation.sessionId, cwd, mcpServers });
        run.sessionId = conversation.sessionId;
      } else {
        const result = await this.runtime.newSession({ cwd, mcpServers });
        run.sessionId = typeof result === 'string' ? result : result.sessionId;
        if (!run.sessionId) throw hostError('host_runtime_session_invalid');
        this.store.session(conversation.id, run.sessionId);
      }
      if (resumeCold && typeof this.runtime.seedTranscriptIfEmpty === 'function') {
        const prior = this.store.messages(conversation.id, conversation.projectId)
          .filter(row => row.id !== run.userMessageId && row.id !== run.message?.id && row.status !== 'running')
          .map(row => ({ role: row.role, content: row.content }));
        const seeded = this.runtime.seedTranscriptIfEmpty(run.sessionId, prior);
        if (seeded) {
          this.emit({
            type: 'runtime.inner',
            conversationId: conversation.id,
            runId: run.id,
            phase: 'model',
            phaseLabel: '正在把前文交给新模型',
            goal: text,
            facts: [],
            hypotheses: [],
            compressed: false,
            hold: `已接上这条会话里的 ${seeded} 条消息`,
          });
        }
      }
      this.loadedSessions.set(run.sessionId, this.runtime.status.pid);
      if (run.cancelled) throw hostError('host_reply_cancelled');
      run.state = 'running'; this.emit({ type: 'runtime.status', conversationId: conversation.id, runId: run.id, state: 'running' });
      const project = this.store.project(conversation.projectId);
      // Ask once for the exact business scopes missing from an explicit intent.
      // A previous persisted grant for this runtime never produces another offer.
      if (this.runtimeId === 'xai-api' && project.externalId) {
        const requested=requestedBusinessTools(text,{ applications:this.store.applications(),
          certificateAvailable:Boolean(this.apps.certificateTasks?.available), scanningAvailable:Boolean(this.apps.scanningTasks?.available) });
        const offer = requested.length?this.permissionOffer(conversation.id,requested):null;
        if (offer?.offers.length) {
          const allowed = await this.requestAuthorization(conversation.id, { ...offer, question: text });
          if (run.cancelled) throw hostError('host_reply_cancelled');
          const available=new Set(this.apps.capabilities(conversation.projectId,{runtimeIdentity:this.runtimeIdentity()}).map(tool=>tool.name));
          if (!allowed || !requested.every(name=>available.has(name))) throw hostError('host_tool_permission_denied', 403);
        }
      }
      const discovery = buildDiscoverySummary({
        apps: this.apps,
        projectId: conversation.projectId,
        runtimeIdentity: this.runtimeIdentity(),
        skillLease: run.skillScope,
        library: this.library,
      });
      run.discoveryFingerprint = discovery.fingerprint;
      const memory = readConversationMemory(this.store, conversation.id);
      const memoryBlock = formatMemoryPromptBlock(memory);
      run.toolMemory = [];
      run.userText = text;
      const answerMode = this.answerMode();
      // Single blackboard: runtime session harness owns goal/facts. Do NOT inject an empty ephemeral goal here.
      const taskContext = this.store.task(conversation.id, run.taskId);
      const prompt = (isReadOnlyDataRequest(text) || isProjectReadQuestion(text) || certificateTaskIntent(text) || scanningTaskIntent(text) ? '' : formatTaskPrompt(taskContext)) + buildTurnPrompt({
        project,
        discoveryBlock: formatDiscoveryPromptBlock(discovery),
        memoryBlock,
        userText: text,
        answerMode,
      });
      // OAuth lifetime: refresh before send so expired access does not bare-401.
      if (this.runtimeId === 'xai-api') {
        try { await this.ensureOAuthAccess(); } catch { /* 401 path / modelTurn may force-refresh once */ }
      }
      run.promptStarted = true;
      // 401 refresh+retry lives in OpenAiCompatibleRuntime.#modelTurn (once, only if no text streamed).
      const result = await this.runtime.prompt({ sessionId: run.sessionId, prompt, userText: text, taskContext: { id: taskContext.id, goal: taskContext.goal }, turnId: run.id, answerMode });
      if (result?.replaceText && typeof result.assistantText === 'string') run.text = result.assistantText;
      skills.validate(conversation.id, run.skillScope);
      run.metrics = result.metrics;
      run.stopReason = result.stopReason; run.outcomeUnknown = false;
      const stopped = ['max_turn_requests', 'paused_budget', 'tool_loop_stalled'].includes(result.stopReason)
        ? buildTurnStopReport(this.store.task(conversation.id, run.taskId), run, result.stopReason, run.currentToolResults, this.store.allocation(conversation.projectId)) : null;
      if (stopped && !run.accessInvalidated) {
        run.text = stopped.text;
        run.outcomeUnknown = stopped.outcomeUnknown;
        run.metrics = { ...run.metrics, stopReport: { source: 'host_receipts', ...stopped.counts } };
      }
      // Write session harness confirmed facts back into SQLite memory (advisory).
      try {
        const facts = result?.harness?.confirmedFacts;
        if (Array.isArray(facts)) {
          for (const fact of facts.slice(-4)) pinFact(this.store, conversation.id, fact);
        }
      } catch { /* advisory */ }
      status = run.accessInvalidated ? 'failed' : result.stopReason === 'cancelled' ? 'cancelled' : result.stopReason === 'refusal' ? 'refused' : result.stopReason === 'end_turn' ? 'completed' : 'incomplete';
    } catch (error) {
      const cancelledBeforePrompt = error.code === 'host_reply_cancelled' && !run.promptStarted;
      status = cancelledBeforePrompt ? 'cancelled' : 'failed';
      run.failureCode = typeof error.code === 'string' ? error.code : 'host_runtime_failed';
      if (run.accessInvalidated || run.failureCode === 'host_skill_access_changed') { run.text = ''; run.accessInvalidated = true; }
      run.outcomeUnknown = cancelledBeforePrompt ? false : Boolean(error.details?.outcomeUnknown ?? run.promptStarted);
      run.stopReason = error.details?.stopReason ?? '';
      run.authenticationRequired = !run.promptStarted && authenticationRequired(run.failureCode, error.details);
      if (run.authenticationRequired && this.runtimeState !== 'authentication_required') {
        this.runtimeState = 'authentication_required';
        this.emit({ type: 'runtime.status', state: this.runtimeState, identity: this.runtimeIdentity(), ...authenticationNotice(this.runtimeState) });
      }
      this.emit({ type: 'runtime.error', conversationId: conversation.id, runId: run.id, code: typeof error.code === 'string' ? error.code : 'host_runtime_failed',
        ...(run.authenticationRequired ? { authenticationRequired: true, outcomeUnknown: false, message: AUTHENTICATION_REQUIRED_MESSAGE } : {}) });
    } finally {
      clearInterval(heartbeat);
      run.state = 'finishing';
      this.#revokeRunScope(run);
      this.#cancelAuthorizations(conversation.id, run.id);
      await this.subagents.cancelTree(run.id);
      const disposition = { outcomeUnknown: Boolean(run.outcomeUnknown), failureCode: run.failureCode ?? '', stopReason: run.stopReason ?? '', cancelRequested: Boolean(run.cancelled),
        ...(run.authenticationRequired ? { authenticationRequired: true } : {}) };
      this.store.finishMessage(run.message.id, run.text, status, disposition);
      this.#changeTask(run, task => finishTaskRun(task, run, status, disposition));
      try {
        if (run.promptStarted) {
          recordTurn(this.store, conversation.id, {
            userText: run.userText ?? text,
            assistantText: run.text,
            tools: run.toolMemory ?? [],
          });
        }
      } catch { /* Memory is advisory; never fail the turn. */ }
      const hostTotalMs = performance.now() - run.started;
      this.store.metrics(run.id, conversation.id, { fixture: this.runtimeEvidence === 'synthetic-or-custom', runtimeEvidence: this.runtimeEvidence, totalMs: hostTotalMs, outcome: status, ...(run.metrics ?? {}), hostTotalMs, ...disposition });
      for (const [id, permission] of this.permissions) if (permission.runId === run.id) this.permissions.delete(id);
      this.runs.delete(conversation.id);
      this.skillAccess?.release?.(conversation.id, run.skillScope);
      this.emit({ type: 'message.completed', conversationId: conversation.id, runId: run.id, status, content: run.text, messageId: run.message.id, ...disposition });
    }
  }
  runtimeEvent(event) {
    if (event.type === 'runtime_ready') { this.runtimeState = 'ready'; this.emit({ type: 'runtime.status', state: 'ready', identity: this.runtimeIdentity() }); }
    if (event.type === 'runtime_error') {
      this.runtimeState = authenticationRequired(event.code, event) ? 'authentication_required' : 'failed';
      this.emit({ type: 'runtime.status', state: this.runtimeState, identity: this.runtimeIdentity(), code: event.code, ...authenticationNotice(this.runtimeState) });
    }
    if (event.type === 'permission_resolved') this.permissions.delete(String(event.requestId));
    const run = [...this.runs.values()].find(run => event.sessionId && run.sessionId === event.sessionId);
    if (!run) return;
    if (event.replay || (event.turnId && event.turnId !== run.id)) return;
    run.lastEventAt = new Date().toISOString();
    if (run.skillScope && this.skillAccess) { try { this.skillAccess.validateQuick(run.conversationId, run.skillScope); } catch { run.accessInvalidated = true; void this.cancel(run.conversationId); return; } }
    const base = { conversationId: run.conversationId, runId: run.id, messageId: run.message.id };
    if (event.type === 'text_delta') {
      const text = event.text ?? event.delta ?? '';
      if (typeof text !== 'string' || Buffer.byteLength(run.text + text) > 262144) { void this.cancel(run.conversationId); return; }
      run.text += text;
      this.emit({ type: 'message.delta', ...base, delta: text });
    } else if (event.type === 'permission_requested') {
      const id = String(event.id ?? event.requestId);
      const options = (event.options ?? []).map(option => ({ id: option.id ?? option.optionId, label: option.label ?? option.name ?? option.kind }));
      const permission = { id, ...base, title: 'Runtime permission request', description: event.title ?? event.toolCall?.title ?? 'Review this operation before allowing it.', options };
      this.permissions.set(id, permission); this.emit({ type: 'permission.request', ...permission });
    } else if (event.type === 'performance' || event.type === 'turn_error') run.metrics = event.metrics ?? event;
    else if (event.type === 'tool_call' || event.type === 'tool_update' || event.type === 'plan' || event.type === 'step') {
      if (this.runtimeId !== 'xai-api' && event.type === 'tool_call') run.externalEffectsPossible = true;
      this.#normalizePlanStep(run, event, base);
      if (event.type === 'tool_call' || event.type === 'tool_update') {
        const summary = summarizeToolEvent(event);
        if (summary) {
          if (!run.toolMemory) run.toolMemory = [];
          const existing = run.toolMemory.find(item => item.name === summary.name);
          if (existing) { existing.status = summary.status; if (summary.summary) existing.summary = summary.summary; }
          else if (run.toolMemory.length < 6) run.toolMemory.push(summary);
        }
        this.emit({ ...event, ...base, type: `runtime.${event.type}` });
      }
    } else this.emit({ ...event, ...base, type: `runtime.${event.type}` });
  }
  #normalizePlanStep(run, event, base) {
    if (!run.plan) run.plan = { steps: [], emitted: false };
    const toolCallId = String(event.toolCallId ?? event.id ?? event.update?.toolCallId ?? `step-${run.plan.steps.length + 1}`);
    const rawTitle = event.title ?? event.update?.title ?? event.toolCall?.title ?? toolCallId;
    const title = (event.type === 'tool_call' || event.type === 'tool_update') ? humanizeToolTitle(rawTitle) : rawTitle;
    const status = event.status ?? event.update?.status ?? (event.type === 'tool_call' ? 'in_progress' : 'completed');
    let step = run.plan.steps.find(item => item.id === toolCallId);
    if (!step) {
      step = { id: toolCallId, title: String(title).slice(0, 240), status: 'pending', index: run.plan.steps.length };
      run.plan.steps.push(step);
    }
    step.title = String(title).slice(0, 240);
    step.status = status;
    if (!run.plan.emitted && run.plan.steps.length) {
      run.plan.emitted = true;
      this.emit({ type: 'runtime.plan', ...base, steps: run.plan.steps.map(item => ({ id: item.id, title: item.title, status: item.status, index: item.index })) });
    } else if (run.plan.emitted) {
      this.emit({ type: 'runtime.plan', ...base, steps: run.plan.steps.map(item => ({ id: item.id, title: item.title, status: item.status, index: item.index })) });
    }
    this.emit({ type: 'runtime.step', ...base, id: step.id, title: step.title, status: step.status, index: step.index });
  }
  async #authorizeWorkspace(conversation, run, toolName) {
    const kind = workspacePermission(toolName);
    if (!run.workspaceAllow) run.workspaceAllow = new Set();
    if (run.workspaceAllow.has(kind)) return;
    if (this.store.db.prepare('SELECT 1 FROM conversation_workspace_grants WHERE conversation_id=? AND runtime_identity=? AND permission=?').get(conversation.id, this.runtimeIdentity(), kind)) { run.workspaceAllow.add(kind); return; }
    if (!run.workspaceAuthorizations) run.workspaceAuthorizations = new Map();
    if (!run.workspaceAuthorizations.has(kind)) {
      const pending = this.requestAuthorization(conversation.id, {
          projectId: conversation.projectId,
          identity: this.runtimeIdentity(),
          question: run.userText || '',
          offers: [{ appId: 'workspace', revision: 0, permissions: [{ permission: kind, tools: [toolName] }] }],
      });
      run.workspaceAuthorizations.set(kind, pending);
    }
    const pending = run.workspaceAuthorizations.get(kind);
    const accepted = await pending;
    if (run.workspaceAuthorizations.get(kind) === pending) run.workspaceAuthorizations.delete(kind);
    if (!accepted) throw hostError('host_tool_permission_denied', 403, { retryable: false });
    if (!this.isCurrentRun(run) || run.cancelled || run.state === 'cancelling') throw hostError('host_mcp_scope_unavailable', 403, { retryable: false });
    run.workspaceAllow.add(kind);
    this.subagents.revalidate();
  }
  #syncDesktopAssets(hostProjectId) {
    const project = this.store.project(hostProjectId);
    const desktopProjectId = Number(project.externalId);
    if (!Number.isSafeInteger(desktopProjectId) || desktopProjectId <= 0) return;
    const desktopDbPath = desktopAssetDbPath(this.dataDir);
    const app = this.store.applications().find(item => item.id === 'org.oviraptor.assets');
    const dataDir = app ? this.apps.dataDirectoryFor(app) : null;
    if (!desktopDbPath || !dataDir) return;
    const result = syncDesktopAssets({
      desktopDbPath,
      inventoryDbPath: join(dataDir, 'inventory.sqlite3'),
      hostProjectId: project.id,
      desktopProjectId,
    });
    // A mapped desktop read must not silently return an older mirror when its
    // current snapshot could not be synchronized. App-only projects skip above.
    if (result.reason === 'desktop_sync_failed') throw hostError('host_desktop_asset_sync_failed', 503, { retryable: false });
    if (result.reason === 'desktop_inventory_upgrade_required') throw hostError('host_desktop_inventory_upgrade_required', 409, { retryable: false });
  }
  async cancel(conversationId) {
    const run = this.runs.get(conversationId); if (!run) throw hostError('host_reply_unavailable', 404);
    run.cancelled = true; run.state = 'cancelling';
    this.#revokeRunScope(run);
    this.#cancelAuthorizations(conversationId, run.id);
    const childrenStopped = this.subagents.cancelTree(run.id);
    if (run.sessionId) await this.runtime.cancel(run.sessionId);
    await childrenStopped;
    return { cancelling: true, runId: run.id };
  }
  async decide(id, optionId) {
    const permission = this.permissions.get(String(id));
    if (!permission || !permission.options.some(option => option.id === optionId)) throw hostError('host_permission_choice_invalid');
    const accepted = await this.runtime.resolvePermission(String(id), optionId);
    this.permissions.delete(String(id));
    if (!accepted) throw hostError('host_permission_expired', 409);
    return { accepted: true };
  }
  async mcp(token, conversationId, method, params = {}, { runId } = {}) {
    const { run } = this.validateMcpScope(token, conversationId, { runId });
    if (run.parentRunId && method === 'tools/call' && !this.subagents.allows(run, params.name)) throw hostError('host_child_tool_denied', 403);
    if (method === 'tools/call' && params.name === 'host_task_delegate') return this.subagents.delegate(run, params.arguments, params._hostToolCallId);
    if (method === 'tools/call' && params.name === 'host_task_await') return this.subagents.await(run, params.arguments);
    if (method === 'tools/call' && params.name === 'host_task_cancel') return this.subagents.cancel(run, params.arguments);
    if (method === 'tools/call' && params.name === 'host_task_criteria') return { content: [{ type: 'text', text: JSON.stringify(this.#changeTask(run, current => setTaskCriteria(current, params.arguments, run.id))) }], _meta: { hostEvidence: 'advisory' } };
    if (method === 'tools/call' && params.name === 'host_task_verify') {
      try { return { content: [{ type: 'text', text: JSON.stringify(await verifyTask(this, run, params.arguments)) }], _meta: { hostEvidence: 'advisory' } }; }
      finally { this.emit({ type: 'task.updated', conversationId, taskId: run.taskId, runId: run.id, task: this.task(conversationId, { taskId: run.taskId }) }); }
    }
    if (method === 'tools/call' && TASK_TOOL_SPECS.some(tool => tool.name === params.name)) {
      if (params.name === 'host_task_update' && params.arguments?.complete && [...this.inflightEffects.values()].some(effect => effect.conversationId === conversationId)) throw hostError('host_task_effect_pending', 409);
      if (params.name === 'host_task_update' && params.arguments?.complete) {
        if (run.parentRunId || Object.keys(params.arguments).some(key => !['expectedRevision', 'complete'].includes(key))) throw hostError('host_task_acceptance_required', 409);
        if ([...this.subagents.runs.values()].some(child => child.parentRunId === run.id) || this.store.task(conversationId, run.taskId).children?.some(child => child.status === 'running')) throw hostError('host_child_pending', 409);
        if (params.arguments.expectedRevision !== this.store.task(conversationId, run.taskId).revision) throw hostError('host_task_revision_conflict', 409);
        assertAcceptance(this.store.task(conversationId, run.taskId), run.id);
      }
      const task = params.name === 'host_task_read' ? this.store.task(conversationId, run.taskId)
        : this.#changeTask(run, current => { const next = updateTask(current, params.arguments ?? {}); return params.arguments?.complete ? { ...next, acceptance: { ...current.acceptance, validAtRevision: current.revision + 1 } } : next; });
      return { content: [{ type: 'text', text: JSON.stringify(task) }], _meta: { hostEvidence: 'advisory' } };
    }
    if (method !== 'tools/call') {
      const value = await this.#mcpRequest(token, conversationId, method, params);
      if (run.parentRunId && method === 'tools/list') value.tools = value.tools.filter(tool => this.subagents.allows(run, tool.name));
      return value;
    }
    const conversation = this.store.conversation(conversationId);
    const capability = this.apps.capabilities(conversation.projectId, { runtimeIdentity: this.runtimeIdentity() }).find(tool => tool.name.replace('::', '__') === params.name);
    const readOnly = ['workspace_read', 'workspace_list'].includes(params.name)
      || Boolean(run.skillScope && this.skillAccess?.toolSpecs(conversationId, run.skillScope).some(tool => tool.name === params.name))
      || typeof capability?.permission === 'string' && (capability.permission === 'read' || capability.permission.endsWith('.read'));
    const operationId = randomUUID();
    const digest = argumentDigest(params.arguments);
    const pendingWrite = !readOnly && this.store.task(conversationId, run.taskId)?.evidence?.some(item => item.outcomeUnknown && !item.readOnly && item.toolName === params.name && item.argumentDigest === digest);
    if (pendingWrite) throw hostError('host_operation_unknown', 409, { retryable: false });
    let evidenceId;
    this.#changeTask(run, task => { const started = startTaskEvidence(task, run.id, params.name, params._hostToolCallId, { readOnly, operationId, argumentDigest: digest }); evidenceId = started.evidenceId; return started.task; });
    let admitted = false;
    try {
      this.store.admitToolCall(conversation.projectId, operationId, readOnly ? 'read' : 'write');
      admitted = true;
    } catch (error) {
      this.#changeTask(run, task => ({ ...finishTaskEvidence(task, evidenceId, { ok: false, code: error.code || 'host_budget_hard_stop', outcomeUnknown: false }), ...(error.code === 'host_budget_paused' ? { status: 'paused_budget' } : {}) }));
      throw error;
    }
    const effectId = evidenceId ?? randomUUID();
    let settled;
    const completion = new Promise(resolve => { settled = resolve; });
    this.activeMcpCalls.add(completion);
    if (!readOnly) this.inflightEffects.set(effectId, { conversationId, runId: run.id, taskId: run.taskId });
    const { _hostToolCallId, ...toolParams } = params;
    try {
      const result = await this.#mcpRequest(token, conversationId, method, toolParams, operationId, readOnly);
      const rawBody = scrubbedToolBody(result);
      const rawRecordId = this.store.saveToolRecord({ id: randomUUID(), projectId: conversation.projectId, conversationId, operationId, sha256: createHash('sha256').update(rawBody).digest('hex'), body: rawBody });
      const unknownEffect = !readOnly && (result?.isError === true || result?.structuredContent?.outcomeUnknown === true);
      if (admitted) this.store.settleToolCall(operationId, unknownEffect ? 'indeterminate' : 'consumed');
      this.#changeTask(run, task => finishTaskEvidence(task, evidenceId, { ok: result?.isError !== true && !unknownEffect, summary: JSON.stringify(truncateMcpResult(result, 1600)), outcomeUnknown: unknownEffect, reconciliation: readOnly ? bindReconciliation(task, reconciliationCandidate(result)) : undefined, rawRecordId }));
      if (!run.currentToolResults) run.currentToolResults = new Map();
      run.currentToolResults.set(evidenceId, { name: params.name, result: structuredClone(result) });
      return { ...result, _meta: { ...(result?._meta ?? {}), hostEvidenceId: evidenceId } };
    } catch (error) {
      const knownNotSent = error.details?.outcomeUnknown === false || ['host_tool_permission_denied', 'host_tool_input_invalid', 'host_operation_unknown', 'host_budget_hard_stop', 'host_budget_paused'].includes(error.code);
      if (admitted) this.store.settleToolCall(operationId, knownNotSent ? 'released' : (readOnly ? 'consumed' : 'indeterminate'));
      this.#changeTask(run, task => finishTaskEvidence(task, evidenceId, { ok: false, code: error.code ?? 'host_tool_failed', outcomeUnknown: !readOnly && (error.details?.outcomeUnknown ?? !knownNotSent) }));
      throw error;
    } finally { this.inflightEffects.delete(effectId); this.activeMcpCalls.delete(completion); settled(); }
  }
  async #mcpRequest(token, conversationId, method, params = {}, operationId, readOnly = true) {
    const { scope, run } = this.validateMcpScope(token, conversationId);
    const conversation = this.store.conversation(conversationId);
    if (method === 'initialize') return { protocolVersion: '2025-06-18', capabilities: { tools: { listChanged: true } }, serverInfo: { name: 'oviraptor-apps', version: '1.0.0' } };
    const skillTools = run.skillScope && this.skillAccess ? this.skillAccess.toolSpecs(conversationId, run.skillScope) : [];
    if (method === 'tools/list') return { tools: [...this.apps.capabilities(conversation.projectId, { runtimeIdentity: this.runtimeIdentity() }).map(tool => ({ name: tool.name.replace('::', '__'), description: tool.description, inputSchema: tool.inputSchema })), ...WORKSPACE_TOOL_SPECS, ...TASK_TOOL_SPECS, ...(this.runtimeId === 'xai-api' ? SUBAGENT_TOOL_SPECS : []), ...VERIFIER_TOOL_SPECS, ...skillTools] };
    if (method === 'tools/call') {
      if (String(params.name || '').startsWith('workspace_')) {
        try {
          if (!WORKSPACE_TOOL_SPECS.some(tool => tool.name === params.name)) {
            // Unknown or unsupported tools must not ask for a misleading read grant.
            return await runWorkspaceTool('', params.name, params.arguments ?? {});
          }
          await this.#authorizeWorkspace(conversation, run, params.name);
          const current = this.validateMcpScope(token, conversationId);
          if (current.scope !== scope || current.run !== run) throw hostError('host_mcp_scope_unavailable', 403, { retryable: false });
          const root = join(this.dataDir, 'workspaces', String(conversation.projectId), 'agent');
          const value = await runWorkspaceTool(root, params.name, params.arguments ?? {}, {
            anchor: this.dataDir,
            assertAuthorized: () => {
              const admitted = this.validateMcpScope(token, conversationId);
              if (admitted.scope !== scope || admitted.run !== run) throw hostError('host_mcp_scope_unavailable', 403, { retryable: false });
              if (!run.workspaceAllow?.has(workspacePermission(params.name))) throw hostError('host_tool_permission_denied', 403, { retryable: false });
            },
          });
          const completed = this.validateMcpScope(token, conversationId);
          if (completed.scope !== scope || completed.run !== run) throw hostError('host_mcp_scope_unavailable', 403, { retryable: false });
          return { content: [{ type: 'text', text: JSON.stringify({ ...value, workspace: root }) }] };
        } catch (error) {
          // Only host-owned error codes cross HTTP/MCP. Raw OS errors remain
          // sanitized by the transport rather than disclosing filesystem paths.
          if (/^workspace_[a-z_]+$/u.test(error?.code ?? '')) throw hostError(`host_${error.code}`, 400, { retryable: false });
          throw error;
        }
      }
      if (skillTools.some(tool => tool.name === params.name)) {
        const result = this.skillAccess.execute(conversationId, run.skillScope, params.name, params.arguments ?? {}, { pairedSkillIds: this.#pairedSkillIds(conversation.projectId) });
        if (this.mcpScopes.get(token) !== scope || !this.isCurrentRun(run) || run.cancelled || scope.runtimePid !== this.runtime?.status?.pid) throw hostError('host_mcp_scope_unavailable', 403);
        this.skillAccess.validate(conversationId, run.skillScope);
        return result;
      }
      return this.#mcpAppCall(token, conversationId, scope, run, conversation, params, operationId, readOnly);
    }
    if (method === 'ping') return {};
    throw hostError('host_mcp_method_unavailable');
  }
  #pairedSkillIds(projectId) {
    if (!this.library?.listSkills) return [];
    const byName = new Map(this.library.listSkills().filter(skill => skill.enabled !== false).map(skill => [skill.name, skill.id]));
    const ids = new Set();
    for (const tool of this.apps.capabilities(projectId, { runtimeIdentity: this.runtimeIdentity() })) {
      const app = this.apps.store.application(tool.appId);
      for (const paired of app?.manifest?.paired_skills ?? []) {
        const id = byName.get(paired.name);
        if (id) ids.add(id);
      }
    }
    return [...ids];
  }
  async #mcpAppCall(token, conversationId, scope, run, conversation, params, operationId = randomUUID(), readOnly = true) {
    const resolveTool = () => this.apps.capabilities(conversation.projectId, { runtimeIdentity: this.runtimeIdentity() })
      .find(tool => tool.name.replace('::', '__') === params.name);
    let tool = resolveTool();
    if (!tool) throw hostError('host_tool_permission_denied', 403, { retryable: false });
    if (isDesktopAssetRead(tool.name)) this.#syncDesktopAssets(conversation.projectId);
    let revision = tool.grantRevision;
    let lastError;
    for (let attempt = 0; attempt <= 2; attempt++) {
      try {
        const toolName = tool.name.split('::')[1];
        const result = await this.apps.call(tool.appId, toolName, conversation.projectId, params.arguments ?? {}, revision, operationId);
        if (this.mcpScopes.get(token) !== scope || !this.isCurrentRun(run) || run.cancelled || run.state === 'cancelling' || scope.runtimePid !== this.runtime?.status?.pid) throw hostError('host_mcp_scope_unavailable', 403, { retryable: false });
        if (toolName === 'insight_propose') {
          const structured = result?.structuredContent
            ?? (typeof result?.content?.[0]?.text === 'string' ? (() => { try { return JSON.parse(result.content[0].text); } catch { return null; } })() : null);
          if (structured?.proposalId) {
            this.#emitInsightProposal(structured, { projectId: conversation.projectId, conversationId: conversation.id });
          }
        }
        return truncateMcpResult(result);
      } catch (error) {
        lastError = error;
        const code = typeof error?.code === 'string' ? error.code.split(':')[0] : '';
        if (code === 'host_tool_permission_denied' || code === 'host_tool_authority_changed' || code === 'host_mcp_scope_unavailable') {
          error.retryable = false;
          throw error;
        }
        const retryable = readOnly && (error.retryable === true || RETRYABLE_HOST_CODES.has(code));
        if (!retryable || attempt >= 2) {
          error.retryable = false;
          throw error;
        }
        tool = resolveTool();
        if (!tool) throw hostError('host_tool_permission_denied', 403, { retryable: false });
        revision = tool.grantRevision;
        await new Promise(resolve => setTimeout(resolve, 40 * (attempt + 1)));
      }
    }
    lastError.retryable = false;
    throw lastError;
  }
  validateMcpScope(token, conversationId, { requireActive = true, runId } = {}) {
    const scope = this.mcpScopes.get(token), run = scope?.parentRunId ? this.subagents.runs.get(scope.runId) : this.runs.get(conversationId);
    if (!scope || this.runtimeChanging || scope.conversationId !== conversationId || scope.runtimePid !== this.runtime?.status?.pid || scope.runId && scope.runId !== run?.id || runId !== undefined && runId !== run?.id || (requireActive && (!run || run.cancelled || ['cancelling', 'finishing'].includes(run.state)))) throw hostError('host_mcp_scope_unavailable', 403);
    if (run?.parentRunId) this.subagents.parentActive(run);
    return { scope, run };
  }
  async close() {
    this.#cancelOAuthPoll();
    for (const run of this.runs.values()) await this.cancel(run.conversationId);
    await this.runtime?.close(); await Promise.allSettled([...this.runs.values()].map(run => run.completion));
    await this.apps.close(); await Promise.allSettled([...this.activeMcpCalls]); this.store.close();
    this.library?.close();
  }
}
