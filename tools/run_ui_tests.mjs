// Discover suites instead of keeping a second, easily stale hand-written list.
import { readdir } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../', import.meta.url));
const suites = (await readdir(new URL('./', import.meta.url))).filter(name => /^test_.+\.cjs$/u.test(name)).sort().map(name => `tools/${name}`);
if (!suites.length) throw Error('No UI regression suites discovered');
const child = spawn(process.execPath, ['--test', '--test-concurrency=1', ...suites], { cwd: root, stdio: 'inherit', env: process.env });
child.once('error', error => { process.stderr.write(error.message + '\n'); process.exitCode = 1; });
child.once('exit', (code, signal) => { process.exitCode = signal ? 1 : code ?? 1; });
