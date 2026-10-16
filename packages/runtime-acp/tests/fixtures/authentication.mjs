// Synthetic ACP subprocess: only its explicitly supplied temporary journal is
// touched. No Grok executable, accounts, network, provider or login is involved.
import { appendFileSync } from 'node:fs';
import { createInterface } from 'node:readline';

const [mode, journal] = process.argv.slice(2);
const send = value => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...value }) + '\n');
const methods = mode === 'grok-com' ? [{ id: 'grok.com', name: 'Grok', description: 'Sign in with Grok' }]
  : mode === 'oidc' ? [{ id: 'oidc', name: 'Synthetic OIDC' }]
    : mode === 'mixed' ? [{ id: 'grok.com' }, { id: 'oidc' }]
      : mode === 'empty' ? []
        : mode === 'unknown' ? [{ id: 'unknown_method' }]
          : mode === 'api-key' ? [{ id: 'xai.api_key' }]
            : [{ id: 'cached_token', name: 'Synthetic cached credential' }];
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  appendFileSync(journal, JSON.stringify(request) + '\n');
  const result = value => send({ id: request.id, result: value });
  if (request.method === 'initialize') {
    result({ protocolVersion: 1, agentCapabilities: { loadSession: true, mcpCapabilities: { http: true, sse: true } },
      authMethods: methods, _meta: { defaultAuthMethodId: null } });
    if (mode === 'grok-com') send({ method: '_x.ai/mcp/servers_updated', params: { mcpServers: [] } });
  } else if (request.method === 'authenticate') {
    if (mode === 'auth-rejected') send({ id: request.id, error: { code: -32000, message: 'Synthetic private diagnostic, never forward' } });
    else result({});
  } else if (request.method === 'session/new') result({ sessionId: 'synthetic-auth-session' });
  else if (request.method === 'session/prompt') {
    if (mode === 'prompt-rejected') send({ id: request.id, error: { code: -32000, message: 'Synthetic private prompt diagnostic, never forward' } });
    else result({ stopReason: 'end_turn' });
  } else send({ id: request.id, error: { code: -32601, message: 'Unsupported fixture method' } });
});
