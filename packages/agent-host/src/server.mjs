import http from 'node:http';
import { randomBytes, timingSafeEqual } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, extname } from 'node:path';
import { hostError } from './store.mjs';

const mime = path => ({ '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml' }[extname(path)] ?? 'application/octet-stream');
async function body(request) {
  let text = '';
  for await (const chunk of request) { text += chunk; if (Buffer.byteLength(text) > 65536) throw hostError('host_request_too_large', 413); }
  try { return text ? JSON.parse(text) : {}; } catch { throw hostError('host_request_invalid'); }
}
const sameToken = (value, expected) => typeof value === 'string' && value.length === expected.length && timingSafeEqual(Buffer.from(value), Buffer.from(expected));

export async function serveHost(host, { port = 0, uiDir, token = randomBytes(32).toString('hex') } = {}) {
  const clients = new Set(); let origin;
  const previous = host.onEvent;
  host.onEvent = event => {
    previous(event);
    const encoded = `data: ${JSON.stringify(event)}\n\n`;
    for (const response of clients) if (!response.write(encoded)) { response.destroy(); clients.delete(response); }
  };
  const server = http.createServer(async (request, response) => {
    response.setHeader('Cache-Control', 'no-store'); response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Referrer-Policy', 'no-referrer');
    const json = value => { response.setHeader('Content-Type', 'application/json'); response.end(JSON.stringify(value)); };
    try {
      if (`http://${request.headers.host}` !== origin) throw hostError('host_origin_invalid', 403);
      if (request.headers.origin !== undefined && request.headers.origin !== origin) throw hostError('host_origin_invalid', 403);
      const url = new URL(request.url, origin);
      const path = decodeURIComponent(url.pathname); const parts = path.split('/').filter(Boolean);
      const method = request.method;
      if (parts[0] === 'mcp') {
        if (method !== 'POST' || parts.length !== 2) throw hostError('host_route_unavailable', 404);
        const input = await body(request);
        return json(await host.mcp(request.headers['x-oviraptor-mcp'], parts[1], input.method, input.params));
      }
      if (parts[0] === 'api') {
        if (!sameToken(request.headers['x-oviraptor-session'], token)) throw hostError('host_session_required', 403);
        if (path === '/api/events' && method === 'GET') {
          response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'keep-alive' });
          response.write(`data: ${JSON.stringify({ type: 'host.snapshot', ...host.status() })}\n\n`);
          clients.add(response); request.on('close', () => clients.delete(response)); return;
        }
        if (path === '/api/status' && method === 'GET') return json(host.status());
        if (path === '/api/projects' && method === 'GET') return json({ items: host.store.projects() });
        if (path === '/api/projects' && method === 'POST') return json(host.store.createProject((await body(request)).name));
        if (path === '/api/conversations' && method === 'GET') return json({ items: host.store.conversations(url.searchParams.get('projectId')) });
        if (path === '/api/conversations' && method === 'POST') { const input = await body(request); return json(host.store.createConversation(input.projectId, input.title)); }
        if (parts[1] === 'conversations' && parts[3] === 'messages' && parts.length === 4) {
          if (method === 'GET') return json({ items: host.store.messages(parts[2], url.searchParams.get('projectId') ?? undefined) });
          if (method === 'POST') return json(await host.send(parts[2], await body(request)));
        }
        if (path === '/api/runtime/cancel' && method === 'POST') return json(await host.cancel((await body(request)).conversationId));
        if (parts[1] === 'runtime' && parts[2] === 'permissions' && parts.length === 4 && method === 'POST') return json(await host.decide(parts[3], (await body(request)).optionId));
        if (path === '/api/apps' && method === 'GET') return json({ items: host.apps.list() });
        if (path === '/api/apps' && method === 'POST') return json(await host.apps.install((await body(request)).path));
        if (parts[1] === 'apps' && parts.length === 3 && method === 'DELETE') return json(await host.apps.uninstall(parts[2]));
        if (parts[1] === 'apps' && parts[3] === 'state' && parts.length === 4 && method === 'POST') return json(await host.apps.setEnabled(parts[2], (await body(request)).enabled));
        if (parts[1] === 'apps' && parts[3] === 'grants' && parts.length === 4 && method === 'POST') {
          const input = await body(request);
          if (input.disclosureAccepted !== true) throw hostError('host_runtime_disclosure_required', 403);
          return json(host.apps.setGrants(parts[2], input.projectId, input.permissions, input.expectedRevision));
        }
        if (parts[1] === 'apps' && parts[3] === 'tools' && parts.length === 5 && method === 'POST') {
          const input = await body(request);
          return json(await host.apps.call(parts[2], parts[4], input.projectId, input.arguments, input.expectedGrantRevision));
        }
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
      response.statusCode = error.status ?? (error.code === 'ENOENT' ? 404 : 400);
      json({ error: error.code?.startsWith('host_') ? error.code : 'host_request_failed' });
    }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  origin = `http://127.0.0.1:${server.address().port}`; host.setOrigin(origin);
  return { origin, token, server, close: async () => { for (const response of clients) response.destroy(); await new Promise(resolve => server.close(resolve)); } };
}
