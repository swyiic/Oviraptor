import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HostStore } from '../src/store.mjs';
import { LayaConnector, readLayaConfig, McpRegistry } from '../src/jev-connector.mjs';

// A minimal stdio MCP server speaking newline-delimited JSON-RPC, like the real
// laya-mlx-mcp-server (which surfaces laya_decide / laya_route / laya_shortlist …).
// It also echoes LAYA_PRELOAD back through a tools/call so we can prove the
// lazy-preload default env reached the child.
const MOCK = [
  "let buf='';process.stdin.setEncoding('utf8');",
  "process.stdin.on('data',c=>{buf+=c;let i;while((i=buf.indexOf('\\n'))>=0){const line=buf.slice(0,i).trim();buf=buf.slice(i+1);if(!line)continue;let m;try{m=JSON.parse(line);}catch{continue;}if(m.id==null)continue;let result;",
  "if(m.method==='initialize')result={protocolVersion:'2024-11-05',serverInfo:{name:'mock-laya'}};",
  "else if(m.method==='tools/list')result={tools:[{name:'laya_decide',description:'typed decision',inputSchema:{type:'object'}},{name:'laya_route',description:'route',inputSchema:{type:'object'}}]};",
  "else if(m.method==='tools/call')result={content:[{type:'text',text:JSON.stringify({tool:m.params&&m.params.name,preload:process.env.LAYA_PRELOAD||null})}]};",
  "else result={};",
  "process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result})+'\\n');}});",
].join('\n');

function writeMock(dir) { const p = join(dir, 'mock-laya.mjs'); writeFileSync(p, MOCK); return p; }

test('readLayaConfig defaults: command, lazy preload env, laya apiKeyEnv', () => {
  const store = new HostStore(':memory:');
  const cfg = readLayaConfig(store);
  assert.equal(cfg.command, 'laya-mlx-mcp-server');
  assert.equal(cfg.apiKeyEnv, 'LAYA_API_KEY');
  assert.equal(cfg.env.LAYA_PRELOAD, '0'); // handshake stays fast, no model download
  assert.equal(cfg.enabled, false);        // disabled by default
});

test('LayaConnector handshakes a stdio MCP server and surfaces prefixed laya_ tools', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'laya-'));
  const store = new HostStore(':memory:');
  try {
    const mock = writeMock(dir);
    const c = new LayaConnector(store);
    await c.configure({ enabled: true, command: process.execPath, args: [mock] });
    assert.equal(c.status().state, 'ready');
    const specs = c.toolSpecs();
    assert.ok(specs.some(s => s.name === 'laya_laya_decide')); // id prefix + tool name
    assert.ok(specs.some(s => s.name === 'laya_laya_route'));
    assert.ok(specs[0].description.includes('不授予任何权限')); // advisory wall stated
    assert.ok(c.owns('laya_laya_decide'));
    assert.ok(!c.owns('workspace_run'));
    assert.ok(!c.owns('jev_Choice'));
    const r = await c.callTool('laya_laya_decide', { head: 'q', options: ['a', 'b'] });
    assert.match(JSON.stringify(r), /laya_decide/);
    // The lazy-preload default env reached the child.
    assert.match(r.content[0].text, /"preload":"0"/);
    await c.stop();
    assert.equal(c.toolSpecs().length, 0); // stopped → no tools
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('config persists in host_settings laya_mcp; disabled exposes nothing; bad command degrades, no crash', async () => {
  const store = new HostStore(':memory:');
  const c = new LayaConnector(store);
  await c.configure({ enabled: false, command: 'laya-mlx-mcp-server' });
  assert.equal(c.toolSpecs().length, 0);
  assert.equal(JSON.parse(store.db.prepare("SELECT value FROM host_settings WHERE key='laya_mcp'").get().value).command, 'laya-mlx-mcp-server');
  await c.configure({ enabled: true, command: 'definitely-not-a-real-binary-xyz-123', args: [] });
  assert.ok(['error', 'starting', 'disabled'].includes(c.status().state));
  assert.equal(c.toolSpecs().length, 0);
  await assert.rejects(c.callTool('laya_laya_decide', {})); // not ready → rejects, not crash
  await c.stop();
});

test('McpRegistry exposes laya as a built-in, non-removable, advisory connector alongside jev', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'laya-reg-'));
  const store = new HostStore(':memory:');
  try {
    const mock = writeMock(dir);
    const reg = new McpRegistry(store);
    // laya listed as built-in + non-removable
    const listed = reg.list().connectors;
    const laya = listed.find(c => c.id === 'laya');
    assert.ok(laya); assert.equal(laya.builtin, true); assert.equal(laya.removable, false);
    // cannot be removed or re-added as a user connector (id reserved)
    await assert.rejects(reg.remove('laya'), /not_removable/);
    await assert.rejects(reg.add({ id: 'laya', command: 'x', enabled: false }), /reserved/);
    // once enabled, its tools flow through the registry's merged surface + routing
    await reg.laya.configure({ enabled: true, command: process.execPath, args: [mock] });
    assert.ok(reg.toolSpecs().some(s => s.name === 'laya_laya_decide'));
    assert.ok(reg.owns('laya_laya_decide'));
    const r = await reg.callTool('laya_laya_decide', {});
    assert.match(JSON.stringify(r), /laya_decide/);
    await reg.stopAll();
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
