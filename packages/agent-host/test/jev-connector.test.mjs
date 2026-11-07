import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { JevConnector } from '../src/jev-connector.mjs';

// A minimal stdio MCP server speaking newline-delimited JSON-RPC, like jev-studio.
const MOCK = [
  "let buf='';",
  "process.stdin.setEncoding('utf8');",
  "process.stdin.on('data',c=>{buf+=c;let i;while((i=buf.indexOf('\\n'))>=0){const line=buf.slice(0,i).trim();buf=buf.slice(i+1);if(!line)continue;let m;try{m=JSON.parse(line);}catch{continue;}if(m.id==null)continue;let result;",
  "if(m.method==='initialize')result={protocolVersion:'2024-11-05',serverInfo:{name:'mock-jev'}};",
  "else if(m.method==='tools/list')result={tools:[{name:'Choice',description:'typed choice',inputSchema:{type:'object',properties:{options:{type:'array'}}}},{name:'Score',description:'score',inputSchema:{type:'object'}}]};",
  "else if(m.method==='tools/call')result={content:[{type:'text',text:JSON.stringify({picked:(m.params&&m.params.arguments)||{}})}]};",
  "else result={};",
  "process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result})+'\\n');}});",
].join('\n');

function writeMock(dir) { const p = join(dir, 'mock-jev.mjs'); writeFileSync(p, MOCK); return p; }

test('connector handshakes a stdio MCP server and surfaces prefixed decision tools', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'jev-'));
  const store = new HostStore(':memory:');
  try {
    const mock = writeMock(dir);
    const c = new JevConnector(store);
    await c.configure({ enabled: true, command: process.execPath, args: [mock] });
    assert.equal(c.status().state, 'ready');
    const specs = c.toolSpecs();
    assert.ok(specs.some(s => s.name === 'jev_Choice'));
    assert.ok(specs.some(s => s.name === 'jev_Score'));
    assert.ok(specs[0].description.includes('不授予任何权限')); // advisory wall stated
    assert.ok(c.owns('jev_Choice'));
    assert.ok(!c.owns('workspace_run'));
    const r = await c.callTool('jev_Choice', { options: ['a', 'b'] });
    assert.match(JSON.stringify(r), /picked/);
    await c.callTool('jev_Score', {}); // second tool works too
    await c.stop();
    assert.equal(c.toolSpecs().length, 0); // stopped → no tools
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('config persists in host_settings; disabled exposes nothing; bad command degrades, no crash', async () => {
  const store = new HostStore(':memory:');
  const c = new JevConnector(store);
  await c.configure({ enabled: false, command: 'jev-studio' });
  assert.equal(c.toolSpecs().length, 0);
  // persisted
  assert.equal(JSON.parse(store.db.prepare("SELECT value FROM host_settings WHERE key='jev_mcp'").get().value).command, 'jev-studio');
  // bad binary must not throw or hang — degrades to error/disabled with no tools
  await c.configure({ enabled: true, command: 'definitely-not-a-real-binary-xyz-123', args: [] });
  assert.ok(['error', 'starting', 'disabled'].includes(c.status().state));
  assert.equal(c.toolSpecs().length, 0);
  await assert.rejects(c.callTool('jev_Choice', {})); // not ready → rejects, not crash
  await c.stop();
});

test('api key reaches child env as TYPESAFE_API_KEY, extra env passes, status never leaks the value', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'jev-'));
  const store = new HostStore(':memory:');
  try {
    // Mock echoes selected env vars back through a tools/call result.
    const mock = join(dir, 'echo-env.mjs');
    writeFileSync(mock, [
      "let buf='';process.stdin.setEncoding('utf8');",
      "process.stdin.on('data',c=>{buf+=c;let i;while((i=buf.indexOf('\\n'))>=0){const line=buf.slice(0,i).trim();buf=buf.slice(i+1);if(!line)continue;let m;try{m=JSON.parse(line);}catch{continue;}if(m.id==null)continue;let result;",
      "if(m.method==='initialize')result={protocolVersion:'2024-11-05',serverInfo:{name:'echo'}};",
      "else if(m.method==='tools/list')result={tools:[{name:'Echo',description:'echo',inputSchema:{type:'object'}}]};",
      "else if(m.method==='tools/call')result={content:[{type:'text',text:JSON.stringify({key:process.env.TYPESAFE_API_KEY||null,extra:process.env.JEV_EXTRA||null})}]};",
      "else result={};",
      "process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result})+'\\n');}});",
    ].join('\n'));
    const c = new JevConnector(store);
    await c.configure({ enabled: true, command: process.execPath, args: [mock], apiKey: 'sk-secret-123', env: { JEV_EXTRA: 'xyz' } });
    assert.equal(c.status().state, 'ready');
    const st = c.status();
    assert.equal(st.hasApiKey, true);          // presence exposed
    assert.equal(st.apiKeyEnv, 'TYPESAFE_API_KEY'); // default env name
    assert.deepEqual(st.envKeys, ['JEV_EXTRA']);
    assert.ok(!JSON.stringify(st).includes('sk-secret-123')); // value NEVER leaked
    const r = await c.callTool('jev_Echo', {});
    const text = r.content[0].text;
    assert.match(text, /sk-secret-123/); // key actually reached the child env
    assert.match(text, /xyz/);            // extra env reached the child
    await c.stop();
    await c.configure({ apiKey: '' });     // empty clears the key
    assert.equal(c.status().hasApiKey, false);
    await c.stop();
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
