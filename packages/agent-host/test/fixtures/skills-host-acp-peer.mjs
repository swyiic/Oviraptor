// Synthetic ACP peer that starts the production MCP bridge as a real child.
// All requests stay on the host's scoped localhost endpoint. No model/account.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import readline from 'node:readline';

const sessions = new Map(); let nextSession = 0;
const send = message => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
const reply = (id, result) => send({ id, result });
const update = (sessionId, value) => send({ method: 'session/update', params: { sessionId, update: value } });
const text = (sessionId, value) => update(sessionId, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: value } });

function connect(server, cwd) {
  if (server.command !== process.execPath || !server.args?.[0]?.endsWith('/mcp-bridge.mjs') || !Array.isArray(server.env)) throw Error('Invalid scoped MCP fixture configuration');
  const child = spawn(server.command, server.args, { cwd, shell: false, stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, ...Object.fromEntries(server.env.map(item => [item.name, item.value])) } });
  const pending = new Map(); let nextId = 0;
  const fail = () => { for (const operation of pending.values()) { clearTimeout(operation.timer); operation.reject(Error('Synthetic bridge ended')); } pending.clear(); };
  child.stderr.on('data', () => {}); child.on('error', fail); child.on('exit', fail); child.stdin.on('error', fail);
  readline.createInterface({ input: child.stdout }).on('line', line => {
    const message = JSON.parse(line), operation = pending.get(message.id); if (!operation) return;
    pending.delete(message.id); clearTimeout(operation.timer);
    if (message.error) operation.reject(Object.assign(Error('Scoped MCP operation rejected'), { code: message.error.code })); else operation.resolve(message.result);
  });
  return { child, request(method, params = {}) {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(Error('Synthetic scoped MCP deadline')); }, 4000);
      pending.set(id, { resolve, reject, timer }); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  } };
}

async function cancelled(sessionId, session) {
  if (session.finishing || !session.prompt || !session.outputReady) return;
  session.finishing = true;
  let deniedAfterCancel = false;
  try { await session.bridge.request('tools/call', { name: 'skills_read', arguments: { skillId: session.action.calls?.[0]?.arguments?.skillId ?? 1 } }); }
  catch { deniedAfterCancel = true; }
  // Deliberately attempt a successful final answer despite cancellation. The
  // host, rather than this synthetic peer, must withhold stale guidance output.
  text(sessionId, JSON.stringify({ ...session.output, deniedAfterCancel, attemptedFinal: 'WITHHELD-SKILLS-FINAL' }));
  reply(session.prompt, { stopReason: 'end_turn' }); session.prompt = null;
}

async function handle(message) {
  const { id, method, params = {} } = message;
  if (method === 'initialize') return reply(id, { protocolVersion: 1, agentCapabilities: { loadSession: true }, authMethods: [{ id: 'cached_token', name: 'Synthetic cached authentication' }] });
  if (method === 'authenticate') { if (params.methodId !== 'cached_token') throw Error('Unexpected synthetic authentication'); return reply(id, {}); }
  if (method === 'session/new') {
    if (params.mcpServers?.length !== 1 || params.mcpServers[0].name !== 'oviraptor-apps') throw Error('Missing real MCP bridge');
    const sessionId = `skills-host-peer-${++nextSession}`, bridge = connect(params.mcpServers[0], params.cwd);
    sessions.set(sessionId, { bridge, history: [] });
    await bridge.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'synthetic-skills-peer', version: '1' } });
    return reply(id, { sessionId });
  }
  if (method === 'session/load') {
    const session = sessions.get(params.sessionId); if (!session || session.prompt) throw Error('Unknown or busy synthetic session');
    session.bridge.child.kill('SIGTERM'); session.bridge = connect(params.mcpServers[0], params.cwd);
    await session.bridge.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'synthetic-skills-peer', version: '1' } });
    for (const message of session.history) text(params.sessionId, message);
    return reply(id, {});
  }
  if (method === 'session/prompt') {
    const session = sessions.get(params.sessionId); if (!session || session.prompt) throw Error('Unknown or busy synthetic session');
    const source = params.prompt.map(part => part.text).join(''), marker = 'SKILLS-HOST-INTEGRATION:', index = source.lastIndexOf(marker);
    if (index < 0) throw Error('Missing synthetic action marker');
    session.prompt = id; session.action = JSON.parse(source.slice(index + marker.length)); session.outputReady = false; session.cancelled = false; session.finishing = false;
    const listing = await session.bridge.request('tools/list');
    session.output = { agentPid: process.pid, bridgePid: session.bridge.child.pid, tools: listing.tools, results: [], historyCount: session.history.length };
    for (const [index, call] of (session.action.calls ?? []).entries()) {
      const toolCallId = `skills-host-call-${id}-${index}`;
      update(params.sessionId, { sessionUpdate: 'tool_call', toolCallId, title: 'Read explicitly selected Skill reference', kind: 'read', status: 'in_progress' });
      try {
        const result = await session.bridge.request('tools/call', { name: call.name, arguments: call.arguments ?? {} });
        session.output.results.push({ name: call.name, result });
        update(params.sessionId, { sessionUpdate: 'tool_call_update', toolCallId, status: 'completed' });
      } catch (error) {
        session.output.results.push({ name: call.name, error: { code: error.code ?? 'mcp_rejected' } });
        update(params.sessionId, { sessionUpdate: 'tool_call_update', toolCallId, status: 'failed' });
      }
    }
    session.outputReady = true;
    if (session.cancelled) return cancelled(params.sessionId, session);
    if (session.action.action === 'read-wait') {
      const content = session.output.results.find(item => typeof item.result?.structuredContent?.content === 'string')?.result.structuredContent.content ?? '';
      text(params.sessionId, 'SKILLS_READ_READY:' + JSON.stringify({ agentPid: process.pid, bridgePid: session.bridge.child.pid,
        resultsReceived: session.output.results.length, contentSha256: createHash('sha256').update(content).digest('hex') }) + '\n');
      return;
    }
    session.history.push(source, JSON.stringify(session.output));
    text(params.sessionId, JSON.stringify(session.output)); reply(id, { stopReason: 'end_turn' }); session.prompt = null; return;
  }
  if (method === 'session/cancel') {
    const session = sessions.get(params.sessionId); if (!session?.prompt) return;
    session.cancelled = true; return cancelled(params.sessionId, session);
  }
  if (id !== undefined) send({ id, error: { code: -32601, message: 'Unsupported synthetic method' } });
}
readline.createInterface({ input: process.stdin }).on('line', line => {
  void handle(JSON.parse(line)).catch(() => { process.stderr.write('Synthetic Skills integration peer failed\n'); process.kill(process.pid, 'SIGTERM'); });
});
process.on('SIGTERM', () => { for (const session of sessions.values()) session.bridge.child.kill('SIGTERM'); setTimeout(() => process.exit(0), 10); });
