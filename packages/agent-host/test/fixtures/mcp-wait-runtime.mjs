// Local ACP peer that keeps a Host run open until cancellation. No model calls.
import readline from 'node:readline';
const prompts = new Map();
const send = value => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...value }) + '\n');
readline.createInterface({ input: process.stdin }).on('line', source => {
  const { id, method, params = {} } = JSON.parse(source);
  if (method === 'initialize') send({ id, result: { protocolVersion: 1, agentCapabilities: { loadSession: true }, authMethods: [{ id: 'cached_token', name: 'Synthetic cached auth' }] } });
  else if (method === 'authenticate' || method === 'session/load') send({ id, result: {} });
  else if (method === 'session/new') send({ id, result: { sessionId: 'synthetic-notification-session' } });
  else if (method === 'session/prompt') { prompts.set(params.sessionId, id); send({ method: 'session/update', params: { sessionId: params.sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Synthetic run held open' } } } }); }
  else if (method === 'session/cancel' && prompts.has(params.sessionId)) { send({ id: prompts.get(params.sessionId), result: { stopReason: 'cancelled' } }); prompts.delete(params.sessionId); }
});
