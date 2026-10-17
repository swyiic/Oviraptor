// An actual local child process. No network, files, credentials or Grok invocation.
import readline from 'node:readline';
const mode = process.argv[2] ?? 'normal';
const sessions = new Map(), permissions = new Map();
let nextSession = 0, nextPermission = 0, initialized = false, authenticated = false;
const send = message => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
const result = (id, value) => send({ id, result: value });
const update = (sessionId, value) => send({ method: 'session/update', params: { sessionId, update: value } });
const text = (sessionId, value) => update(sessionId, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: value } });
function finish(sessionId, reason = 'end_turn') {
  const session = sessions.get(sessionId); if (!session?.prompt) return;
  for (const timer of session.timers ?? []) clearTimeout(timer);
  const request = session.prompt; session.prompt = null; result(request.id, { stopReason: reason });
}
function permission(sessionId) {
  const id = ++nextPermission;
  permissions.set(id, { sessionId });
  send({ id, method: 'session/request_permission', params: { sessionId, toolCall: { toolCallId: `tool-${id}`, title: 'Synthetic write request', kind: 'edit', status: 'pending' }, options: mode === 'allow-only' ? [{ optionId: 'yes', name: 'Allow once', kind: 'allow_once' }] : [{ optionId: 'yes', name: 'Allow once', kind: 'allow_once' }, { optionId: 'no', name: 'Reject once', kind: 'reject_once' }] } });
}
const lines = readline.createInterface({ input: process.stdin });
lines.on('line', line => {
  const message = JSON.parse(line), { id, method, params } = message;
  if (method === 'initialize') {
    if (params.protocolVersion !== 1 || params.clientCapabilities.fs.readTextFile || params.clientCapabilities.fs.writeTextFile || params.clientCapabilities.terminal) process.exit(71);
    initialized = true;
    if (mode === 'init-hang') return;
    result(id, { protocolVersion: mode === 'version-2' ? 2 : 1, agentCapabilities: { loadSession: mode !== 'no-load' }, authMethods: mode === 'no-auth' ? [] : [{ id: 'cached_token', name: 'Local cached authentication' }] }); return;
  }
  if (method === 'authenticate') {
    if (!initialized || params.methodId !== 'cached_token' || params._meta.headless !== true || Object.keys(params).some(key => /token|key/i.test(key))) process.exit(72);
    authenticated = true; result(id, {}); return;
  }
  if (method === 'session/new') {
    if (!authenticated || !params.cwd.startsWith('/') || !Array.isArray(params.mcpServers)) process.exit(73);
    const sessionId = `fixture-${process.pid}-${++nextSession}`; sessions.set(sessionId, {}); result(id, { sessionId }); return;
  }
  if (method === 'session/load') {
    if (mode === 'no-load') process.exit(74);
    sessions.set(params.sessionId, {}); text(params.sessionId, 'Synthetic restored history'); result(id, {}); return;
  }
  if (method === 'session/prompt') {
    const session = sessions.get(params.sessionId); if (!session || session.prompt) process.exit(75);
    session.prompt = message; session.kind = params.prompt.map(block => block.text).join(''); session.timers = [];
    if (session.kind === 'crash') { process.exit(23); return; }
    if (session.kind === 'bad-json') { process.stdout.write('not-json\n'); return; }
    if (session.kind === 'bad-envelope') { send({ id, result: {}, error: { code: 1, message: 'bad' } }); return; }
    if (session.kind === 'oversized') { process.stdout.write('x'.repeat(2_000_000)); return; }
    if (session.kind === 'remote-error') { process.stderr.write('synthetic-private-stderr\n'); session.prompt = null; send({ id, error: { code: -32000, message: 'synthetic-private-diagnostic' } }); return; }
    if (session.kind === 'permission' || session.kind === 'permission-cancel') { permission(params.sessionId); return; }
    if (['hang', 'ignore-cancel', 'wait'].includes(session.kind)) { text(params.sessionId, 'Synthetic waiting'); return; }
    // Deliberately split a multibyte UTF-8 sequence between data events.
    const frame = Buffer.from(JSON.stringify({ jsonrpc: '2.0', method: 'session/update', params: { sessionId: params.sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: '你好 ' } } } }) + '\n');
    const split = frame.indexOf(Buffer.from('你')) + 1;
    if (session.kind === 'stream') { process.stdout.write(frame.subarray(0, split)); session.timers.push(setTimeout(() => process.stdout.write(frame.subarray(split)), 10)); }
    else text(params.sessionId, '你好 ');
    session.timers.push(setTimeout(() => update(params.sessionId, { sessionUpdate: 'tool_call', toolCallId: 'synthetic-tool', title: 'Synthetic scoped search', kind: 'search', status: 'in_progress' }), 20));
    session.timers.push(setTimeout(() => { text(params.sessionId, session.kind); update(params.sessionId, { sessionUpdate: 'tool_call_update', toolCallId: 'synthetic-tool', status: 'completed' }); finish(params.sessionId); }, 45)); return;
  }
  if (method === 'session/cancel') {
    if (Object.hasOwn(message, 'id')) process.exit(76);
    const session = sessions.get(params.sessionId); if (!session?.prompt || session.kind === 'ignore-cancel') return;
    session.cancelled = true;
    if (session.kind === 'permission-cancel') { permission(params.sessionId); return; }
    update(params.sessionId, { sessionUpdate: 'tool_call_update', toolCallId: 'synthetic-tool', status: 'failed' }); finish(params.sessionId, 'cancelled'); return;
  }
  if (!method && Object.hasOwn(message, 'result') && permissions.has(id)) {
    const entry = permissions.get(id), outcome = message.result?.outcome;
    if (!['selected', 'cancelled'].includes(outcome?.outcome)) process.exit(77);
    permissions.delete(id); const session = sessions.get(entry.sessionId);
    if (session.cancelled) {
      if (outcome.outcome !== 'cancelled') process.exit(78);
      if (![...permissions.values()].some(item => item.sessionId === entry.sessionId)) finish(entry.sessionId, 'cancelled');
    } else if (session.kind !== 'permission-cancel') { text(entry.sessionId, `permission:${outcome.optionId ?? 'cancelled'}`); finish(entry.sessionId); }
    return;
  }
  if (id !== undefined) send({ id, error: { code: -32601, message: 'Unsupported fixture method' } });
});
