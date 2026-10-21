import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

test('inventory search streams exact totals and bounded pages within a 32 MiB heap', { timeout: 45000 }, async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-assets-bounded-search-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const fixture = fileURLToPath(new URL('./fixtures/bounded-search.mjs', import.meta.url));
  const child = spawn(process.execPath, ['--max-old-space-size=32', fixture, directory], { cwd: directory, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '', errors = '';
  child.stdout.setEncoding('utf8'); child.stderr.setEncoding('utf8');
  child.stdout.on('data', part => { output = (output + part).slice(-8192); });
  child.stderr.on('data', part => { errors = (errors + part).slice(-8192); });
  t.after(() => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); });
  const exited = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', (code, signal) => resolve({ code, signal })); });
  assert.equal(exited.code, 0, `Bounded search child exited ${exited.code}/${exited.signal}: ${errors}`);
  const proof = JSON.parse(output);
  assert.equal(proof.passed, true); assert.equal(proof.seeded, 30000); assert.equal(proof.matchingProjectTotal, 30000);
  assert.equal(proof.filteredTotal, 30); assert.equal(proof.databaseRowsUnchanged, true); assert.equal(proof.oldSpaceMb, 32);
});
