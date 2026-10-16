import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { HostStore } from '../src/store.mjs';
import { AgentHost } from '../src/service.mjs';

const cli = fileURLToPath(new URL('../src/cli.mjs', import.meta.url));
async function directory(t) { const path = await mkdtemp(join(tmpdir(), 'oviraptor-runtime-cli-')); t.after(() => rm(path, { recursive: true, force: true })); return path; }
function seed(path, selected) {
  const store = new HostStore(join(path, 'host.sqlite'));
  store.db.prepare("INSERT INTO host_settings(key,value) VALUES('runtime',?)").run(selected);
  store.db.prepare("INSERT INTO host_settings(key,value) VALUES('runtime_revision','7')").run(); store.close();
}
function factory() {
  const counts = { constructions: 0, starts: 0, closes: 0 };
  return { counts, create() { counts.constructions++; return { command: 'synthetic-runtime', status: { pid: null },
    async start() { counts.starts++; throw Error('No runtime transport is allowed in selection tests'); },
    async close() { counts.closes++; } }; } };
}
function processCli(t, path, args = [], execArgs = []) {
  // No account or auth environment is forwarded, and no message is submitted.
  const child = spawn(process.execPath, [...execArgs, cli, '--data-dir', path, '--port', '0', ...args], { env: { PATH: process.env.PATH ?? '', NODE_NO_WARNINGS: '1' }, stdio: ['pipe', 'pipe', 'pipe'] });
  let stdout = '', stderr = '', exited = false;
  const completion = new Promise(resolve => child.once('exit', (code, signal) => { exited = true; resolve({ code, signal }); }));
  child.stdout.on('data', chunk => { stdout += chunk; }); child.stderr.on('data', chunk => { stderr += chunk; }); child.stdin.on('error', () => {});
  t.after(async () => { if (!exited) { child.kill('SIGTERM'); await completion; } });
  return { child, completion, get stdout() { return stdout; }, get stderr() { return stderr; }, async ready() {
    const deadline = Date.now() + 3000;
    await new Promise((resolve, reject) => { const check = () => { if (/^Runtime: .+\n/m.test(stdout)) return resolve(); if (exited || Date.now() > deadline) return reject(Error('Synthetic CLI startup did not become ready')); setTimeout(check, 5); }; check(); });
  }, async stop() { child.kill('SIGTERM'); const result = await completion; assert.equal(result.code, 0, stderr); } };
}

test('explicit none overrides saved Grok and an injected runtime without constructing or starting a transport', async t => {
  const path = await directory(t); seed(path, 'grok-acp'); const runtime = factory();
  const host = new AgentHost({ dataDir: path, runtimeSelection: 'none', runtimeFactory: runtime.create });
  t.after(() => host.close());
  assert.equal(host.settings().runtime.id, 'none'); assert.equal(host.runtime, null); assert.deepEqual(runtime.counts, { constructions: 0, starts: 0, closes: 0 });
  assert.equal(host.runtimeIdentity(), 'none:7');
  assert.equal(host.store.db.prepare("SELECT value FROM host_settings WHERE key='runtime'").get().value, 'grok-acp', 'A startup override does not silently persist a new preference');
  const injectedPath = await directory(t), injected = runtime.create();
  const overridden = new AgentHost({ dataDir: injectedPath, runtime: injected, runtimeSelection: 'none', runtimeFactory: runtime.create });
  t.after(() => overridden.close()); assert.equal(overridden.runtime, null); assert.equal(runtime.counts.constructions, 1); assert.equal(runtime.counts.starts, 0);
});

test('omitting selection restores persisted Grok while explicit Grok overrides saved none without starting/authenticating', async t => {
  for (const [saved, runtimeSelection] of [['grok-acp', undefined], ['none', 'grok-acp']]) {
    const path = await directory(t); seed(path, saved); const runtime = factory();
    const host = new AgentHost({ dataDir: path, runtimeFactory: runtime.create, runtimeSelection }); t.after(() => host.close());
    assert.equal(host.settings().runtime.id, 'grok-acp'); assert.equal(host.status().runtime.state, 'configured'); assert.equal(host.runtimeIdentity(), 'grok-acp:7');
    assert.deepEqual(runtime.counts, { constructions: 1, starts: 0, closes: 0 });
    assert.equal(host.store.db.prepare("SELECT value FROM host_settings WHERE key='runtime'").get().value, saved);
  }
});

test('startup override preserves explicit settings generation changes and clears old MCP/session leases', async t => {
  const path = await directory(t); seed(path, 'grok-acp'); const runtime = factory();
  const host = new AgentHost({ dataDir: path, runtimeFactory: runtime.create, runtimeSelection: 'none' }); t.after(() => host.close());
  host.mcpScopes.set('synthetic-old-scope', {}); host.loadedSessions.set('synthetic-old-session', 1);
  await host.setRuntime('grok-acp'); const first = host.runtimeIdentity();
  assert.equal(first, 'grok-acp:8'); assert.equal(host.mcpScopes.size, 0); assert.equal(host.loadedSessions.size, 0);
  await host.setRuntime('none'); await host.setRuntime('grok-acp');
  assert.equal(host.runtimeIdentity(), 'grok-acp:10'); assert.notEqual(host.runtimeIdentity(), first); assert.equal(runtime.counts.starts, 0);
  assert.equal(host.store.db.prepare("SELECT value FROM host_settings WHERE key='runtime_revision'").get().value, '10');
});

test('invalid or unavailable constructor selections are rejected before creating local state', async t => {
  const parent = await directory(t);
  for (const [runtimeSelection, code] of [['unknown-runtime', 'host_runtime_invalid'], ['grok-acp', 'host_runtime_unavailable']]) {
    const path = join(parent, runtimeSelection);
    assert.throws(() => new AgentHost({ dataDir: path, runtimeSelection }), { code }); await assert.rejects(stat(path), { code: 'ENOENT' });
  }
});

test('real CLI distinguishes explicit none from omitted restore and logs the Host actual selection', async t => {
  for (const [saved, args, line] of [
    ['grok-acp', ['--runtime', 'none'], 'Runtime: unconfigured (save-only chat)'],
    ['grok-acp', [], 'Runtime: grok-acp (not connected until an explicit message)'],
    ['none', ['--runtime', 'grok-acp'], 'Runtime: grok-acp (not connected until an explicit message)'],
  ]) {
    const path = await directory(t); seed(path, saved); const peer = processCli(t, path, args); await peer.ready();
    assert(peer.stdout.includes(line), peer.stdout); assert.equal(peer.stderr, ''); await peer.stop();
    const store = new HostStore(join(path, 'host.sqlite'));
    assert.equal(store.db.prepare("SELECT value FROM host_settings WHERE key='runtime'").get().value, saved); store.close();
  }
});

test('real CLI rejects invalid and missing runtime values before opening a database', async t => {
  for (const args of [['--runtime', 'unknown'], ['--runtime']]) {
    const path = await directory(t), peer = processCli(t, path, args);
    assert.equal((await peer.completion).code, 2); assert.equal(peer.stdout, ''); assert.equal(peer.stderr, 'Expected --runtime none or --runtime grok-acp.\n');
    await assert.rejects(stat(join(path, 'host.sqlite')), { code: 'ENOENT' });
  }
});
test('observed CLI readiness guarantees graceful signal handling even if preempted immediately after stdout',async t=>{
  const path=await directory(t),hook=join(path,'pause-after-ready.mjs');
  await writeFile(hook,"const original=process.stdout.write;process.stdout.write=function(chunk,...args){const result=Reflect.apply(original,this,[chunk,...args]);if(String(chunk).includes('Oviraptor host:'))Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,300);return result;};\n");
  const peer=processCli(t,join(path,'data'),['--runtime','none'],['--import',hook]);
  await peer.ready();await peer.stop();assert.equal(peer.stderr,'');
});
