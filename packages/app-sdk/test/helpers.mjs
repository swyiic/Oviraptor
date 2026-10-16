import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
export function manifest(overrides = {}) {
  return { schemaVersion: 1, id: 'org.oviraptor.test', version: '1.0.0', hostApi: '^1.0.0', entry: 'worker.mjs', dataSchemaVersion: 1,
    pages: [{ id: 'home', title: 'Test', path: 'page.html' }],
    permissions: [{ id: 'test.invoke', description: 'Invoke local test tool' }],
    tools: [{ name: 'echo', description: 'Echo', inputSchema: { type: 'object', properties: { text: { type: 'string', maxLength: 32 } }, required: ['text'], additionalProperties: false }, permission: 'test.invoke' }],
    lifecycle: { start: true, stop: true }, files: [{ path: 'worker.mjs', sha256: 'a'.repeat(64) }, { path: 'page.html', sha256: 'b'.repeat(64) }], ...overrides };
}
export const context = (operationId = 'op-1', hostBoundProjectId = 1) => ({ hostBoundProjectId, grantRevision: 2, operationId });
export const start = (dataDir, appId = 'org.oviraptor.test') => ({ dataDir, context: { hostApiVersion: '1.0.0', appId } });
export async function temporary(t, prefix = 'oviraptor-sdk-') {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}
export function rpcProcess(directory, { permission = false, dataDir, entry = 'worker.mjs' } = {}) {
  const args = permission ? ['--permission', `--allow-fs-read=${directory}`, `--allow-fs-read=${dataDir}`, `--allow-fs-write=${dataDir}`, join(directory, entry)] : [join(directory, entry)];
  const child = spawn(process.execPath, args, { cwd: directory, stdio: ['pipe', 'pipe', 'pipe'], env: { PATH: process.env.PATH } });
  const waiting = new Map();
  const unexpected = [];
  const all = [];
  let nextId = 1;
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', part => { stderr += part; });
  const lines = createInterface({ input: child.stdout });
  lines.on('line', line => {
    let response;
    try { response = JSON.parse(line); } catch { unexpected.push(line); return; }
    all.push(response);
    const pending = waiting.get(response.id);
    if (pending) { waiting.delete(response.id); clearTimeout(pending.timer); pending.resolve(response); }
  });
  const exited = new Promise(resolve => child.once('exit', (code, signal) => {
    for (const pending of waiting.values()) { clearTimeout(pending.timer); pending.reject(new Error(`Worker exited ${code}/${signal}: ${stderr}`)); }
    waiting.clear(); resolve({ code, signal });
  }));
  function request(method, params, id = nextId++) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { waiting.delete(id); reject(new Error(`RPC ${method} timed out: ${stderr}`)); }, 5000);
      waiting.set(id, { resolve, reject, timer });
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  }
  return { child, request, all, unexpected, exited, get stderr() { return stderr; },
    async end() { child.stdin.end(); return exited; },
    async kill() { child.kill('SIGKILL'); await exited; }, args };
}
