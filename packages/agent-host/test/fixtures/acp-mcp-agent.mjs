// Synthetic ACP peer. It really starts the host-provided stdio MCP bridge.
// No Grok, model/provider request, credential extraction, or external network.
import { spawn } from 'node:child_process';
import readline from 'node:readline';

const sessions = new Map(); let nextSession = 0;
const send = message => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
const reply = (id, result) => send({ id, result });
const update = (sessionId, value) => send({ method: 'session/update', params: { sessionId, update: value } });
const text = (sessionId, value) => send({ method: 'session/update', params: { sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: value } } } });
function connect(server, cwd) {
  if (server.command !== process.execPath || !server.args?.[0]?.endsWith('/mcp-bridge.mjs') || !Array.isArray(server.env)) throw Error('Invalid synthetic MCP configuration');
  const child = spawn(server.command, server.args, { cwd, env: { ...process.env, ...Object.fromEntries(server.env.map(item => [item.name, item.value])) }, stdio: ['pipe', 'pipe', 'pipe'], shell: false });
  const pending = new Map(); let nextId = 0;
  child.stderr.on('data', () => {});
  readline.createInterface({ input: child.stdout }).on('line', line => {
    const value = JSON.parse(line), operation = pending.get(value.id); if (!operation) return;
    pending.delete(value.id); clearTimeout(operation.timer);
    if (value.error) operation.reject(Object.assign(Error('MCP operation rejected'), { code: value.error.code })); else operation.resolve(value.result);
  });
  child.on('exit', () => { for (const operation of pending.values()) { clearTimeout(operation.timer); operation.reject(Error('MCP bridge exited')); } pending.clear(); });
  child.on('error', () => { for (const operation of pending.values()) operation.reject(Error('MCP bridge failed')); });
  return { child, request(method, params = {}) {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(Error('Synthetic MCP deadline')); }, 3000);
      pending.set(id, { resolve, reject, timer }); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  } };
}
async function handle(message) {
  const { id, method, params = {} } = message;
  if (method === 'initialize') { reply(id, { protocolVersion: 1, agentCapabilities: { loadSession: false }, authMethods: [{ id: 'cached_token', name: 'Synthetic authentication only' }] }); return; }
  if (method === 'authenticate') { if (params.methodId !== 'cached_token') throw Error('Unexpected authentication method'); reply(id, {}); return; }
  if (method === 'session/new') {
    if (params.mcpServers?.length !== 1 || params.mcpServers[0].name !== 'oviraptor-apps') throw Error('Expected scoped MCP bridge');
    const bridge = connect(params.mcpServers[0], params.cwd), sessionId = `synthetic-chain-${++nextSession}`;
    sessions.set(sessionId, { bridge });
    await bridge.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'synthetic-acp-agent', version: '1' } });
    reply(id, { sessionId }); return;
  }
  if (method === 'session/prompt') {
    const session = sessions.get(params.sessionId); if (!session) throw Error('Unknown session');
    session.prompt = id;
    const source = params.prompt.map(block => block.text).join(''), marker = source.lastIndexOf('CHAIN:');
    const action = JSON.parse(source.slice(marker + 6));
    const output = { agentPid: process.pid, bridgePid: session.bridge.child.pid };
    const listing = await session.bridge.request('tools/list'); output.tools = listing.tools.map(tool => tool.name);
    if (action.action === 'wait-cancel') { text(params.sessionId, 'chain-wait-ready\n'); return; }
    if (action.action !== 'list') {
      const tool = listing.tools.find(tool => tool.name.endsWith('__echo'));
      const name = action.action === 'call-known' ? 'org.oviraptor.echo__echo' : tool?.name;
      if (!name) output.unavailable = true;
      else {
        const args = { text: action.text ?? 'Synthetic MCP echo' };
        if (action.action === 'extra-project') args.projectId = action.projectId;
        const toolCallId = `synthetic-chain-tool-${id}`;
        update(params.sessionId, { sessionUpdate: 'tool_call', toolCallId, title: 'Call explicitly granted Echo capability', kind: 'other', status: 'in_progress' });
        try { output.call = await session.bridge.request('tools/call', { name, arguments: args }); update(params.sessionId, { sessionUpdate: 'tool_call_update', toolCallId, status: 'completed' }); }
        catch (error) { output.error = { code: error.code ?? 'mcp_rejected' }; update(params.sessionId, { sessionUpdate: 'tool_call_update', toolCallId, status: 'failed' }); }
      }
    }
    text(params.sessionId, JSON.stringify(output)); reply(id, { stopReason: 'end_turn' }); session.prompt = null; return;
  }
  if (method === 'session/cancel') {
    const session = sessions.get(params.sessionId); if (!session?.prompt) return;
    let denied = false;
    try { await session.bridge.request('tools/list'); } catch { denied = true; }
    text(params.sessionId, JSON.stringify({ cancelledMcpDenied: denied })); reply(session.prompt, { stopReason: 'cancelled' }); session.prompt = null; return;
  }
  if (id !== undefined) send({ id, error: { code: -32601, message: 'Unsupported synthetic method' } });
}
readline.createInterface({ input: process.stdin }).on('line', line => { void handle(JSON.parse(line)).catch(() => { process.stderr.write('Synthetic chain failure\n'); process.exitCode = 1; process.kill(process.pid, 'SIGTERM'); }); });
process.on('SIGTERM', () => { for (const { bridge } of sessions.values()) bridge.child.kill('SIGTERM'); setTimeout(() => process.exit(0), 10); });
