/** Browser-only client for an opaque-origin application page. No Host token. */
import { readHandoff } from './handoff.mjs';
export const PAGE_PROTOCOL = 'oviraptor.app-ui.v2';
export const PAGE_REQUEST_BYTES = 1024 * 1024;
const namePattern = /^[a-z][a-z0-9_]{0,63}$/u;
const error = code => Object.assign(new Error(code), { code });
function bytes(value) {
  const encoded = JSON.stringify(value, (_key, item) => {
    if (['undefined', 'function', 'symbol', 'bigint'].includes(typeof item) || typeof item === 'number' && !Number.isFinite(item)) throw error('app_page_arguments_invalid');
    return item;
  });
  if (typeof encoded !== 'string' || new TextEncoder().encode(encoded).length > PAGE_REQUEST_BYTES) throw error('app_page_request_too_large');
  return encoded;
}
function hostOrigin(value) {
  try {
    const url = new URL(value);
    return url.origin === value && url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  } catch { return false; }
}
export function createPageClient({ appId, window: view = globalThis.window, timeoutMs = 30000 } = {}) {
  if (!view?.parent || view.parent === view || typeof appId !== 'string' || !appId || appId.length > 128 || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 120000) throw error('app_page_client_invalid');
  let context, origin, nonce, closed = false, nextId = 1;
  const pending = new Map();
  const activityListeners = new Set();
  let acceptReady, rejectReady;
  const ready = new Promise((resolve, reject) => { acceptReady = resolve; rejectReady = reject; });
  const readyTimer = setTimeout(() => rejectReady(error('app_page_initialization_timeout')), timeoutMs);
  readyTimer.unref?.();
  // A late consumer can still await the same rejected promise without an
  // unhandled rejection before its UI has finished mounting.
  ready.catch(() => {});
  function settleAll(code) {
    for (const item of pending.values()) { clearTimeout(item.timer); item.reject(error(code)); }
    pending.clear();
  }
  function receive(event) {
    const message = event.data;
    if (closed || event.source !== view.parent || !hostOrigin(event.origin) || !message || typeof message !== 'object' || Array.isArray(message) || message.protocol !== PAGE_PROTOCOL || message.appId !== appId) return;
    if (message.type === 'init') {
      if (typeof message.nonce !== 'string' || message.nonce.length < 16 || message.nonce.length > 128 || !Number.isSafeInteger(message.projectId) || message.projectId < 1) return;
      let handoff;
      try { if (Object.hasOwn(message, 'handoff')) handoff = readHandoff(message.handoff, message.projectId); } catch { return; }
      if (nonce && (nonce !== message.nonce || context.projectId !== message.projectId || origin !== event.origin)) settleAll('app_page_context_changed');
      origin = event.origin; nonce = message.nonce;
      context = Object.freeze({ appId, projectId: message.projectId, projectName: typeof message.projectName === 'string' ? message.projectName : '', appRevision: message.appRevision, grantRevision: message.grantRevision, ...(handoff ? { handoff } : {}) });
      clearTimeout(readyTimer); acceptReady(context);
      send({ type: 'ready' }); return;
    }
    if (!nonce || event.origin !== origin || message.nonce !== nonce) return;
    if (message.type === 'activity') { for (const listener of activityListeners) { try { listener(); } catch { /* A page observer cannot change transport. */ } } return; }
    if (message.type !== 'result' || !Number.isSafeInteger(message.requestId)) return;
    const item = pending.get(message.requestId);
    if (!item || item.nonce !== nonce) return;
    pending.delete(message.requestId); clearTimeout(item.timer);
    if (Object.hasOwn(message, 'error') && Object.hasOwn(message, 'result')) item.reject(error('app_page_response_invalid'));
    else if (Object.hasOwn(message, 'error')) item.reject(error(typeof message.error?.code === 'string' && /^[A-Za-z0-9_]{1,96}$/u.test(message.error.code) ? message.error.code : 'app_page_action_failed'));
    else if (Object.hasOwn(message, 'result')) item.resolve(message.result);
    else item.reject(error('app_page_response_invalid'));
  }
  function send(message) {
    if (closed || !nonce) throw error('app_page_unavailable');
    view.parent.postMessage({ ...message, protocol: PAGE_PROTOCOL, appId, nonce }, origin);
  }
  async function call(type, name, args = {}, { operationId } = {}) {
    await ready;
    if (closed) throw error('app_page_closed');
    if (!namePattern.test(name) || !args || typeof args !== 'object' || Array.isArray(args)) throw error('app_page_arguments_invalid');
    operationId ??= view.crypto?.randomUUID?.();
    if (typeof operationId !== 'string' || operationId.length < 1 || operationId.length > 128 || /[\u0000-\u001f\u007f]/u.test(operationId)) throw error('app_page_operation_invalid');
    if (pending.size >= 32) throw error('app_page_busy');
    const requestId = nextId++;
    const message = { type, requestId, name, arguments: args, operationId };
    bytes({ ...message, protocol: PAGE_PROTOCOL, appId, nonce });
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(requestId); reject(error('app_page_result_unknown')); }, timeoutMs);
      pending.set(requestId, { resolve, reject, timer, nonce });
      try { send(message); }
      catch (failure) { clearTimeout(timer); pending.delete(requestId); reject(failure); }
    });
  }
  function destroy() {
    if (closed) return;
    closed = true; clearTimeout(readyTimer); rejectReady(error('app_page_closed'));
    settleAll('app_page_closed'); activityListeners.clear(); view.removeEventListener('message', receive);
  }
  view.addEventListener('message', receive);
  return Object.freeze({ ready, get context() { return context; },
    callAction: (name, args, options) => call('action.call', name, args, options),
    callTool: (name, args, options) => call('tool.call', name, args, options),
    downloadArtifact: (name, artifactId, { filename, ...options } = {}) => call('artifact.download', name, { artifactId, filename }, options),
    openExternal: url => call('navigation.external', 'external', { url }),
    handoff: (targetAppId, targetPageId, payload) => call('navigation.handoff', 'handoff', { targetAppId, targetPageId, payload }),
    subscribe(listener) { if (typeof listener !== 'function' || closed) throw error('app_page_listener_invalid'); activityListeners.add(listener); return () => activityListeners.delete(listener); },
    close() { if (nonce && !closed) send({ type: 'close' }); destroy(); }, destroy });
}
