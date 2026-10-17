const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { transpile } = require('./shell_module_harness.cjs');

const root = path.join(__dirname, '..');
const source = (relative) => fs.readFileSync(path.join(root, relative), 'utf8');

function loadModule(relative, imports = {}) {
  const exports = {};
  new Function(
    'require',
    'exports',
    transpile(source(relative)),
  )((name) => {
    if (Object.prototype.hasOwnProperty.call(imports, name)) return imports[name];
    if (name.startsWith('.')) {
      const base = path.resolve(path.dirname(path.join(root, relative)), name);
      const file = fs.existsSync(`${base}.ts`) ? `${base}.ts` : `${base}.js`;
      return loadModule(path.relative(root, file), imports);
    }
    throw Error(`Unexpected import ${name} from ${relative}`);
  }, exports);
  return exports;
}

test('agent-ui choose accepts xai-api alongside none and grok-acp', () => {
  const settings = source('packages/agent-ui/src/settings.js');
  assert.match(settings, /choose\(value\)\{if\(\!\['none','grok-acp','xai-api'\]\.includes\(value\)\)return;/);
  assert.doesNotMatch(settings, /choose\(value\)\{if\(\!\['none','grok-acp'\]\.includes\(value\)\)return;/);
});

test('desktop config center exposes agent-runtime section and panel', () => {
  const app = source('src/App.vue');
  assert.match(app, /configSection==='agent-runtime'/);
  assert.match(app, /AgentRuntimeSettingsPanel/);
  assert.match(app, /Agent runtime \/ xAI Grok/);
  assert.match(app, /from "\.\/features\/agentHost\/components\/AgentRuntimeSettingsPanel\.vue"/);
  const panel = source('src/features/agentHost/components/AgentRuntimeSettingsPanel.vue');
  assert.match(panel, /openAgentHostSettingsClient/);
  assert.match(panel, /apiKeyConfigured/);
  assert.match(panel, /data-testid="agent-runtime-unavailable"/);
  assert.match(panel, /data-testid="agent-runtime-section-runtime"/);
  assert.match(panel, /data-testid="agent-runtime-section-provider"/);
  assert.match(panel, /data-testid="agent-oauth-login"/);
  assert.match(panel, /data-testid="agent-runtime-oauth"/);
  assert.match(panel, /data-testid="agent-runtime-section-status"/);
  assert.match(panel, /type="password"/);
  assert.match(panel, /tr\("已配置", "Configured"\)/);
  assert.doesNotMatch(panel, /apiKey\s*[:=]\s*settings/);
  const client = source('src/features/agentHost/settingsClient.ts');
  assert.match(client, /\/api\/settings/);
  assert.match(client, /\/api\/settings\/provider/);
  assert.match(client, /\/api\/settings\/provider\/oauth\/start/);
  assert.match(client, /\/api\/settings\/provider\/oauth\/poll/);
  assert.match(client, /\/api\/settings\/provider\/oauth\/logout/);
  assert.match(client, /X-Oviraptor-Session/);
  const api = source('src/features/agentHost/api.ts');
  assert.match(api, /agent_host_endpoint/);
});

test('host settings client uses X-Oviraptor-Session and never echoes api keys', async () => {
  const calls = [];
  const responses = new Map([
    [
      'GET /api/settings',
      {
        runtime: { id: 'none', state: 'unconfigured', label: 'No runtime configured' },
        apiProvider: { configured: false, apiKeyConfigured: false },
        runtimes: ['none', 'grok-acp', 'xai-api'],
      },
    ],
    [
      'POST /api/settings',
      {
        runtime: { id: 'grok-acp', state: 'unconfigured', label: 'Official Grok ACP' },
        apiProvider: { configured: false, apiKeyConfigured: false },
      },
    ],
    [
      'POST /api/settings/provider',
      {
        configured: true,
        provider: 'xai',
        model: 'grok-4.7',
        baseUrl: 'https://api.x.ai/v1',
        apiKeyConfigured: true,
      },
    ],
  ]);
  const fetchImpl = async (url, init = {}) => {
    const method = (init.method || 'GET').toUpperCase();
    const pathName = new URL(url).pathname;
    calls.push({
      method,
      path: pathName,
      headers: init.headers,
      body: init.body ? JSON.parse(init.body) : undefined,
    });
    if (method === 'POST' && pathName === '/api/settings/provider') {
      responses.set('GET /api/settings', {
        runtime: { id: 'none', state: 'unconfigured' },
        apiProvider: {
          configured: true,
          provider: 'xai',
          model: 'grok-4.7',
          baseUrl: 'https://api.x.ai/v1',
          apiKeyConfigured: true,
        },
      });
    }
    const payload = responses.get(`${method} ${pathName}`);
    assert.ok(payload, `unexpected ${method} ${pathName}`);
    return {
      ok: true,
      status: 200,
      async text() {
        return JSON.stringify(payload);
      },
    };
  };
  const { createHostSettingsClient } = loadModule('src/features/agentHost/settingsClient.ts');
  const client = createHostSettingsClient({
    origin: 'http://127.0.0.1:9',
    session: 'ab'.repeat(16),
    fetchImpl,
  });
  const before = await client.getSettings();
  assert.equal(before.runtime.id, 'none');
  assert.equal(before.apiProvider.apiKeyConfigured, false);
  const afterRuntime = await client.setRuntime('grok-acp');
  assert.equal(afterRuntime.runtime.id, 'grok-acp');
  const afterProvider = await client.setProvider({ model: 'grok-4.7', apiKey: 'xai-secret-key-value' });
  assert.equal(afterProvider.apiProvider.apiKeyConfigured, true);
  assert.equal(afterProvider.apiProvider.model, 'grok-4.7');
  assert.equal(Object.prototype.hasOwnProperty.call(afterProvider.apiProvider, 'apiKey'), false);
  assert.deepEqual(
    calls.map((item) => [item.method, item.path, item.headers['X-Oviraptor-Session']]),
    [
      ['GET', '/api/settings', 'ab'.repeat(16)],
      ['POST', '/api/settings', 'ab'.repeat(16)],
      ['POST', '/api/settings/provider', 'ab'.repeat(16)],
      ['GET', '/api/settings', 'ab'.repeat(16)],
    ],
  );
  assert.equal(calls[2].body.provider, 'xai');
  assert.equal(calls[2].body.apiKey, 'xai-secret-key-value');
  assert.doesNotMatch(JSON.stringify(afterProvider), /xai-secret-key-value/);
});

test('unsupported runtime from host is rejected by the client', async () => {
  const { createHostSettingsClient } = loadModule('src/features/agentHost/settingsClient.ts');
  const client = createHostSettingsClient({
    origin: 'http://127.0.0.1:9',
    session: 'cd'.repeat(16),
    fetchImpl: async () => ({
      ok: true,
      status: 200,
      async text() {
        return JSON.stringify({ runtime: { id: 'custom' } });
      },
    }),
  });
  await assert.rejects(() => client.getSettings(), /agent_host_unsupported_runtime/);
});
