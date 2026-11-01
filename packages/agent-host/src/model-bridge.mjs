// A credential-owning transport for native model callers. It performs no agent
// actions and never chooses a different model or returns login credentials.
import { once } from 'node:events';
import { hostError } from './store.mjs';
import { networkFetch } from './net-proxy.mjs';

export async function forwardOAuthModelRequest(host, request, response, input) {
  const listing = input === null;
  if (!listing && (!input || typeof input !== 'object' || Array.isArray(input)
    || !Array.isArray(input.messages) || !input.messages.length
    || typeof input.model !== 'string' || !/^[A-Za-z0-9._:/-]{1,128}$/.test(input.model)
    || (input.stream !== undefined && typeof input.stream !== 'boolean'))) throw hostError('host_model_request_invalid');
  const controller = new AbortController();
  const cancel = () => { if (!response.writableFinished) controller.abort(); };
  response.once('close', cancel);
  request.once('aborted', cancel);
  try {
    let login = await host.ensureSavedOAuthAccess();
    controller.signal.throwIfAborted();
    const send = () => networkFetch(host.providerFetch, `https://api.x.ai/v1/${listing ? 'models' : 'chat/completions'}`, {
      method: listing ? 'GET' : 'POST', redirect: 'error', signal: controller.signal,
      // The caller's session header, proxy or URL can never become upstream auth.
      headers: { Authorization: `Bearer ${login.accessToken}`, 'Content-Type': 'application/json', Accept: input?.stream ? 'text/event-stream' : 'application/json' },
      ...(listing ? {} : { body: JSON.stringify(input) }),
    });
    let upstream = await send();
    if (upstream.status === 401 && login.refreshToken) {
      await upstream.body?.cancel();
      login = await host.ensureSavedOAuthAccess({ force: true });
      controller.signal.throwIfAborted();
      upstream = await send();
    }
    response.statusCode = upstream.status;
    response.setHeader('Content-Type', upstream.headers.get('content-type') || 'application/json');
    if (!upstream.body) { response.end(); return; }
    const reader = upstream.body.getReader();
    let total = 0;
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        total += value.byteLength;
        if (total > 32 * 1024 * 1024) throw hostError('host_model_response_too_large', 502);
        if (!response.write(value)) await once(response, 'drain', { signal: controller.signal });
      }
      response.end();
    } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
  } finally {
    controller.abort();
    response.removeListener('close', cancel);
    request.removeListener('aborted', cancel);
  }
}
