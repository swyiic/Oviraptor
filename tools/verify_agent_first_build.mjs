import assert from 'node:assert/strict';
import { cp, mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';

// Run against only the generated distribution, outside the repository and node_modules.
const source = resolve(process.argv[2] ?? '.agent-build/agent-first');
const scratch = await mkdtemp(join(tmpdir(), 'oviraptor-independent-build-'));
let child, ended = false, completion;
try {
  const distribution = join(scratch, 'distribution');
  await cp(source, distribution, { recursive: true });
  const packages = (await readdir(join(distribution, 'packages'))).sort();
  assert.deepEqual(packages, ['agent-host', 'agent-ui', 'app-sdk', 'runtime-acp', 'skills-library']);
  child = spawn(process.execPath, [join(distribution, 'packages/agent-host/src/cli.mjs'), '--data-dir', join(scratch, 'data'), '--runtime', 'none'], {
    cwd: scratch, env: { PATH: process.env.PATH ?? '', NODE_NO_WARNINGS: '1' }, stdio: ['pipe', 'pipe', 'pipe'],
  });
  let output = '', errors = '';
  child.stdout.on('data', bytes => { output += bytes; }); child.stderr.on('data', bytes => { errors += bytes; });
  child.stdin.on('error', () => {});
  completion = new Promise((yes, no) => { child.once('error', no); child.once('exit', (code, signal) => { ended = true; yes({ code, signal }); }); });
  const deadline = Date.now() + 5000;
  while (!/Oviraptor host: (http:\/\/127\.0\.0\.1:\d+)/.test(output)) {
    if (ended || Date.now() >= deadline) throw Error('Independent distribution failed to start');
    await new Promise(yes => setTimeout(yes, 10));
  }
  const origin = output.match(/Oviraptor host: (http:\/\/127\.0\.0\.1:\d+)/)[1];
  const page = await (await fetch(origin)).text();
  const token = page.match(/window\.__OVIRAPTOR_HOST__=(\{[^<]+\})<\/script>/)?.[1];
  assert(token, 'Built HTML includes the process-local API credential');
  const headers = { 'X-Oviraptor-Session': JSON.parse(token).token, 'Content-Type': 'application/json' };
  const request = async (path, body) => {
    const response = await fetch(origin + path, { method: body === undefined ? 'GET' : 'POST', headers, body: body === undefined ? undefined : JSON.stringify(body) });
    assert.equal(response.status, 200, `Standalone API failed: ${path}`); return response.json();
  };
  assert.deepEqual((await request('/api/apps')).items, []);
  const project = await request('/api/projects', { name: 'Synthetic standalone project' });
  const conversation = await request('/api/conversations', { projectId: project.id });
  assert.equal((await request(`/api/conversations/${conversation.id}/messages`, { projectId: project.id, text: 'Synthetic save-only message' })).savedOnly, true);
  assert.equal((await request(`/api/conversations/${conversation.id}/messages`)).items.length, 1);
  const installed = await request('/api/skills/bundled', {});
  assert.equal(installed.skillCount, 36); assert.equal(installed.fileCount, 443);
  assert.equal((await request('/api/skills')).items.length, 36);
  assert.deepEqual((await request('/api/apps')).items, []);
  assert.equal((await request('/api/status')).runtime.state, 'unconfigured');
  assert.equal((await request('/api/runtime/metrics')).total, 0);
  child.kill('SIGTERM'); assert.equal((await completion).code, 0); assert.equal(errors, '');
  process.stdout.write(JSON.stringify({ accepted: true, packages, installedApplications: 0, savedMessages: 1, bundledSkills: 36, bundledFiles: 443, runtimeStarted: false, modelRequests: 0 }) + '\n');
} finally {
  if (child && !ended) { child.kill('SIGTERM'); await completion; }
  await rm(scratch, { recursive: true, force: true });
}
