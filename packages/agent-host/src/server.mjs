import http from 'node:http';
import { randomBytes, timingSafeEqual } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, extname } from 'node:path';
import { hostError, mcpErrorPayload } from './store.mjs';
import { runtimeMetrics } from './metrics.mjs';
import { ArtifactDownloads } from './artifacts.mjs';
import { PageNavigation } from './navigation.mjs';
import { readNativeApplicationErrorCode } from './app-process.mjs';

const mime = path => ({ '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml' }[extname(path)] ?? 'application/octet-stream');
async function body(request, maximumBytes = 65536) {
  let text = '';
  for await (const chunk of request) { text += chunk; if (Buffer.byteLength(text) > maximumBytes) throw hostError('host_request_too_large', 413); }
  try { return text ? JSON.parse(text) : {}; } catch { throw hostError('host_request_invalid'); }
}
const sameToken = (value, expected) => typeof value === 'string' && value.length === expected.length && timingSafeEqual(Buffer.from(value), Buffer.from(expected));
function navigationBody(value, keys) {
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length !== keys.length || keys.some(key => !Object.hasOwn(value, key))) throw hostError('host_navigation_input_invalid');
  return value;
}

export async function serveHost(host, { port = 0, uiDir, token = randomBytes(32).toString('hex') } = {}) {
  const clients = new Set(), mcpClients = new Map(), sockets = new Map(); let origin, closing = false, closePromise;
  const downloads = new ArtifactDownloads(host.apps);
  let navigation;
  const pageNavigation = () => navigation ??= new PageNavigation(host.apps);
  const previous = host.onEvent;
  const observe = event => {
    try { previous(event); } catch { /* Observers cannot change admitted work. */ }
    if (closing) return;
    const encoded = `data: ${JSON.stringify(event)}\n\n`;
    for (const response of clients) if (!response.write(encoded)) { response.destroy(); clients.delete(response); }
    for (const [response, connection] of mcpClients) {
      try { host.validateMcpScope(connection.token, connection.conversationId, { requireActive: false }); }
      catch { response.destroy(); mcpClients.delete(response); continue; }
      if (event.type === 'applications.changed' && (!event.projectId || host.store.conversation(connection.conversationId).projectId === event.projectId)) {
        if (!response.write('data: {"type":"tools.changed"}\n\n')) { response.destroy(); mcpClients.delete(response); }
      }
    }
  };
  host.onEvent = observe;
  const server = http.createServer(async (request, response) => {
    const socket = request.socket; sockets.set(socket, (sockets.get(socket) ?? 0) + 1);
    let released = false;
    const release = () => {
      if (released) return; released = true;
      if (sockets.has(socket)) sockets.set(socket, Math.max(0, sockets.get(socket) - 1));
      if (closing && sockets.get(socket) === 0) socket.end();
    };
    response.once('finish', release); response.once('close', release);
    response.setHeader('Cache-Control', 'no-store'); response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Referrer-Policy', 'no-referrer');
    const json = value => { response.setHeader('Content-Type', 'application/json'); response.end(JSON.stringify(value)); };
    try {
      if (`http://${request.headers.host}` !== origin) throw hostError('host_origin_invalid', 403);
      if (request.headers.origin !== undefined && request.headers.origin !== origin) throw hostError('host_origin_invalid', 403);
      if (closing) throw hostError('host_closing', 503);
      const url = new URL(request.url, origin);
      const path = decodeURIComponent(url.pathname); const parts = path.split('/').filter(Boolean);
      const method = request.method;
      if (parts[0] === 'downloads' && parts.length === 2 && method === 'GET') return await downloads.stream(parts[1], response);
      if (parts[0] === 'mcp') {
        if (method === 'GET' && parts.length === 3 && parts[2] === 'events') {
          const credential = request.headers['x-oviraptor-mcp'];
          host.validateMcpScope(credential, parts[1], { requireActive: false });
          response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'keep-alive' });
          response.write('data: {"type":"tools.changed"}\n\n');
          mcpClients.set(response, { token: credential, conversationId: parts[1] });
          response.once('close', () => mcpClients.delete(response)); return;
        }
        if (method !== 'POST' || parts.length !== 2) throw hostError('host_route_unavailable', 404);
        const input = await body(request);
        return json(await host.mcp(request.headers['x-oviraptor-mcp'], parts[1], input.method, input.params));
      }
      if (parts[0] === 'api') {
        if (!sameToken(request.headers['x-oviraptor-session'], token)) throw hostError('host_session_required', 403);
        if (path === '/api/events' && method === 'GET') {
          response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'keep-alive' });
          response.write(`data: ${JSON.stringify({ type: 'host.snapshot', ...host.status() })}\n\n`);
          clients.add(response); response.once('close', () => clients.delete(response)); return;
        }
        if (path === '/api/status' && method === 'GET') return json(host.status());
        if (path === '/api/runtime/metrics' && method === 'GET') return json(runtimeMetrics(host.store, Number(url.searchParams.get('limit') ?? 100)));
        if (path === '/api/settings' && method === 'GET') return json(host.settings());
        if (path === '/api/settings' && method === 'POST') return json(await host.setRuntime((await body(request)).runtime));
        if (parts[1] === 'skills') {
          await host.ensureSkills();
          if (path === '/api/skills/packages' && method === 'GET') return json({ items: host.library.listPackages() });
          if (path === '/api/skills' && method === 'GET') return json({ items: host.library.listSkills() });
          if (path === '/api/skills/preview' && method === 'POST') return json(await host.library.previewDirectory((await body(request)).path));
          if (path === '/api/skills/import' && method === 'POST') { const input = await body(request); const result = await host.library.importDirectory(input.path, input.expectedDigest); await host.skillsChanged(); return json(result); }
          if (path === '/api/skills/bundled' && method === 'POST') { const result = await host.library.installBundled(host.bundledSkillsPath()); await host.skillsChanged(); return json(result); }
          if (parts[2] === 'packages' && parts.length === 4 && method === 'DELETE') { const result = host.library.uninstallPackage(parts[3], await body(request)); await host.skillsChanged(); return json(result); }
          if (parts.length === 4 && parts[3] === 'state' && method === 'POST') { const result = host.library.setEnabled(Number(parts[2]), (await body(request)).enabled); await host.skillsChanged(); return json(result); }
          if (parts.length === 4 && parts[3] === 'resources' && method === 'GET') return json(host.library.listResources(Number(parts[2])));
          if (parts.length === 4 && parts[3] === 'resource' && method === 'GET') return json(host.library.readResource(Number(parts[2]), url.searchParams.get('path'), Number(url.searchParams.get('offset') ?? 0), Number(url.searchParams.get('limit') ?? 8192)));
          throw hostError('host_route_unavailable', 404);
        }
        if (path === '/api/projects' && method === 'GET') return json({ items: host.store.projects() });
        if (path === '/api/projects' && method === 'POST') return json(host.store.createProject((await body(request)).name));
        if (path === '/api/conversations' && method === 'GET') return json({ items: host.store.conversations(url.searchParams.get('projectId')) });
        if (path === '/api/conversations' && method === 'POST') { const input = await body(request); return json(host.store.createConversation(input.projectId, input.title)); }
        if (parts[1] === 'conversations' && parts[3] === 'messages' && parts.length === 4) {
          if (method === 'GET') return json({ items: host.store.messages(parts[2], url.searchParams.get('projectId') ?? undefined) });
          if (method === 'POST') return json(await host.send(parts[2], await body(request)));
        }
        if (parts[1] === 'conversations' && parts[3] === 'skills' && parts.length === 4) {
          const skills = await host.ensureSkills();
          if (method === 'GET') return json(skills.get(parts[2]));
          if (method === 'POST') { const result = skills.set(parts[2], await body(request)); await host.skillsChanged(); return json(result); }
        }
        if (path === '/api/runtime/cancel' && method === 'POST') return json(await host.cancel((await body(request)).conversationId));
        if (parts[1] === 'runtime' && parts[2] === 'permissions' && parts.length === 4 && method === 'POST') return json(await host.decide(parts[3], (await body(request)).optionId));
        if (path === '/api/apps/bundled' && method === 'GET') return json(host.bundledApps());
        if (path === '/api/apps/shell' && method === 'GET') return json(host.shellApps());
        if (path === '/api/apps' && method === 'GET') return json({ items: host.apps.list() });
        if (path === '/api/apps' && method === 'POST') { const result = await host.apps.install((await body(request)).path); host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] }); return json(result); }
        if (parts[1] === 'apps' && parts.length === 3 && method === 'DELETE') { const result = await host.apps.uninstall(parts[2]); host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] }); return json(result); }
        if (parts[1] === 'apps' && parts[3] === 'state' && parts.length === 4 && method === 'POST') { const result = await host.apps.setEnabled(parts[2], (await body(request)).enabled); host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] }); return json(result); }
        if (parts[1] === 'apps' && parts[3] === 'upgrade' && parts.length === 4 && method === 'POST') { const input = await body(request); const result = await host.apps.upgrade(parts[2], input.path, { expectedRevision: input.expectedRevision }); host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] }); return json(result); }
        if (parts[1] === 'apps' && parts[3] === 'grants' && parts.length === 4 && method === 'POST') {
          const input = await body(request);
          if (input.disclosureAccepted !== true) throw hostError('host_runtime_disclosure_required', 403);
          if (input.expectedRuntimeIdentity !== host.runtimeIdentity()) throw hostError('host_runtime_identity_changed', 409);
          const result = host.apps.setGrants(parts[2], input.projectId, input.permissions, input.expectedRevision, host.runtimeIdentity());
          host.emit({ type: 'applications.changed', appId: parts[2], projectId: Number(input.projectId) }); return json(result);
        }
        if (parts[1] === 'apps' && parts[3] === 'navigation' && parts.length === 5 && method === 'POST') {
          if (['external', 'handoff', 'capabilities'].includes(parts[4])) {
            const input = navigationBody(await body(request, 512 * 1024 + 4096), ['scope', 'input']);
            return json(parts[4] === 'external' ? pageNavigation().prepareExternal(parts[2], input.scope, input.input) : parts[4] === 'handoff' ? pageNavigation().prepareHandoff(parts[2], input.scope, input.input) : pageNavigation().capabilities(parts[2], input.scope, input.input));
          }
          if (parts[4] === 'consume') { const input = navigationBody(await body(request), ['token', 'scope']); return json(pageNavigation().consumeHandoff(input.token, parts[2], input.scope)); }
        }
        if (parts[1] === 'navigation' && parts.length === 3 && method === 'POST' && ['confirm', 'cancel'].includes(parts[2])) {
          const input = navigationBody(await body(request), ['token', 'scope']);
          return json(parts[2] === 'confirm' ? pageNavigation().confirm(input.token, input.scope) : pageNavigation().cancel(input.token, input.scope));
        }
        if (parts[1] === 'apps' && parts[3] === 'tools' && parts.length === 5 && method === 'POST') {
          const input = await body(request);
          const app = host.store.application(parts[2]);
          const pageContext = app.manifest.schemaVersion === 2 ? { expectedAppRevision: input.expectedAppRevision, pageId: input.pageId, version: input.version } : undefined;
          return json(await host.apps.call(parts[2], parts[4], input.projectId, input.arguments, input.expectedGrantRevision, input.operationId, pageContext));
        }
        if (parts[1] === 'apps' && parts[3] === 'actions' && parts.length === 5 && method === 'POST') {
          const input = await body(request, 1024 * 1024 + 16384);
          return json(await host.apps.callAction(parts[2], parts[4], input.projectId, input.arguments, {
            expectedAppRevision: input.expectedAppRevision, expectedGrantRevision: input.expectedGrantRevision,
            operationId: input.operationId, pageId: input.pageId, applicationScopeAccepted: input.applicationScopeAccepted,
          }));
        }
        if (parts[1] === 'apps' && parts[3] === 'artifact-downloads' && parts.length === 4 && method === 'POST') return json(await downloads.create(parts[2], await body(request)));
        throw hostError('host_route_unavailable', 404);
      }
      if (parts[0] === 'apps' && parts.length >= 5 && method === 'GET') {
        const file = await host.apps.pageFile(parts[1], parts[2], parts[3], parts.slice(4).join('/'));
        response.setHeader('Content-Security-Policy', "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'; sandbox allow-scripts");
        response.setHeader('Content-Type', mime(file.path)); response.end(file.bytes); return;
      }
      if (method !== 'GET' || !uiDir) throw hostError('host_route_unavailable', 404);
      const relative = path === '/' ? 'index.html' : path.slice(1);
      if (!relative || relative.split('/').some(part => part === '..' || part.startsWith('.')) || relative.includes('\\')) throw hostError('host_route_unavailable', 404);
      let bytes = await readFile(join(uiDir, relative));
      if (relative === 'index.html') bytes = Buffer.from(bytes.toString().replace('</head>', `<script>window.__OVIRAPTOR_HOST__=${JSON.stringify({ token })}</script></head>`));
      response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'");
      response.setHeader('Content-Type', mime(relative)); response.end(bytes);
    } catch (error) {
      if (response.headersSent) { response.destroy(); return; }
      for (const header of ['Content-Length', 'Content-Disposition', 'X-Content-SHA256']) response.removeHeader(header);
      response.statusCode = Number.isInteger(error?.status) && error.status >= 400 && error.status <= 599 ? error.status : error?.code === 'ENOENT' ? 404 : 400;
      const native = readNativeApplicationErrorCode(error);
      const payload = mcpErrorPayload(error);
      if (native) payload.error = native;
      json(payload);
    }
  });
  server.on('connection', socket => {
    sockets.set(socket, 0); socket.once('close', () => sockets.delete(socket));
    if (closing) socket.destroy();
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  origin = `http://127.0.0.1:${server.address().port}`; host.setOrigin(origin);
  return { origin, token, server, close: () => {
    if (closePromise) return closePromise;
    closing = true;
    if (host.onEvent === observe) host.onEvent = previous;
    navigation?.close(); downloads.close();
    for (const response of clients) response.destroy(); clients.clear();
    for (const response of mcpClients.keys()) response.destroy(); mcpClients.clear();
    closePromise = new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    // Chromium may retain preconnect/partial-header sockets which Node's HTTP
    // idle tracking does not close. No request was admitted on these sockets.
    // Already admitted HTTP actions retain their response and native owner.
    for (const [socket, activeRequests] of sockets) if (activeRequests === 0) socket.destroy();
    return closePromise;
  } };
}
