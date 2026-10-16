import { randomBytes } from 'node:crypto';

const fail = (code, status = 400) => { throw Object.assign(new Error(code), { code, status }); };
const ID = /^(?=.{3,128}$)[a-z][a-z0-9]*(?:-[a-z0-9]+)*(?:\.[a-z][a-z0-9]*(?:-[a-z0-9]+)*)+$/u;
const PAGE = /^[a-z][a-z0-9_]{0,63}$/u;
const TOKEN = /^[a-f0-9]{64}$/u;
const SCOPE_KEYS = ['projectId', 'pageId', 'version', 'expectedAppRevision', 'expectedGrantRevision'];
export const NAVIGATION_LIMITS = Object.freeze({ payloadBytes: 512 * 1024, stringBytes: 4096, arrayItems: 1024, objectKeys: 64, nodes: 8192, depth: 8, urlBytes: 2048, maxPending: 64, maxTtlMs: 60000 });

function object(value, required, code = 'host_navigation_input_invalid') {
  if (value === null || typeof value !== 'object' || Array.isArray(value) || ![Object.prototype, null].includes(Object.getPrototypeOf(value))) fail(code);
  const descriptors = Object.getOwnPropertyDescriptors(value), keys = Reflect.ownKeys(descriptors);
  if (keys.length !== required.length || keys.some(key => typeof key !== 'string' || !required.includes(key) || !Object.hasOwn(descriptors[key], 'value') || !descriptors[key].enumerable)) fail(code);
  return value;
}
function scopeValue(value) {
  object(value, SCOPE_KEYS, 'host_navigation_scope_invalid');
  if (!Number.isSafeInteger(value.projectId) || value.projectId < 1 || !PAGE.test(value.pageId) ||
      typeof value.version !== 'string' || !value.version || Buffer.byteLength(value.version) > 80 || /[\u0000-\u0020\u007f]/u.test(value.version) ||
      !Number.isSafeInteger(value.expectedAppRevision) || value.expectedAppRevision < 1 || !Number.isSafeInteger(value.expectedGrantRevision) || value.expectedGrantRevision < 0) fail('host_navigation_scope_invalid');
  return { ...value };
}
function sameScope(a, b) { return SCOPE_KEYS.every(key => a[key] === b[key]); }
function appId(value) { if (typeof value !== 'string' || !ID.test(value)) fail('host_navigation_input_invalid'); return value; }
function pageId(value) { if (typeof value !== 'string' || !PAGE.test(value)) fail('host_navigation_input_invalid'); return value; }
function ticketValue(value) { if (typeof value !== 'string' || !TOKEN.test(value)) fail('host_navigation_ticket_invalid'); return value; }

/** No URL is fetched or opened here. The trusted parent offers the confirmed link for a separate user click. */
export function externalNavigationUrl(value) {
  if (typeof value !== 'string' || !value || Buffer.byteLength(value) > NAVIGATION_LIMITS.urlBytes || !/^https?:\/\//iu.test(value) ||
      /[\s\u0000-\u001f\u007f-\u009f\\]/u.test(value) || /%(?:0[0-9a-f]|1[0-9a-f]|7f)/iu.test(value)) fail('host_navigation_url_invalid');
  let parsed;
  try { parsed = new URL(value); } catch { fail('host_navigation_url_invalid'); }
  if (!['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname || parsed.username || parsed.password || Buffer.byteLength(parsed.href) > NAVIGATION_LIMITS.urlBytes) fail('host_navigation_url_invalid');
  return parsed.href;
}

/** Clone only a finite plain JSON tree. Never invoke getters, silently truncate, or interpret domain payload fields. */
export function handoffPayload(value) {
  let nodes = 0;
  const parents = new Set();
  const visit = (input, depth) => {
    if (++nodes > NAVIGATION_LIMITS.nodes || depth > NAVIGATION_LIMITS.depth) fail('host_navigation_payload_too_large');
    if (input === null || typeof input === 'boolean') return input;
    if (typeof input === 'number') { if (!Number.isFinite(input)) fail('host_navigation_payload_invalid'); return input; }
    if (typeof input === 'string') {
      if (Buffer.byteLength(input) > NAVIGATION_LIMITS.stringBytes) fail('host_navigation_payload_too_large');
      if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(input)) fail('host_navigation_payload_invalid');
      return input;
    }
    if (typeof input !== 'object' || input === null || parents.has(input)) fail('host_navigation_payload_invalid');
    const array = Array.isArray(input);
    if (!array && ![Object.prototype, null].includes(Object.getPrototypeOf(input))) fail('host_navigation_payload_invalid');
    if (array && input.length > NAVIGATION_LIMITS.arrayItems) fail('host_navigation_payload_too_large');
    const descriptors = Object.getOwnPropertyDescriptors(input), keys = Reflect.ownKeys(descriptors);
    if (array ? keys.length !== input.length + 1 : keys.length > NAVIGATION_LIMITS.objectKeys) fail(array ? 'host_navigation_payload_invalid' : 'host_navigation_payload_too_large');
    const result = array ? [] : {};
    parents.add(input);
    for (const key of keys) {
      if (array && key === 'length') continue;
      const descriptor = descriptors[key];
      if (typeof key !== 'string' || !Object.hasOwn(descriptor, 'value') || !descriptor.enumerable ||
          ['__proto__', 'prototype', 'constructor'].includes(key) || /[\u0000-\u001f\u007f]/u.test(key) || Buffer.byteLength(key) > 128 ||
          array && (!/^(0|[1-9]\d*)$/u.test(key) || Number(key) >= input.length)) fail('host_navigation_payload_invalid');
      result[key] = visit(descriptor.value, depth + 1);
    }
    parents.delete(input);
    return result;
  };
  if (value === null || typeof value !== 'object' || Array.isArray(value)) fail('host_navigation_payload_invalid');
  const result = visit(value, 0);
  if (Buffer.byteLength(JSON.stringify(result)) > NAVIGATION_LIMITS.payloadBytes) fail('host_navigation_payload_too_large');
  return result;
}

/** Parent-only, in-memory one-use confirmations. No worker, process, business DB, filesystem, or model calls. */
export class PageNavigation {
  constructor(apps, { now = Date.now, ttlMs = 30000, maxPending = 64 } = {}) {
    if (!apps?.store || typeof now !== 'function' || !Number.isSafeInteger(ttlMs) || ttlMs < 1 || ttlMs > NAVIGATION_LIMITS.maxTtlMs || !Number.isSafeInteger(maxPending) || maxPending < 1 || maxPending > NAVIGATION_LIMITS.maxPending) fail('host_navigation_config_invalid');
    this.apps = apps; this.now = now; this.ttlMs = ttlMs; this.maxPending = maxPending; this.pending = new Map(); this.deliveries = new Map(); this.closed = false;
  }
  #clock() { const now = this.now(); if (!Number.isSafeInteger(now) || now < 0 || now > Number.MAX_SAFE_INTEGER - this.ttlMs) fail('host_navigation_clock_invalid'); return now; }
  #ready() {
    if (this.closed || this.apps.closing) fail('host_navigation_unavailable', 409);
    const now = this.#clock();
    for (const tickets of [this.pending, this.deliveries]) for (const [token, value] of tickets) if (value.expiresAt <= now) tickets.delete(token);
    return now;
  }
  #newToken() {
    let token;
    do { token = randomBytes(32).toString('hex'); } while (this.pending.has(token) || this.deliveries.has(token));
    return token;
  }
  #capture(id, scope, permission) {
    appId(id);
    const project = this.apps.store.project(scope.projectId), app = this.apps.store.application(id), grant = this.apps.store.grant(id, scope.projectId);
    if (!project || project.id !== scope.projectId || project.status !== undefined && project.status !== 'active' ||
        !app.enabled || app.manifest?.schemaVersion !== 2 || this.apps.changing?.has(id) || this.apps.upgradeWorkers?.has(id) || this.apps.workers?.get(id)?.stopped || this.apps.ownerUnresolved?.(id)) fail('host_navigation_unavailable', 409);
    if (app.version !== scope.version || app.revision !== scope.expectedAppRevision || grant.revision !== scope.expectedGrantRevision || !app.manifest.pages.some(page => page.id === scope.pageId)) fail('host_navigation_authority_changed', 409);
    if (permission !== null && (!app.manifest.permissions.some(item => item.id === permission) || !grant.permissions.includes(permission))) fail('host_navigation_permission_denied', 403);
    if (typeof app.digest !== 'string' || !/^[a-f0-9]{64}$/u.test(app.digest)) fail('host_navigation_authority_changed', 409);
    return { appId: id, scope: { ...scope }, digest: app.digest, permission };
  }
  #current(binding, scope = binding.scope) {
    if (!sameScope(binding.scope, scope)) fail('host_navigation_authority_changed', 409);
    const current = this.#capture(binding.appId, scope, binding.permission);
    if (current.digest !== binding.digest) fail('host_navigation_authority_changed', 409);
  }
  #prepare(kind, id, rawScope, rawInput) {
    const now = this.#ready(), scope = scopeValue(rawScope);
    const source = this.#capture(id, scope, kind === 'external' ? 'ui.external.open' : 'ui.handoff');
    if (this.pending.size + this.deliveries.size >= this.maxPending) fail('host_navigation_limit', 429);
    const expiresAt = now + this.ttlMs, token = this.#newToken();
    let value;
    if (kind === 'external') {
      object(rawInput, ['url']);
      value = { kind, source, url: externalNavigationUrl(rawInput.url), expiresAt };
    } else {
      object(rawInput, ['targetAppId', 'targetPageId', 'payload']);
      const targetAppId = appId(rawInput.targetAppId), targetPageId = pageId(rawInput.targetPageId), payload = handoffPayload(rawInput.payload);
      const targetApp = this.apps.store.application(targetAppId), targetGrant = this.apps.store.grant(targetAppId, scope.projectId);
      const targetScope = scopeValue({ projectId: scope.projectId, pageId: targetPageId, version: targetApp.version, expectedAppRevision: targetApp.revision, expectedGrantRevision: targetGrant.revision });
      value = { kind, source, target: this.#capture(targetAppId, targetScope, 'ui.handoff'), payload, expiresAt };
    }
    this.pending.set(token, value);
    return kind === 'external' ? { token, kind, projectId: scope.projectId, url: value.url, expiresAt }
      : { token, kind, targetAppId: value.target.appId, targetPageId: value.target.scope.pageId, projectId: scope.projectId, preview: structuredClone(value.payload), expiresAt };
  }
  prepareExternal(id, scope, input) { return this.#prepare('external', id, scope, input); }
  prepareHandoff(id, scope, input) { return this.#prepare('handoff', id, scope, input); }
  capabilities(id, rawScope, rawInput) {
    this.#ready(); const scope = scopeValue(rawScope);
    this.#capture(id, scope, null);
    object(rawInput, ['targetAppId', 'targetPageId']);
    const targetAppId = appId(rawInput.targetAppId), targetPageId = pageId(rawInput.targetPageId);
    const available = permission => {
      try { this.#capture(id, scope, permission); return true; }
      catch (error) { if (error.code === 'host_navigation_permission_denied') return false; throw error; }
    };
    const externalOpen = available('ui.external.open'); let handoff = false;
    if (available('ui.handoff')) {
      try {
        const target = this.apps.store.application(targetAppId), grant = this.apps.store.grant(targetAppId, scope.projectId);
        this.#capture(targetAppId, scopeValue({projectId:scope.projectId,pageId:targetPageId,version:target.version,expectedAppRevision:target.revision,expectedGrantRevision:grant.revision}), 'ui.handoff');
        handoff = true;
      } catch (error) {
        if (!['host_app_unavailable', 'host_navigation_unavailable', 'host_navigation_permission_denied', 'host_navigation_authority_changed'].includes(error.code)) throw error;
      }
    }
    return { externalOpen, handoff };
  }
  confirm(token, rawScope) {
    this.#ready(); ticketValue(token); const scope = scopeValue(rawScope), value = this.pending.get(token);
    if (!value) fail('host_navigation_ticket_invalid', 409);
    // Failed confirmations also consume the token: a stale dialog cannot later regain authority.
    this.pending.delete(token);
    this.#current(value.source, scope);
    if (value.kind === 'external') return { kind: 'external', url: value.url };
    this.#current(value.target);
    const deliveryToken = this.#newToken(); this.deliveries.set(deliveryToken, value);
    return { kind: 'handoff', targetAppId: value.target.appId, targetPageId: value.target.scope.pageId, projectId: value.source.scope.projectId, deliveryToken };
  }
  consumeHandoff(token, id, rawScope) {
    this.#ready(); ticketValue(token); appId(id); const scope = scopeValue(rawScope), value = this.deliveries.get(token);
    if (!value) fail('host_navigation_ticket_invalid', 409);
    this.deliveries.delete(token);
    if (id !== value.target.appId) fail('host_navigation_authority_changed', 409);
    this.#current(value.source); this.#current(value.target, scope);
    return { protocol: 'oviraptor.handoff.v1', purpose: 'draft-only', sourceAppId: value.source.appId, projectId: value.source.scope.projectId, payload: structuredClone(value.payload) };
  }
  cancel(token, rawScope) {
    this.#ready(); ticketValue(token); const scope = scopeValue(rawScope), value = this.pending.get(token);
    if (!value || !sameScope(value.source.scope, scope)) fail('host_navigation_ticket_invalid', 409);
    this.pending.delete(token); return { cancelled: true };
  }
  close() { this.closed = true; this.pending.clear(); this.deliveries.clear(); }
}
