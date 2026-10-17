import { proxyFetch } from './net-proxy.mjs';
import { randomUUID, randomBytes } from 'node:crypto';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { HostStore, hostError, boundedText, RETRYABLE_HOST_CODES } from './store.mjs';
import { AppManager } from './apps.mjs';
import { ensureBundledApps, bundledAppsStatus } from './bundled-apps.mjs';
import { shellCatalog as buildShellCatalog } from './shell-apps.mjs';
import { buildDiscoverySummary, formatDiscoveryPromptBlock } from './discovery.mjs';
import { formatMemoryPromptBlock, readConversationMemory, recordTurn, summarizeToolEvent, pinFact } from './memory.mjs';
import { ProviderStore, OpenAiCompatibleRuntime, truncateMcpResult } from './provider.mjs';
import { buildTurnPrompt, humanizeToolTitle, normalizeAnswerMode, answerModeOptions, prefersResponsesApi } from './quality-prompt.mjs';
import {
  requestDeviceCode, pollDeviceCodeOnce, pollDeviceCodeUntilDone, normalizeOAuthTokens,
  publicDevicePending, refreshAccessToken, accessTokenIsExpiring, XAI_OAUTH,
} from './xai-oauth.mjs';
import { buildPairedSkillsPreview, importPairedSkills as importPairedSkillsAction } from './paired-skills.mjs';

const RUNTIME_IDS = Object.freeze(['none', 'grok-acp', 'xai-api']);
const AUTHENTICATION_REQUIRED_MESSAGE = 'Official Grok authentication is required. Sign in separately using your approved private Grok profile, then explicitly retry. No login or message resend was started.';
const authenticationRequired = (code, details) => code === 'ACP_CACHED_AUTH_UNAVAILABLE' && details?.authenticationRequired === true && details?.outcomeUnknown === false;
const authenticationNotice = state => state === 'authentication_required' ? { authenticationRequired: true, message: AUTHENTICATION_REQUIRED_MESSAGE } : {};

/** Conversation orchestration only. Model intelligence stays in the selected runtime. */
export class AgentHost {
  constructor({ dataDir, runtime = null, runtimeFactory = null, runtimeSelection, store, apps, onEvent = () => {}, providerFetch }) {
    if (runtimeSelection !== undefined && !RUNTIME_IDS.includes(runtimeSelection)) throw hostError('host_runtime_invalid');
    if (runtimeSelection === 'grok-acp' && !runtime && !runtimeFactory) throw hostError('host_runtime_unavailable');
    this.dataDir = dataDir; this.store = store ?? new HostStore(join(dataDir, 'host.sqlite'));
    this.apps = apps ?? new AppManager(this.store, join(dataDir, 'applications'));
    this.runtimeFactory = runtimeFactory; this.providerStore = new ProviderStore(dataDir); this.providerFetch = providerFetch;
    this.oauthFetch = providerFetch; // injectable for OAuth HTTP in tests
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
    this.onEvent = onEvent; this.runs = new Map(); this.permissions = new Map(); this.mcpScopes = new Map(); this.loadedSessions = new Map();
    this.apps.onActivity = event => this.emit({ type: 'application.activity', appId: event.appId, ...(typeof event.busy === 'boolean' ? { busy: event.busy } : {}) });
    this.runtimeRevision = Number(this.store.db.prepare("SELECT value FROM host_settings WHERE key='runtime_revision'").get()?.value ?? 1);
    this.runtimeState = this.runtime ? 'configured' : 'unconfigured';
    this.runtimeEvidence = this.runtime?.evidence ?? (this.runtime?.command === 'grok' ? 'official-cli-unverified' : this.runtime ? 'synthetic-or-custom' : 'none');
  }
  runtimeIdentity() { return `${this.runtime ? this.runtimeId : 'none'}:${this.runtimeRevision}`; }
  #providerRuntime() {
    const config = this.providerStore.read();
    const bearer = this.providerStore.bearer();
    if (!bearer || !config) return null;
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
  async setProvider(input) {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    const view = this.providerStore.save(input ?? {});
    if (this.runtimeId === 'xai-api') await this.setRuntime('xai-api', { force: true });
    return { ...this.settings(), apiProvider: view };
  }
  async clearProvider() {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    this.#cancelOAuthPoll();
    if (this.runtimeId === 'xai-api') await this.setRuntime('none');
    this.providerStore.clear(); return this.settings();
  }
  #cancelOAuthPoll() {
    this.oauthAbort?.abort();
    this.oauthAbort = null;
    this.oauthPoll = null;
  }
  /** Start RFC 8628 device-code login; host background-polls and stores tokens on success. */
  async startProviderOAuth({ referrer = XAI_OAUTH.REFERRER, background = true } = {}) {
    if (this.runs.size || this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    this.#cancelOAuthPoll();
    const fetchImpl = this.oauthFetch ?? proxyFetch;
    const device = await requestDeviceCode({ fetch: fetchImpl, referrer });
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
    const result = await pollDeviceCodeOnce(pending.device_code, { fetch: fetchImpl });
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
    const view = this.providerStore.publicView();
    if (view.oauthConfigured || view.apiKeyConfigured) {
      if (this.runtimeId === 'xai-api') await this.setRuntime('none');
      this.providerStore.clear();
    }
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
    };
  }
  async setRuntime(id, { force = false } = {}) {
    if (!RUNTIME_IDS.includes(id)) throw hostError('host_runtime_invalid');
    if (this.runs.size) throw hostError('host_runtime_busy', 409);
    if (this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    if (!force && (this.runtime ? this.runtimeId : 'none') === id) return this.settings();
    if (id === 'grok-acp' && !this.runtimeFactory) throw hostError('host_runtime_unavailable');
    if (id === 'xai-api' && !this.providerStore.bearer()) throw hostError('host_provider_api_key_required');
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
  emit(event) { try { this.onEvent({ ...event, timestamp: new Date().toISOString() }); } catch { this.observerErrors = (this.observerErrors ?? 0) + 1; } }
  status() { return { protocolVersion: 1, runtime: { state: this.runtimeState, id: this.runtime ? this.runtimeId : null, identity: this.runtimeIdentity(), evidence: this.runtimeEvidence, ...authenticationNotice(this.runtimeState) },
    activeRuns: [...this.runs.values()].map(({ id, conversationId, state }) => ({ id, conversationId, state })), permissions: [...this.permissions.values()] }; }
  setOrigin(origin) { this.origin = origin; }
  async send(conversationId, input) {
    if (this.runtimeChanging) throw hostError('host_runtime_busy', 409);
    const conversation = this.store.conversation(conversationId, input.projectId);
    if (this.runs.has(conversationId)) throw hostError('host_reply_already_running', 409);
    const text = boundedText(input.text);
    const userMessage = this.store.addMessage(conversationId, 'user', text);
    if (!this.runtime) return { userMessage, runtime: { state: 'unconfigured' }, savedOnly: true };
    const run = { id: randomUUID(), conversationId, sessionId: null, state: 'starting', text: '', started: performance.now() };
    run.message = this.store.addMessage(conversationId, 'assistant', '', 'running', run.id);
    this.runs.set(conversationId, run);
    this.emit({ type: 'applications.changed', projectId: conversation.projectId });
    // The HTTP response does not claim the model turn completed.
    run.completion = this.#run(conversation, text, run).catch(() => {});
    return { userMessage, run: { id: run.id, conversationId, state: run.state } };
  }
  async #run(conversation, text, run) {
    let status = 'failed';
    try {
      const skills = await this.ensureSkills();
      if (run.cancelled) throw hostError('host_reply_cancelled');
      run.skillScope = skills.capture(conversation.id);
      await this.runtime.start(); this.runtimeState = 'ready';
      if (run.cancelled) throw hostError('host_reply_cancelled');
      skills.validate(conversation.id, run.skillScope);
      const cwd = join(this.dataDir, 'workspaces', String(conversation.projectId));
      await mkdir(cwd, { recursive: true, mode: 0o700 });
      const previousScope = [...this.mcpScopes].find(([, scope]) => scope.conversationId === conversation.id);
      let scopeToken = previousScope?.[0];
      if (previousScope && previousScope[1].runtimePid !== this.runtime.status.pid) { this.mcpScopes.delete(scopeToken); scopeToken = null; }
      if (!scopeToken) { scopeToken = randomBytes(32).toString('hex'); this.mcpScopes.set(scopeToken, { conversationId: conversation.id, runtimePid: this.runtime.status.pid }); }
      const mcpServers = this.origin ? [{ name: 'oviraptor-apps', command: process.execPath,
        args: [fileURLToPath(new URL('./mcp-bridge.mjs', import.meta.url))], env: [
          { name: 'OVIRAPTOR_HOST_ORIGIN', value: this.origin }, { name: 'OVIRAPTOR_MCP_TOKEN', value: scopeToken },
          { name: 'OVIRAPTOR_CONVERSATION', value: conversation.id },
        ] }] : [];
      if (conversation.sessionId) {
        if (this.loadedSessions.get(conversation.sessionId) !== this.runtime.status.pid) await this.runtime.loadSession({ sessionId: conversation.sessionId, cwd, mcpServers });
        run.sessionId = conversation.sessionId;
      } else {
        const result = await this.runtime.newSession({ cwd, mcpServers });
        run.sessionId = typeof result === 'string' ? result : result.sessionId;
        if (!run.sessionId) throw hostError('host_runtime_session_invalid');
        this.store.session(conversation.id, run.sessionId);
      }
      this.loadedSessions.set(run.sessionId, this.runtime.status.pid);
      if (run.cancelled) throw hostError('host_reply_cancelled');
      run.state = 'running'; this.emit({ type: 'runtime.status', conversationId: conversation.id, runId: run.id, state: 'running' });
      const project = this.store.project(conversation.projectId);
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
      const prompt = buildTurnPrompt({
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
      const result = await this.runtime.prompt({ sessionId: run.sessionId, prompt, userText: text, turnId: run.id, answerMode });
      skills.validate(conversation.id, run.skillScope);
      run.metrics = result.metrics;
      run.stopReason = result.stopReason; run.outcomeUnknown = false;
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
      const disposition = { outcomeUnknown: Boolean(run.outcomeUnknown), failureCode: run.failureCode ?? '', stopReason: run.stopReason ?? '', cancelRequested: Boolean(run.cancelled),
        ...(run.authenticationRequired ? { authenticationRequired: true } : {}) };
      this.store.finishMessage(run.message.id, run.text, status, disposition);
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
  async cancel(conversationId) {
    const run = this.runs.get(conversationId); if (!run) throw hostError('host_reply_unavailable', 404);
    run.cancelled = true; run.state = 'cancelling';
    if (run.sessionId) await this.runtime.cancel(run.sessionId);
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
  async mcp(token, conversationId, method, params = {}) {
    const { scope, run } = this.validateMcpScope(token, conversationId);
    const conversation = this.store.conversation(conversationId);
    if (method === 'initialize') return { protocolVersion: '2025-06-18', capabilities: { tools: { listChanged: true } }, serverInfo: { name: 'oviraptor-apps', version: '1.0.0' } };
    const skillTools = run.skillScope && this.skillAccess ? this.skillAccess.toolSpecs(conversationId, run.skillScope) : [];
    if (method === 'tools/list') return { tools: [...this.apps.capabilities(conversation.projectId, { runtimeIdentity: this.runtimeIdentity() }).map(tool => ({ name: tool.name.replace('::', '__'), description: tool.description, inputSchema: tool.inputSchema })), ...skillTools] };
    if (method === 'tools/call') {
      if (skillTools.some(tool => tool.name === params.name)) {
        const result = this.skillAccess.execute(conversationId, run.skillScope, params.name, params.arguments ?? {}, { pairedSkillIds: this.#pairedSkillIds(conversation.projectId) });
        if (this.mcpScopes.get(token) !== scope || this.runs.get(conversationId) !== run || run.cancelled || scope.runtimePid !== this.runtime?.status?.pid) throw hostError('host_mcp_scope_unavailable', 403);
        this.skillAccess.validate(conversationId, run.skillScope);
        return result;
      }
      return this.#mcpAppCall(token, conversationId, scope, run, conversation, params);
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
  async #mcpAppCall(token, conversationId, scope, run, conversation, params) {
    const resolveTool = () => this.apps.capabilities(conversation.projectId, { runtimeIdentity: this.runtimeIdentity() })
      .find(tool => tool.name.replace('::', '__') === params.name);
    let tool = resolveTool();
    if (!tool) throw hostError('host_tool_permission_denied', 403, { retryable: false });
    let revision = tool.grantRevision;
    let lastError;
    for (let attempt = 0; attempt <= 2; attempt++) {
      try {
        const result = await this.apps.call(tool.appId, tool.name.split('::')[1], conversation.projectId, params.arguments ?? {}, revision);
        if (this.mcpScopes.get(token) !== scope || this.runs.get(conversationId) !== run || run.cancelled || run.state === 'cancelling' || scope.runtimePid !== this.runtime?.status?.pid) throw hostError('host_mcp_scope_unavailable', 403, { retryable: false });
        return truncateMcpResult(result);
      } catch (error) {
        lastError = error;
        const code = typeof error?.code === 'string' ? error.code.split(':')[0] : '';
        if (code === 'host_tool_permission_denied' || code === 'host_tool_authority_changed' || code === 'host_mcp_scope_unavailable') {
          error.retryable = false;
          throw error;
        }
        const retryable = error.retryable === true || RETRYABLE_HOST_CODES.has(code);
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
  validateMcpScope(token, conversationId, { requireActive = true } = {}) {
    const scope = this.mcpScopes.get(token), run = this.runs.get(conversationId);
    if (!scope || this.runtimeChanging || scope.conversationId !== conversationId || scope.runtimePid !== this.runtime?.status?.pid || (requireActive && (!run || run.cancelled || run.state === 'cancelling'))) throw hostError('host_mcp_scope_unavailable', 403);
    return { scope, run };
  }
  async close() {
    this.#cancelOAuthPoll();
    for (const run of this.runs.values()) await this.cancel(run.conversationId);
    await this.runtime?.close(); await Promise.allSettled([...this.runs.values()].map(run => run.completion));
    await this.apps.close(); this.store.close();
    this.library?.close();
  }
}
