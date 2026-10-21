// Local synthetic scope HTTP/SSE endpoint; never uses account credentials.
import http from 'node:http';
import { randomBytes } from 'node:crypto';

export async function mcpHttpFixture(t, { initialEvent = true, onPost, eventStatus = 200 } = {}) {
  const token = randomBytes(32).toString('hex'), conversation = 'synthetic-conversation';
  const state = { urls: [], posts: [], eventHeaders: [], opened: 0, closed: 0, activePosts: 0, peakPosts: 0, streams: new Set(), sockets: new Set() };
  const server = http.createServer(async (request, response) => {
    state.urls.push(request.url);
    if (request.headers['x-oviraptor-mcp'] !== token) { response.writeHead(403).end(); return; }
    if (request.url === `/mcp/${conversation}/events` && request.method === 'GET') {
      state.eventHeaders.push({ accept: request.headers.accept, scoped: request.headers['x-oviraptor-mcp'] === token });
      if (eventStatus !== 200) { response.writeHead(eventStatus).end(); return; }
      state.opened++; state.streams.add(response); response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store' });
      response.write(': synthetic connected\n\n');
      if (initialEvent) response.write('data: {"type":"tools.changed"}\n\n');
      response.on('close', () => { state.closed++; state.streams.delete(response); }); return;
    }
    if (request.url !== `/mcp/${conversation}` || request.method !== 'POST') { response.writeHead(404).end(); return; }
    let source = ''; for await (const chunk of request) source += chunk;
    const input = JSON.parse(source); state.posts.push(input); state.activePosts++; state.peakPosts = Math.max(state.peakPosts, state.activePosts);
    let finished = false;
    const complete = value => { if (finished) return; finished = true; state.activePosts--; response.setHeader('Content-Type', 'application/json'); response.end(JSON.stringify(value)); };
    response.on('close', () => { if (!finished) { finished = true; state.activePosts--; } });
    if (onPost) return onPost(input, complete, response);
    complete(input.method === 'initialize' ? { protocolVersion: '2025-06-18', capabilities: { tools: { listChanged: true } }, serverInfo: { name: 'synthetic', version: '1' } } : { tools: [] });
  });
  server.on('connection', socket => { state.sockets.add(socket); socket.on('close', () => state.sockets.delete(socket)); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(async () => { for (const socket of state.sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); });
  return { state, token, conversation, origin: `http://127.0.0.1:${server.address().port}`,
    event(value) { for (const response of state.streams) response.write(`data: ${JSON.stringify(value)}\n\n`); },
    raw(value) { for (const response of state.streams) response.write(value); },
  };
}
