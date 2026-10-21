import { spawn as nodeSpawn } from 'node:child_process';
import { join } from 'node:path';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { hostError } from './store.mjs';

// Only a validated response from the installed native subprocess can mint this
// provenance. Ordinary throws and JSON/property copies cannot forge it.
const nativeApplicationErrors = new WeakMap();
export const readNativeApplicationErrorCode = error => error !== null && (typeof error === 'object' || typeof error === 'function') ? nativeApplicationErrors.get(error) : undefined;
function applicationError(native, response) {
  const code = response.data?.code;
  if (!native || typeof code !== 'string' || !/^[A-Za-z0-9_]{1,96}$/u.test(code)) return hostError('host_app_tool_rejected');
  const error = hostError(code); nativeApplicationErrors.set(error, code); return error;
}

/** One app worker process; it is not tied to a page or chat connection. */
export class AppProcess {
  constructor(app, dataDir, { spawn = nodeSpawn, timeoutMs = 15000, onActivity = () => {}, onOwnerState = () => {} } = {}) {
    this.app = app; this.dataDir = dataDir; this.spawn = spawn; this.timeoutMs = timeoutMs;
    this.native = app.manifest.schemaVersion === 2;
    this.nativeBusy = this.native ? true : undefined;
    this.onActivity = onActivity;
    this.onOwnerState = onOwnerState;
    this.pending = new Map(); this.nextId = 1; this.active = 0; this.stopped = false;
  }
  async start() {
    if (this.starting) return this.starting;
    this.starting = this.#start();
    return this.starting;
  }
  async #start() {
    const executable = join(this.app.path, this.app.manifest.entry);
    if (this.native && (this.app.manifest.runtime.platform !== process.platform || this.app.manifest.runtime.arch !== process.arch)) throw hostError('host_app_platform_incompatible');
    const manifestSha256 = this.native ? createHash('sha256').update(await readFile(join(this.app.path, 'manifest.json'))).digest('hex') : null;
    this.child = this.spawn(this.native ? executable : process.execPath, this.native
      ? ['--app-worker', '--data-dir', this.dataDir, '--package-dir', this.app.path]
      : ['--permission', `--allow-fs-read=${this.app.path}`, `--allow-fs-read=${this.dataDir}`, `--allow-fs-write=${this.dataDir}`, executable], {
      cwd: this.app.path, env: { PATH: process.env.PATH ?? '', NODE_NO_WARNINGS: '1' }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
    });
    this.exitObserved = false;
    this.exitPromise = new Promise(resolve => { this.resolveExit = resolve; });
    let buffer = '';
    this.child.stdout.setEncoding('utf8');
    this.child.stdout.on('data', chunk => {
      buffer += chunk;
      if (Buffer.byteLength(buffer) > (this.native ? 4 * 1024 * 1024 + 4096 : 131072)) return this.fail('host_app_frame_too_large');
      let index;
      while ((index = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, index); buffer = buffer.slice(index + 1);
        try {
          const message = JSON.parse(line, (_key, value) => { if (typeof value === 'number' && !Number.isFinite(value)) throw Error('nonfinite'); return value; });
          if (this.native && message?.jsonrpc === '2.0' && message.method === 'application/event') {
            if (Object.keys(message).some(key => !['jsonrpc', 'method', 'params'].includes(key)) || !message.params || typeof message.params !== 'object' || Array.isArray(message.params) || Object.keys(message.params).some(key => !['name', 'payload'].includes(key)) || !/^[A-Za-z0-9_.-]{1,96}$/u.test(message.params.name) || Buffer.byteLength(line) > 65536) return this.fail('host_app_protocol_invalid');
            // Domain notifications are refresh hints. Results are fetched only
            // through current scoped permissions; raw payloads never enter SSE.
            try { this.onActivity({ appId: this.app.id }); } catch { /* Observer loss grants no authority. */ }
            void this.refreshStatus();
            continue;
          }
          if (!message || message.jsonrpc !== '2.0' || message.method || !this.pending.has(message.id) ||
              Object.keys(message).some(key => !['jsonrpc', 'id', 'result', 'error'].includes(key)) ||
              Object.hasOwn(message, 'result') === Object.hasOwn(message, 'error')) return this.fail('host_app_protocol_invalid');
          if (Object.hasOwn(message, 'error') && (!message.error || typeof message.error !== 'object' || Array.isArray(message.error) || !Number.isInteger(message.error.code) || typeof message.error.message !== 'string')) return this.fail('host_app_protocol_invalid');
          const pending = this.pending.get(message.id);
          const maximum = pending.method === 'action/call' ? 4 * 1024 * 1024 : pending.method === 'lifecycle/status' ? 4096 : 65536;
          if (this.native && Buffer.byteLength(line) + 1 > maximum) return this.fail('host_app_frame_too_large');
          this.pending.delete(message.id); clearTimeout(pending.timer);
          if (message.error) pending.reject(applicationError(this.native, message.error)); else pending.resolve(message.result);
        } catch { this.fail('host_app_protocol_invalid'); return; }
      }
    });
    // Drain diagnostics without exposing package-controlled strings as host logs.
    this.child.stderr.on('data', () => {});
    this.child.stdin.on('error', () => this.fail('host_app_transport_failed'));
    this.child.stdout.on('error', () => this.fail('host_app_transport_failed'));
    this.child.stderr.on('error', () => this.fail('host_app_transport_failed'));
    this.child.on('error', () => { this.exitObserved = true; this.resolveExit(); this.fail('host_app_process_failed'); });
    this.child.on('exit', () => { this.exitObserved = true; this.resolveExit(); this.fail('host_app_process_exited'); });
    const apiVersion = this.native ? '2.0.0' : '1.0.0';
    const description = await this.request('describe', { hostApiVersion: apiVersion });
    const catalogueMatches = this.native
      ? description?.manifestSha256 === manifestSha256 && JSON.stringify(description.toolNames) === JSON.stringify(this.app.manifest.tools.map(item => item.name)) && JSON.stringify(description.actionNames) === JSON.stringify(this.app.manifest.actions.map(item => item.name)) && JSON.stringify(description.pageIds) === JSON.stringify(this.app.manifest.pages.map(item => item.id))
      : JSON.stringify(description?.tools) === JSON.stringify(this.app.manifest.tools) && JSON.stringify(description?.pages) === JSON.stringify(this.app.manifest.pages);
    if (description?.protocolVersion !== (this.native ? 2 : 1) || description.appId !== this.app.id || description.version !== this.app.version || !catalogueMatches) {
      this.fail('host_app_description_mismatch'); throw hostError('host_app_description_mismatch');
    }
    const result = await this.request('lifecycle/start', { dataDir: this.dataDir, ...(this.native ? { packageDir: this.app.path } : {}), context: { hostApiVersion: apiVersion, appId: this.app.id } });
    if (result?.dataSchemaVersion !== this.app.manifest.dataSchemaVersion) { this.fail('host_app_migration_failed'); throw hostError('host_app_migration_failed'); }
    if (this.native) {
      try { this.onOwnerState({ state: 'running', pid: this.child.pid }); }
      catch { this.fail('host_app_owner_persistence_failed'); throw hostError('host_app_owner_persistence_failed', 409); }
    }
    if (this.native) { await this.refreshStatus(); if (this.stopped) throw hostError('host_app_owner_unresolved', 409); }
    return description;
  }
  async refreshStatus() {
    if (!this.native || !this.child || this.stopped) return;
    if (this.refreshingStatus) return this.refreshingStatus;
    this.refreshingStatus = (async () => {
      try {
        const result = await this.request('lifecycle/status', {});
        this.nativeBusy = !result || typeof result !== 'object' || Array.isArray(result) || result.busy !== false ||
          ['uncertain', 'outcomeUnknown', 'ownerUnresolved', 'quarantined'].some(key => Object.hasOwn(result, key) && result[key] !== false);
      } catch { this.nativeBusy = true; }
      finally {
        this.#reportNativeStatus();
        clearTimeout(this.statusTimer);
        if (this.nativeBusy && !this.stopped) {
          this.statusTimer = setTimeout(() => { void this.refreshStatus(); }, 2000);
          this.statusTimer.unref();
        }
      }
    })().finally(() => { this.refreshingStatus = null; });
    return this.refreshingStatus;
  }
  #reportNativeStatus() {
    if (!this.native || this.lastNotifiedBusy === this.nativeBusy) return;
    this.lastNotifiedBusy = this.nativeBusy;
    try { this.onActivity({ appId: this.app.id, busy: this.nativeBusy }); } catch { /* Metadata observer only. */ }
  }
  request(method, params) {
    if (!this.child || this.stopped) return Promise.reject(hostError('host_app_process_unavailable'));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(hostError('host_app_timeout')); this.fail('host_app_timeout'); }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer, method });
      this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n', error => { if (error) this.fail('host_app_transport_failed'); });
    });
  }
  async call(name, args, context) {
    await this.start(); this.active++;
    if (this.native) this.nativeBusy = true;
    try {
      const value = await this.request('tool/call', { name, arguments: args, context });
      if (Buffer.byteLength(JSON.stringify(value)) > 65536) throw hostError('host_app_result_too_large');
      if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).some(key => !['content', 'structuredContent', 'isError'].includes(key)) ||
          !Array.isArray(value.content) || value.content.length > 32 || value.content.some(part => !part || typeof part !== 'object' || Array.isArray(part) || Object.keys(part).some(key => !['type', 'text'].includes(key)) || part.type !== 'text' || typeof part.text !== 'string') ||
          (value.isError !== undefined && typeof value.isError !== 'boolean') || (value.structuredContent !== undefined && (!value.structuredContent || typeof value.structuredContent !== 'object' || Array.isArray(value.structuredContent)))) throw hostError('host_app_result_invalid');
      return value;
    }
    finally { this.active--; if (this.native) await this.refreshStatus(); }
  }
  async action(name, args, context) {
    if (!this.native) throw hostError('host_app_actions_unavailable');
    await this.start(); this.active++;
    this.nativeBusy = true;
    try {
      const value = await this.request('action/call', { name, arguments: args, context });
      if (Buffer.byteLength(JSON.stringify(value)) > 4 * 1024 * 1024) throw hostError('host_app_result_too_large');
      return value;
    } finally { this.active--; await this.refreshStatus(); }
  }
  async stop() {
    if (this.active) throw hostError('host_app_busy', 409);
    if (!this.child) return;
    if (this.native && this.stopped && !this.stopConfirmed) throw hostError('host_app_owner_unresolved', 409);
    if (this.exitObserved) {
      if (this.native) this.onOwnerState({ state: 'stopped', pid: this.child.pid, idleStopConfirmed: this.stopConfirmed === true, exitObserved: true });
      return;
    }
    if (!this.stopped) {
      const result = await this.request('lifecycle/stop', {});
      if (result?.busy) throw hostError('host_app_busy', 409);
      if (this.native && result?.busy !== false) throw hostError('host_app_stop_unconfirmed', 409);
      this.idleStopConfirmed = result?.busy === false;
      if (this.native) { this.stopConfirmed = true; this.nativeBusy = false; }
      this.stopped = true; clearTimeout(this.statusTimer); this.child.kill();
    }
    let timer;
    try {
      await Promise.race([this.exitPromise, new Promise((_, reject) => { timer = setTimeout(() => reject(hostError('host_app_stop_unconfirmed', 409)), this.timeoutMs); })]);
      if (this.native) this.onOwnerState({ state: 'stopped', pid: this.child.pid, idleStopConfirmed: this.stopConfirmed === true, exitObserved: this.exitObserved === true });
    }
    finally { clearTimeout(timer); }
  }
  fail(code) {
    this.stopped = true;
    if (this.native) this.nativeBusy = !this.stopConfirmed;
    this.#reportNativeStatus();
    if (this.native && !this.stopConfirmed) {
      try { this.onOwnerState({ state: 'unresolved', pid: this.child?.pid }); }
      catch { /* Earlier durable starting/running/unresolved remains blocking; never clear it on I/O/DB failure. */ }
    }
    clearTimeout(this.statusTimer);
    for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(hostError(code)); }
    this.pending.clear();
    // A disconnected native controller does not establish that accepted work
    // had no effect. Keep its owner quarantined; no force-kill or replay.
    if (!this.native) this.child?.kill();
  }
}
