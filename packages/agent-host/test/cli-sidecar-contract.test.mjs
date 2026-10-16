import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomBytes } from 'node:crypto';

const cli = fileURLToPath(new URL('../src/cli.mjs', import.meta.url));

test('cli prints origin and session for desktop sidecar wiring', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-cli-sidecar-'));
  t.after(async () => { await rm(directory, { recursive: true, force: true }); });
  const token = randomBytes(32).toString('hex');
  const child = spawn(process.execPath, [cli, '--data-dir', directory, '--session-token', token, '--port', '0', '--runtime', 'none'], {
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  t.after(() => { try { child.kill('SIGKILL'); } catch { /* already exited */ } });
  let stdout = '';
  const ready = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(Error('cli sidecar contract timed out')), 8000);
    child.stdout.on('data', chunk => {
      stdout += chunk;
      if (stdout.includes('Oviraptor host:') && stdout.includes('Session:')) {
        clearTimeout(timer);
        resolve(true);
      }
    });
    child.on('exit', code => {
      clearTimeout(timer);
      reject(Error(`cli exited early with ${code}: ${stdout}`));
    });
  });
  assert.equal(ready, true);
  const origin = stdout.match(/Oviraptor host:\s*(\S+)/)?.[1];
  const session = stdout.match(/Session:\s*(\S+)/)?.[1];
  assert.match(origin ?? '', /^http:\/\/127\.0\.0\.1:\d+$/);
  assert.equal(session, token);
  const response = await fetch(`${origin}/api/status`, { headers: { 'X-Oviraptor-Session': token } });
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.protocolVersion, 1);
  child.kill('SIGKILL');
});
