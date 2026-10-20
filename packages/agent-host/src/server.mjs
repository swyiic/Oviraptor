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
/** Desktop webview origins allowed to call session-authenticated /api/* (CORS). */
function isDesktopWebviewOrigin(value) {
  if (typeof value !== 'string' || !value) return false;
  if (value === 'tauri://localhost' || value === 'https://tauri.localhost') return true;
  let parsed;
  try { parsed = new URL(value); } catch { return false; }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return false;
  if (parsed.hostname !== 'localhost' && parsed.hostname !== '127.0.0.1') return false;
  return !parsed.username && !parsed.password;
}
function applyDesktopCors(response, requestOrigin) {
  response.setHeader('Access-Control-Allow-Origin', requestOrigin);
  response.setHeader('Access-Control-Allow-Headers', 'X-Oviraptor-Session, Content-Type');
  response.setHeader('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS');
  response.setHeader('Vary', 'Origin');
}
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
      if (closing) throw hostError('host_closing', 503);
      const url = new URL(request.url, origin);
      const path = decodeURIComponent(url.pathname); const parts = path.split('/').filter(Boolean);
      const method = request.method;
      const requestOrigin = request.headers.origin;
      const isApi = parts[0] === 'api';
      const desktopCors = isApi && isDesktopWebviewOrigin(requestOrigin);
      if (requestOrigin !== undefined && requestOrigin !== origin && !desktopCors) throw hostError('host_origin_invalid', 403);
      if (desktopCors) applyDesktopCors(response, requestOrigin);
      // Preflight only: no session, no body. Actual /api/* still requires X-Oviraptor-Session.
      if (method === 'OPTIONS' && desktopCors) { response.writeHead(204); response.end(); return; }
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
        return json(await host.mcp(request.headers['x-oviraptor-mcp'], parts[1], input.method, input.params, { runId: input.runId }));
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
        if (path === '/api/settings/network' && method === 'GET') return json(host.networkSettings());
        if (path === '/api/settings/network' && method === 'POST') return json(host.setNetwork(await body(request)));
        if (path === '/api/settings/provider' && method === 'POST') return json(await host.setProvider(await body(request)));
        if (path === '/api/settings/provider' && method === 'DELETE') return json(await host.clearProvider());
        if (path === '/api/settings/provider/oauth/start' && method === 'POST') {
          const input = await body(request).catch(() => ({}));
          return json(await host.startProviderOAuth(input ?? {}));
        }
        if (path === '/api/settings/provider/oauth/poll' && method === 'POST') return json(await host.pollProviderOAuth());
        if (path === '/api/settings/provider/oauth/select' && method === 'POST') return json(await host.selectProviderOAuth());
        if (path === '/api/settings/provider/oauth/status' && method === 'GET') return json(await host.pollProviderOAuth());
        if (path === '/api/settings/provider/oauth/logout' && method === 'POST') return json(await host.logoutProviderOAuth());
        if (path === '/api/settings' && method === 'POST') {
          const input = await body(request);
          let result = null;
          if (input?.answerMode !== undefined) result = host.setAnswerMode(input.answerMode);
          if (input?.scratchConversations !== undefined) result = host.setScratchConversations(input.scratchConversations);
          if (input?.runtime !== undefined) result = await host.setRuntime(input.runtime);
          return json(result ?? host.settings());
        }
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
        if (path === '/api/projects' && method === 'POST') {
          const input = await body(request);
          if (input?.ensureByExternalId != null && input.ensureByExternalId !== '') {
            return json(host.store.ensureProjectByExternalId(input.ensureByExternalId, input.name ?? `Workspace ${input.ensureByExternalId}`));
          }
          const kind = input?.scratch === true || input?.kind === 'scratch' ? 'scratch' : input?.kind;
          const name = input?.name ?? (kind === 'scratch' ? undefined : null);
          if (kind === 'scratch' && (name == null || name === '')) {
            return json(host.store.createScratchProject());
          }
          return json(host.store.createProject(input.name, { externalId: input.externalId, kind }));
        }
        if (parts[1] === 'projects' && parts.length === 4 && parts[3] === 'promote' && method === 'POST') {
          const input = await body(request);
          return json(host.promoteProject(parts[2], input.name));
        }
        if (parts[1] === 'projects' && parts.length === 5 && parts[3] === 'insights' && parts[4] === 'propose' && method === 'POST') {
          const input = await body(request);
          return json(host.proposeInsight(parts[2], input));
        }
        if (parts[1] === 'projects' && parts.length === 5 && parts[3] === 'insights' && parts[4] === 'commit' && method === 'POST') {
          const input = await body(request);
          return json(host.commitInsight(parts[2], input));
        }
        if (path === '/api/conversations' && method === 'GET') return json({ items: host.store.conversations(url.searchParams.get('projectId')) });
        if (path === '/api/conversations' && method === 'POST') {
          const input = await body(request);
          return json(host.createConversationFromRequest(input));
        }
        if (parts[1] === 'conversations' && parts.length === 3 && method === 'DELETE') {
          host.deleteConversation(decodeURIComponent(parts[2]), url.searchParams.get('projectId'));
          return json({ ok: true });
        }
        if (parts[1] === 'conversations' && parts[3] === 'messages' && parts.length === 4) {
          if (method === 'GET') return json({ items: host.store.messages(parts[2], url.searchParams.get('projectId') ?? undefined) });
          if (method === 'POST') return json(await host.send(parts[2], await body(request)));
        }
        if (parts[1] === 'conversations' && parts[3] === 'task' && parts.length === 4 && method === 'GET') {
          return json({ task: host.task(parts[2], { projectId: url.searchParams.get('projectId') ?? undefined, taskId: url.searchParams.get('taskId') ?? undefined }) });
        }
        if (parts[1] === 'conversations' && parts[3] === 'approvals' && parts.length === 4 && method === 'POST') {
          return json(host.prepareTask(parts[2], await body(request)));
        }
        if (parts[1] === 'conversations' && parts[3] === 'approvals' && parts[5] === 'confirm' && parts.length === 6 && method === 'POST') {
          return json(host.confirmTask(parts[2], { ...await body(request), approvalId: parts[4] }));
        }
        if (parts[1] === 'projects' && parts[3] === 'history' && parts.length === 4 && method === 'GET') {
          return json(host.queryHistory(parts[2], { includePurged: url.searchParams.get('includePurged') === '1' }));
        }
        if (parts[1] === 'projects' && parts[3] === 'budget' && parts.length === 4 && method === 'POST') {
          const input = await body(request);
          if (input.owner === 'native') return json(host.reserveNativeBudget(parts[2], input.amount));
          return json(host.setBudgetAllocation(parts[2], input));
        }
        if (parts[1] === 'conversations' && parts[3] === 'task' && parts[4] === 'report' && parts.length === 5 && method === 'GET') {
          const format = url.searchParams.get('format') === 'html' ? 'html' : 'markdown';
          return json(host.taskReport(parts[2], { projectId: url.searchParams.get('projectId') ?? undefined, taskId: url.searchParams.get('taskId') ?? undefined, format }));
        }
        if (parts[1] === 'conversations' && parts[3] === 'task' && parts[4] === 'continue' && parts.length === 5 && method === 'POST') {
          return json(await host.continueTask(parts[2], await body(request)));
        }
        if (parts[1] === 'conversations' && parts[3] === 'task' && parts[4] === 'children' && parts[6] === 'cancel' && parts.length === 7 && method === 'POST') {
          return json(await host.cancelChild(decodeURIComponent(parts[2]), { ...await body(request), childTaskId: decodeURIComponent(parts[5]) }));
        }
        if (parts[1] === 'conversations' && parts[3] === 'skills' && parts.length === 4) {
          const skills = await host.ensureSkills();
          if (method === 'GET') return json(skills.get(parts[2]));
          if (method === 'POST') { const result = skills.set(parts[2], await body(request)); await host.skillsChanged(); return json(result); }
        }
        if (path === '/api/runtime/cancel' && method === 'POST') return json(await host.cancel((await body(request)).conversationId));
        if (path === '/api/runtime/authorization' && method === 'POST') {
          const input = await body(request);
          return json({ accepted: host.resolveAuthorization(input.conversationId, input.accepted === true, { requestId: input.requestId, runId: input.runId, remember: input.remember === true }) });
        }
        if (path === '/api/runtime/workspace-grants' && method === 'GET') {
          const projectId = url.searchParams.get('projectId');
          if (!projectId) throw hostError('host_project_invalid');
          return json(host.workspaceGrants(url.searchParams.get('conversationId'), projectId));
        }
        if (path === '/api/runtime/workspace-grants' && method === 'DELETE') {
          const input = await body(request);
          if (input.projectId === undefined) throw hostError('host_project_invalid');
          return json(host.clearWorkspaceGrants(input.conversationId, input.projectId));
        }
        if (path === '/api/runtime/steer' && method === 'POST') {
          const input = await body(request);
          return json(host.steer(input.conversationId, input.text));
        }
        if (parts[1] === 'runtime' && parts[2] === 'permissions' && parts.length === 4 && method === 'POST') return json(await host.decide(parts[3], (await body(request)).optionId));
        if (path === '/api/apps/bundled' && method === 'GET') return json(host.bundledApps());
        if (path === '/api/apps/bundled/ensure' && method === 'POST') {
          const input = await body(request).catch(() => ({}));
          const result = await host.ensureBundledApps({ force: input?.force !== false });
          return json({ ...result, items: host.bundledApps().items, shell: host.shellApps() });
        }
        if (path === '/api/apps/shell' && method === 'GET') return json(host.shellApps());
        if (path === '/api/apps' && method === 'GET') return json({ items: host.apps.list() });
        if (path === '/api/apps' && method === 'POST') {
          const result = await host.apps.install((await body(request)).path);
          host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] });
          return json(await host.withPairedSkillsPreview(result));
        }
        if (parts[1] === 'apps' && parts.length === 3 && method === 'DELETE') { const result = await host.apps.uninstall(parts[2]); host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] }); return json(result); }
        if (parts[1] === 'apps' && parts[3] === 'state' && parts.length === 4 && method === 'POST') {
          const enabled = (await body(request)).enabled;
          const result = await host.apps.setEnabled(parts[2], enabled);
          host.emit({ type: 'applications.changed', appId: result.id ?? parts[2] });
          return json(enabled ? await host.withPairedSkillsPreview(result) : result);
        }
        if (parts[1] === 'apps' && parts[3] === 'paired-skills' && parts.length === 4 && method === 'GET') return json(await host.pairedSkillsPreview(parts[2]));
        if (parts[1] === 'apps' && parts[3] === 'paired-skills' && parts[4] === 'import' && parts.length === 5 && method === 'POST') {
          const result = await host.importPairedSkills(parts[2], await body(request));
          return json(result);
        }
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
