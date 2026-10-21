// Compile only the production lifecycle helper against the already cached libc.
// No Cargo, bundle rebuild, desktop launch, user database, or process-name kills.
import { readdir, mkdtemp, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = fileURLToPath(new URL('../', import.meta.url));
const dependencies = join(root, 'src-tauri/target/debug/deps');
const libc = (await readdir(dependencies)).find(name => /^liblibc-[a-z0-9]+\.rlib$/.test(name));
if (!libc) throw Error('Cached libc missing; run after the normal Rust dependency build.');
const temporary = await mkdtemp('/private/tmp/oviraptor-lifecycle-regression-');
const harness = join(temporary, 'harness.rs'), binary = join(temporary, 'lifecycle-tests');
await writeFile(harness, `#[path = ${JSON.stringify(resolve(root, 'src-tauri/src/agent_host_lifecycle.rs'))}] mod lifecycle;\n`);
const compile = spawnSync('rustc', ['--edition=2021', '--test', harness, '--extern', `libc=${join(dependencies, libc)}`, '-o', binary], { stdio: 'inherit' });
if (compile.status !== 0) process.exit(compile.status ?? 1);
const tests = spawnSync(binary, ['--test-threads=1', '--nocapture'], { stdio: 'inherit', env: { ...process.env, TMPDIR: '/private/tmp', OVIRAPTOR_TEST_NODE: process.execPath } });
console.log(JSON.stringify({ temporary, binary, cargoExecuted: false, desktopAppLaunched: false }));
process.exitCode = tests.status ?? 1;
