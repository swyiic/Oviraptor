import { spawn as spawnProcess } from 'node:child_process';
import { StringDecoder } from 'node:string_decoder';
import { isAbsolute } from 'node:path';
import { randomUUID } from 'node:crypto';
import { performance } from 'node:perf_hooks';

export class AcpRuntimeError extends Error {
  constructor(code, message, details = {}) {
    super(message); this.name = 'AcpRuntimeError'; this.code = code; this.details = details;
  }
}
const fail = (code, message, details) => new AcpRuntimeError(code, message, details);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const idValid = value => typeof value === 'string' || Number.isSafeInteger(value);
const stopReasons = new Set(['end_turn', 'max_tokens', 'max_turn_requests', 'refusal', 'cancelled']);

/** ACP v1 transport only. The official CLI owns authentication, tools and agent reasoning. */
export class GrokAcpRuntime {
  constructor({ cwd = process.cwd(), command = 'grok', args = ['--no-auto-update', 'agent', 'stdio'],
    spawn = spawnProcess, env, onEvent = () => {}, onPermission,
    permissionPolicy = 'reject', requestTimeoutMs = 30_000, promptTimeoutMs = 120_000,
    cancelTimeoutMs = 5_000, permissionTimeoutMs = 30_000, maxLineBytes = 1_048_576,
    now = () => performance.now() } = {}) {
    if (!isAbsolute(cwd)) throw fail('ACP_INVALID_CWD', 'An absolute working directory is required');
    if (!['reject', 'ask'].includes(permissionPolicy)) throw fail('ACP_INVALID_OPTIONS', 'Unknown permission policy');
    for (const value of [requestTimeoutMs, promptTimeoutMs, cancelTimeoutMs, permissionTimeoutMs, maxLineBytes]) {
      if (!Number.isSafeInteger(value) || value < 1) throw fail('ACP_INVALID_OPTIONS', 'Positive integer limits are required');
    }
    Object.assign(this, { cwd, command, args: [...args], spawn, env, onEvent, onPermission, permissionPolicy,
      requestTimeoutMs, promptTimeoutMs, cancelTimeoutMs, permissionTimeoutMs, maxLineBytes, now });
    this.evidence = spawn === spawnProcess && command === 'grok' ? 'official-cli-unverified' : 'synthetic-or-custom';
    this.state = 'idle'; this.protocol = null; this.coldStartMs = null;
    this.authenticationRequired = false;
    this._process = null; this._generation = 0; this._nextId = 0; this._sequence = 0;
    this._pending = new Map(); this._sessions = new Map(); this._queues = new Map();
    this._active = new Map(); this._permissions = new Map(); this._startPromise = null;
    this._callbackErrors = 0; this._stderrBytes = 0;
  }

  get status() {
    return { state: this.state, protocolVersion: this.protocol?.protocolVersion ?? null,
      capabilities: structuredClone(this.protocol?.agentCapabilities ?? {}), coldStartMs: this.coldStartMs,
      pid: this._process?.pid ?? null, pendingRequests: this._pending.size, activeTurns: this._active.size,
      pendingPermissions: this._permissions.size, callbackErrors: this._callbackErrors, stderrBytes: this._stderrBytes,
      ...(this.authenticationRequired ? { authenticationRequired: true } : {}) };
  }

  _emit(event) {
    try {
      const result = this.onEvent({ generation: this._generation, ...event, sequence: ++this._sequence });
      if (result?.catch) result.catch(() => { this._callbackErrors++; });
    } catch { this._callbackErrors++; }
  }

  _assertConnection(generation, child, starting = false) {
    if (generation !== this._generation || this._process !== child || this.state !== (starting ? 'starting' : 'ready')) {
      throw fail('ACP_CONNECTION_CHANGED', 'The ACP connection ended or changed before completion', { outcomeUnknown: true });
    }
  }

  async start() {
    if (this.state === 'ready') return this.status;
    if (this._startPromise) return this._startPromise;
    if (this.state === 'closed') throw fail('ACP_CLOSED', 'This runtime has been closed');
    this._startPromise = this._start().finally(() => { this._startPromise = null; });
    return this._startPromise;
  }

  async _start() {
    this.state = 'starting'; this.protocol = null; this.coldStartMs = null; this.authenticationRequired = false;
    const started = this.now(), generation = ++this._generation;
    let child;
    try {
      child = this.spawn(this.command, this.args, { cwd: this.cwd, ...(this.env ? { env: this.env } : {}),
        shell: false, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
      if (!child?.stdin?.write || !child?.stdout?.on) throw Error('Missing stdio');
    } catch {
      const error = fail('ACP_SPAWN_FAILED', 'The configured ACP process could not be started');
      this._fatal(error, generation); throw error;
    }
    this._process = child;
    const decoder = new StringDecoder('utf8'); let buffer = '';
    child.stdout.on('data', chunk => {
      if (generation !== this._generation || this.state === 'failed' || this.state === 'closed') return;
      buffer += decoder.write(Buffer.from(chunk));
      for (;;) {
        if (generation !== this._generation) return;
        const end = buffer.indexOf('\n');
        if (end < 0) break;
        const line = buffer.slice(0, end).replace(/\r$/, ''); buffer = buffer.slice(end + 1);
        if (!line || Buffer.byteLength(line) > this.maxLineBytes) {
          this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid or oversized NDJSON frame'), generation); return;
        }
        try { this._receiveLine(line, generation); }
        catch (error) { this._fatal(error instanceof AcpRuntimeError ? error : fail('ACP_PROTOCOL_ERROR', 'Invalid ACP message'), generation); }
        if (this.state === 'failed' || this.state === 'closed') return;
      }
      if (Buffer.byteLength(buffer) > this.maxLineBytes) this._fatal(fail('ACP_PROTOCOL_ERROR', 'NDJSON frame exceeded the configured limit'), generation);
    });
    child.stdout.on('end', () => {
      if (generation !== this._generation || this.state === 'closed' || this.state === 'failed') return;
      this._fatal(fail(buffer ? 'ACP_PROTOCOL_ERROR' : 'ACP_PROCESS_EXIT', buffer ? 'ACP stdout ended with an incomplete frame' : 'ACP stdout closed unexpectedly'), generation);
    });
    // Stderr is deliberately not stored, forwarded or included in errors: CLIs can print credentials.
    child.stderr?.on('data', chunk => { if (generation === this._generation) this._stderrBytes += Buffer.byteLength(chunk); });
    child.stdin.on?.('error', () => this._fatal(fail('ACP_IO_ERROR', 'ACP stdin failed'), generation));
    child.on('error', () => this._fatal(fail('ACP_SPAWN_FAILED', 'The configured ACP process failed'), generation));
    child.on('exit', (code, signal) => {
      if (generation === this._generation && this.state !== 'closed' && this.state !== 'failed') {
        this._fatal(fail('ACP_PROCESS_EXIT', 'ACP process exited unexpectedly', { exitCode: code, signal, outcomeUnknown: true }), generation);
      }
    });
    try {
      const init = await this._rpc('initialize', { protocolVersion: 1,
        clientInfo: { name: 'Oviraptor', version: '0.1.0' },
        clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false } });
      this._assertConnection(generation, child, true);
      if (!object(init) || init.protocolVersion !== 1 || (init.agentCapabilities !== undefined && !object(init.agentCapabilities)) || !Array.isArray(init.authMethods ?? [])) {
        throw fail('ACP_VERSION_UNSUPPORTED', 'The agent did not negotiate ACP protocol version 1');
      }
      if (!(init.authMethods ?? []).some(method => method?.id === 'cached_token')) {
        const loginRequired = init.authMethods?.length > 0 && init.authMethods.every(method => object(method) && ['grok.com', 'oidc'].includes(method.id));
        throw fail('ACP_CACHED_AUTH_UNAVAILABLE', 'The official CLI must advertise cached_token; authenticate it locally before connecting',
          loginRequired ? { authenticationRequired: true, outcomeUnknown: false } : {});
      }
      this.protocol = structuredClone(init);
      await this._rpc('authenticate', { methodId: 'cached_token', _meta: { headless: true } });
      this._assertConnection(generation, child, true);
      this.state = 'ready'; this.coldStartMs = this.now() - started;
      this._emit({ type: 'runtime_ready', protocolVersion: 1, capabilities: structuredClone(init.agentCapabilities ?? {}), metrics: { coldStartMs: this.coldStartMs } });
      this._assertConnection(generation, child);
      return this.status;
    } catch (error) { this._fatal(error, generation); throw error; }
  }

  _send(message) {
    if (!this._process || !['starting', 'ready'].includes(this.state)) throw fail('ACP_NOT_RUNNING', 'The ACP process is not running');
    const data = JSON.stringify(message) + '\n';
    if (Buffer.byteLength(data) > this.maxLineBytes) throw fail('ACP_REQUEST_TOO_LARGE', 'The request exceeded the frame limit');
    const generation = this._generation;
    try { this._process.stdin.write(data, error => { if (error) this._fatal(fail('ACP_IO_ERROR', 'ACP stdin write failed'), generation); }); }
    catch { const error = fail('ACP_IO_ERROR', 'ACP stdin write failed'); this._fatal(error, generation); throw error; }
  }

  _rpc(method, params, { timeoutMs = this.requestTimeoutMs, onTimeout } = {}) {
    const id = ++this._nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (!this._pending.has(id)) return;
        if (onTimeout) { onTimeout(); return; }
        this._fatal(fail('ACP_REQUEST_TIMEOUT', 'An ACP request timed out', { method, outcomeUnknown: true }), this._generation);
      }, timeoutMs);
      this._pending.set(id, { resolve, reject, timer, method });
      try { this._send({ jsonrpc: '2.0', id, method, params }); }
      catch (error) { clearTimeout(timer); this._pending.delete(id); reject(error); }
    });
  }

  _receiveLine(line, generation) {
    if (generation !== this._generation || ['closed', 'failed'].includes(this.state)) return;
    let message;
    try { message = JSON.parse(line); }
    catch { this._fatal(fail('ACP_PROTOCOL_ERROR', 'ACP stdout contained invalid JSON'), generation); return; }
    if (!object(message) || message.jsonrpc !== '2.0') { this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid JSON-RPC envelope'), generation); return; }
    if (typeof message.method === 'string') {
      if (Object.hasOwn(message, 'id')) {
        if (!idValid(message.id)) { this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid agent request identifier'), generation); return; }
        if (message.method === 'session/request_permission') this._permission(message, generation);
        else this._send({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'Client method not supported' } });
      } else if (message.method === 'session/update') this._update(message.params, generation);
      else this._emit({ type: 'notification', method: message.method });
      return;
    }
    if (!idValid(message.id) || Object.hasOwn(message, 'result') === Object.hasOwn(message, 'error')) {
      this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid JSON-RPC response'), generation); return;
    }
    const pending = this._pending.get(message.id);
    if (!pending) { this._fatal(fail('ACP_PROTOCOL_ERROR', 'Unexpected or duplicate JSON-RPC response identifier'), generation); return; }
    clearTimeout(pending.timer); this._pending.delete(message.id);
    if (Object.hasOwn(message, 'error')) {
      if (!object(message.error) || !Number.isInteger(message.error.code) || typeof message.error.message !== 'string') {
        pending.reject(fail('ACP_PROTOCOL_ERROR', 'Invalid agent error response'));
        this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid agent error response'), generation); return;
      }
      // Raw agent error messages/data may contain credentials or request content.
      pending.reject(fail('ACP_REMOTE_ERROR', 'The ACP agent rejected the request', { method: pending.method, rpcCode: message.error.code }));
    } else pending.resolve(message.result);
  }

  _update(params, generation) {
    if (generation !== this._generation || ['closed', 'failed'].includes(this.state)) return;
    if (!object(params) || typeof params.sessionId !== 'string' || !object(params.update) || typeof params.update.sessionUpdate !== 'string') {
      this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid session update'), generation); return;
    }
    const { sessionId, update } = params, turn = this._active.get(sessionId);
    const event = { sessionId, turnId: turn?.id ?? null, replay: this._sessions.get(sessionId)?.loading === true, update: structuredClone(update) };
    if (update.sessionUpdate === 'agent_message_chunk' && update.content?.type === 'text' && typeof update.content.text === 'string') {
      if (turn && turn.firstTextMs === null) turn.firstTextMs = this.now() - turn.started;
      this._emit({ ...event, type: 'text_delta', text: update.content.text });
    } else if (['tool_call', 'tool_call_update'].includes(update.sessionUpdate)) {
      if (turn && turn.firstToolMs === null) turn.firstToolMs = this.now() - turn.started;
      this._emit({ ...event, type: update.sessionUpdate === 'tool_call' ? 'tool_call' : 'tool_update' });
    } else this._emit({ ...event, type: 'session_update' });
  }

  _permission(message, generation) {
    if (generation !== this._generation || ['closed', 'failed'].includes(this.state)) return;
    const params = message.params;
    if (!object(params) || typeof params.sessionId !== 'string' || !object(params.toolCall) || typeof params.toolCall.toolCallId !== 'string' || !Array.isArray(params.options) || params.options.some(option => !object(option) || typeof option.optionId !== 'string' || typeof option.name !== 'string' || !['allow_once', 'allow_always', 'reject_once', 'reject_always'].includes(option.kind)) || new Set(params.options.map(option => option.optionId)).size !== params.options.length) {
      this._fatal(fail('ACP_PROTOCOL_ERROR', 'Invalid permission request'), generation); return;
    }
    const key = `${generation}:${typeof message.id}:${message.id}`;
    if (this._permissions.has(key)) { this._fatal(fail('ACP_PROTOCOL_ERROR', 'Duplicate permission request'), generation); return; }
    const turn = this._active.get(params.sessionId);
    const entry = { key, rpcId: message.id, generation, sessionId: params.sessionId, params: structuredClone(params), turnId: turn?.id ?? null };
    this._permissions.set(key, entry);
    entry.timer = setTimeout(() => this.resolvePermission(key, null), this.permissionTimeoutMs);
    this._emit({ type: 'permission_requested', requestId: key, sessionId: params.sessionId, turnId: entry.turnId, toolCall: structuredClone(params.toolCall), options: structuredClone(params.options) });
    if (!this._permissions.has(key)) return;
    if (!turn || turn.cancelRequested) { this._finishPermission(entry, { outcome: 'cancelled' }); return; }
    if (this.onPermission) {
      Promise.resolve().then(() => this.onPermission({ requestId: key, ...structuredClone(params) }))
        .then(decision => this.resolvePermission(key, typeof decision === 'string' ? decision : decision?.optionId ?? null))
        .catch(() => this.resolvePermission(key, null));
    } else if (this.permissionPolicy === 'reject') this.resolvePermission(key, null);
  }

  resolvePermission(requestId, optionId) {
    const entry = this._permissions.get(requestId);
    if (!entry) return false;
    const turn = this._active.get(entry.sessionId);
    if (!turn || turn.cancelRequested || entry.generation !== this._generation) return this._finishPermission(entry, { outcome: 'cancelled' });
    const selected = entry.params.options.find(option => option.optionId === optionId);
    const rejected = entry.params.options.find(option => option.kind === 'reject_once') ?? entry.params.options.find(option => option.kind === 'reject_always');
    const choice = selected ?? rejected;
    return this._finishPermission(entry, choice ? { outcome: 'selected', optionId: choice.optionId } : { outcome: 'cancelled' });
  }

  _finishPermission(entry, outcome) {
    if (!this._permissions.delete(entry.key)) return false;
    clearTimeout(entry.timer);
    if (entry.generation === this._generation && ['starting', 'ready'].includes(this.state)) {
      try { this._send({ jsonrpc: '2.0', id: entry.rpcId, result: { outcome } }); }
      catch (error) { this._fatal(error, entry.generation); }
    }
    this._emit({ type: 'permission_resolved', generation: entry.generation, requestId: entry.key, sessionId: entry.sessionId, turnId: entry.turnId, outcome });
    return true;
  }

  _scope({ cwd = this.cwd, mcpServers = [] } = {}) {
    if (!isAbsolute(cwd) || !Array.isArray(mcpServers)) throw fail('ACP_INVALID_SESSION', 'An absolute cwd and MCP server array are required');
    return { cwd, mcpServers: structuredClone(mcpServers) };
  }

  async newSession(options = {}) {
    const scope = this._scope(options); await this.start();
    const generation = this._generation, child = this._process;
    const result = await this._rpc('session/new', scope);
    this._assertConnection(generation, child);
    if (!object(result) || typeof result.sessionId !== 'string' || !result.sessionId || this._sessions.has(result.sessionId)) {
      const error = fail('ACP_PROTOCOL_ERROR', 'Agent returned an invalid or duplicate session'); this._fatal(error, generation); throw error;
    }
    this._sessions.set(result.sessionId, { cwd: scope.cwd, generation, loading: false });
    this._emit({ type: 'session_ready', sessionId: result.sessionId, restored: false });
    return structuredClone(result);
  }

  async loadSession({ sessionId, ...options }) {
    if (typeof sessionId !== 'string' || !sessionId) throw fail('ACP_INVALID_SESSION', 'A session identifier is required');
    const scope = this._scope(options), previous = this._sessions.get(sessionId);
    if (previous && previous.cwd !== scope.cwd) throw fail('ACP_SCOPE_MISMATCH', 'The saved session belongs to a different working directory');
    await this.start();
    const generation = this._generation, child = this._process;
    if (this.protocol.agentCapabilities?.loadSession !== true) throw fail('ACP_LOAD_UNSUPPORTED', 'The agent did not advertise session/load support');
    return this._serial(sessionId, async () => {
      this._assertConnection(generation, child);
      const current = this._sessions.get(sessionId);
      if (current && current.cwd !== scope.cwd) throw fail('ACP_SCOPE_MISMATCH', 'The saved session belongs to a different working directory');
      const loading = { cwd: scope.cwd, generation: 0, loading: true };
      this._sessions.set(sessionId, loading);
      try {
        const result = await this._rpc('session/load', { sessionId, ...scope });
        this._assertConnection(generation, child);
        if (!object(result)) { const error = fail('ACP_PROTOCOL_ERROR', 'Invalid session load response'); this._fatal(error, generation); throw error; }
        this._sessions.set(sessionId, { cwd: scope.cwd, generation, loading: false });
        this._emit({ type: 'session_ready', sessionId, restored: true }); return { ...result, sessionId };
      } catch (error) { if (this._sessions.get(sessionId) === loading) this._sessions.set(sessionId, { cwd: scope.cwd, generation: 0, loading: false }); throw error; }
    });
  }

  _serial(sessionId, action) {
    const previous = this._queues.get(sessionId) ?? Promise.resolve();
    const promise = previous.catch(() => {}).then(action);
    this._queues.set(sessionId, promise);
    promise.finally(() => { if (this._queues.get(sessionId) === promise) this._queues.delete(sessionId); }).catch(() => {});
    return promise;
  }

  async prompt({ sessionId, prompt, signal, turnId = randomUUID(), timeoutMs = this.promptTimeoutMs }) {
    if (typeof sessionId !== 'string' || !sessionId || typeof turnId !== 'string' || !turnId || (signal && !(signal instanceof AbortSignal))) throw fail('ACP_INVALID_OPTIONS', 'A session ID, turn ID and optional AbortSignal are required');
    const content = typeof prompt === 'string' ? [{ type: 'text', text: prompt }] : structuredClone(prompt);
    if (!Array.isArray(content) || !content.length || content.some(block => !object(block) || block.type !== 'text' || typeof block.text !== 'string')) return Promise.reject(fail('ACP_INVALID_PROMPT', 'This adapter accepts text prompt blocks'));
    if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1) return Promise.reject(fail('ACP_INVALID_OPTIONS', 'A positive prompt timeout is required'));
    return this._serial(sessionId, () => this._prompt({ sessionId, prompt: content, signal, turnId, timeoutMs }));
  }

  async _prompt({ sessionId, prompt, signal, turnId, timeoutMs }) {
    if (signal?.aborted) throw fail('ACP_ABORTED', 'The queued prompt was cancelled before sending');
    const session = this._sessions.get(sessionId);
    if (this.state !== 'ready' || !session || session.generation !== this._generation) throw fail('ACP_SESSION_NEEDS_LOAD', 'Create or explicitly load this session before prompting; prompts are never replayed');
    const generation = this._generation, child = this._process;
    const turn = { id: turnId, started: this.now(), firstTextMs: null, firstToolMs: null, cancelRequested: false, timedOut: false };
    this._active.set(sessionId, turn);
    const abort = () => this.cancel(sessionId);
    signal?.addEventListener('abort', abort, { once: true });
    try {
      const response = this._rpc('session/prompt', { sessionId, prompt }, { timeoutMs, onTimeout: () => { turn.timedOut = true; this.cancel(sessionId); } });
      this._emit({ type: 'turn_started', sessionId, turnId, generation });
      const result = await response;
      this._assertConnection(generation, child);
      if (!object(result) || !stopReasons.has(result.stopReason)) throw fail('ACP_PROTOCOL_ERROR', 'Agent returned an invalid prompt stop reason');
      const metrics = { coldStartMs: this.coldStartMs, firstTextMs: turn.firstTextMs, firstToolMs: turn.firstToolMs, totalMs: this.now() - turn.started };
      if (turn.timedOut) throw fail('ACP_PROMPT_TIMEOUT', 'The prompt deadline expired; cancellation was acknowledged', { sessionId, turnId, metrics, stopReason: result.stopReason, outcomeUnknown: result.stopReason !== 'cancelled' });
      this._emit({ type: 'turn_complete', sessionId, turnId, generation, stopReason: result.stopReason, metrics });
      return { sessionId, turnId, stopReason: result.stopReason, metrics };
    } catch (error) {
      if (error.code === 'ACP_PROTOCOL_ERROR') this._fatal(error, generation);
      this._emit({ type: 'turn_error', sessionId, turnId, generation, code: error.code ?? 'ACP_ERROR', outcomeUnknown: error.details?.outcomeUnknown ?? true,
        metrics: { coldStartMs: this.coldStartMs, firstTextMs: turn.firstTextMs, firstToolMs: turn.firstToolMs, totalMs: this.now() - turn.started } });
      throw error;
    } finally {
      clearTimeout(turn.cancelTimer); signal?.removeEventListener('abort', abort);
      for (const permission of [...this._permissions.values()]) if (permission.sessionId === sessionId && permission.turnId === turn.id) this._finishPermission(permission, { outcome: 'cancelled' });
      if (this._active.get(sessionId) === turn) this._active.delete(sessionId);
    }
  }

  cancel(sessionId) {
    const turn = this._active.get(sessionId);
    if (!turn || turn.cancelRequested) return false;
    const generation = this._generation, child = this._process;
    turn.cancelRequested = true;
    for (const permission of [...this._permissions.values()]) if (permission.sessionId === sessionId) this._finishPermission(permission, { outcome: 'cancelled' });
    try { this._assertConnection(generation, child); this._send({ jsonrpc: '2.0', method: 'session/cancel', params: { sessionId } }); }
    catch (error) { this._fatal(error, generation); return false; }
    turn.cancelTimer = setTimeout(() => this._fatal(fail('ACP_CANCEL_TIMEOUT', 'The agent did not acknowledge cancellation; the process was terminated', { outcomeUnknown: true }), generation), this.cancelTimeoutMs);
    this._emit({ type: 'cancel_requested', sessionId, turnId: turn.id }); return true;
  }

  _fatal(error, generation) {
    if (generation !== this._generation || this.state === 'closed' || this.state === 'failed') return;
    const child = this._process;
    this.authenticationRequired = this.state === 'starting' && error.code === 'ACP_CACHED_AUTH_UNAVAILABLE' && error.details?.authenticationRequired === true && error.details?.outcomeUnknown === false;
    this.state = 'failed'; this._process = null;
    for (const pending of this._pending.values()) { clearTimeout(pending.timer); pending.reject(error); }
    this._pending.clear();
    for (const turn of this._active.values()) clearTimeout(turn.cancelTimer);
    try { child?.kill('SIGKILL'); } catch { /* The process is already quarantined. */ }
    for (const entry of [...this._permissions.values()]) this._finishPermission(entry, { outcome: 'cancelled' });
    this._emit({ type: 'runtime_error', generation, code: error.code ?? 'ACP_ERROR', outcomeUnknown: error.details?.outcomeUnknown ?? true,
      ...(this.authenticationRequired ? { authenticationRequired: true } : {}) });
  }

  async close() {
    if (this.state === 'closed') return;
    for (const entry of [...this._permissions.values()]) this._finishPermission(entry, { outcome: 'cancelled' });
    const child = this._process;
    this.state = 'closed'; this._process = null;
    for (const pending of this._pending.values()) { clearTimeout(pending.timer); pending.reject(fail('ACP_CLOSED', 'The runtime was closed', { outcomeUnknown: true })); }
    this._pending.clear();
    for (const turn of this._active.values()) clearTimeout(turn.cancelTimer);
    if (child) {
      await new Promise(resolve => {
        let timer; const done = () => { clearTimeout(timer); resolve(); };
        child.once('exit', done); timer = setTimeout(() => { child.kill('SIGKILL'); done(); }, 250);
        child.kill('SIGTERM');
      });
    }
    this._emit({ type: 'runtime_closed' });
  }
}
