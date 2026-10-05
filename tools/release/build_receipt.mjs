// Observe a local macOS build. This is unsigned local evidence, not attestation.
import { lstat, mkdir, mkdtemp, open, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { digest, inventoryFiles } from './file_inventory.mjs';
import { snapshotAppBundle } from './compare_app_bundles.mjs';

const sourceDirectories = ['src', 'public', 'scripts', 'tools', 'src-tauri/src',
  'src-tauri/resources', 'src-tauri/icons', 'src-tauri/capabilities', 'src-tauri/tests'];
const requiredFiles = ['package.json', 'package-lock.json', 'index.html', 'vite.config.ts',
  'tsconfig.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/tauri.conf.json'];
const json = value => `${JSON.stringify(value, null, 2)}\n`;

export async function snapshotBuildInputs(root) {
  const roots = [...sourceDirectories];
  for (const directory of ['', 'src-tauri']) {
    for (const name of (await readdir(path.join(root, directory))).sort()) {
      if (/^\.env(?:\.|$)/.test(name) && name !== '.env.example') throw new Error('local_env_file_not_supported');
      if (/\.(?:json|json5|toml|lock|[cm]?js|ts|rs|html|plist)$/.test(name)
        || ['.npmrc', 'rust-toolchain', '.cargo'].includes(name)) {
        roots.push(directory ? `${directory}/${name}` : name);
      }
    }
  }
  for (const file of requiredFiles) if (!roots.includes(file)) throw new Error(`missing_build_input:${file}`);
  const entries = await inventoryFiles(root, roots, 'build_input');
  return { schemaVersion: 1, scope: 'reviewed_project_inputs', roots: roots.sort(), entries,
    treeSha256: digest(JSON.stringify(entries)) };
}

// Explicit environment allowlist: never save inherited secrets or pass build
// overrides such as NODE_OPTIONS, TAURI_CONFIG or RUSTC_WRAPPER implicitly.
export function buildEnvironment(inherited, targetDir) {
  const env = {};
  for (const key of ['PATH', 'HOME', 'USER', 'LOGNAME', 'TMPDIR', 'SHELL', 'LANG', 'LC_ALL', 'TERM']) {
    if (inherited[key] !== undefined) env[key] = inherited[key];
  }
  return { ...env, CI: 'true', CARGO_BUILD_JOBS: '1', CARGO_NET_OFFLINE: 'true', CARGO_TARGET_DIR: targetDir };
}

async function runBuild(command, options) {
  const log = await open(options.logPath, 'wx', 0o600);
  try {
    return await new Promise((resolve, reject) => {
      const child = spawn(command[0], command.slice(1), {
        cwd: options.root, env: options.env, stdio: ['ignore', log.fd, log.fd], shell: false,
      });
      child.once('error', reject);
      child.once('close', (exitCode, signal) => resolve({ exitCode, signal }));
    });
  } finally { await log.close(); }
}

async function validateProject(root) {
  const pkg = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
  const config = JSON.parse(await readFile(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  if (pkg.scripts?.build !== 'vue-tsc --noEmit && vite build'
    || config.productName !== 'Oviraptor' || config.build?.beforeBuildCommand !== 'npm run build'
    || config.build?.frontendDist !== '../dist' || config.build?.beforeBundleCommand)
    throw new Error('unsupported_build_configuration');
  // Platform overrides/hooks need an explicit review before extending this workflow.
  if ((await readdir(path.join(root, 'src-tauri'))).some(name => /^tauri\..+\.conf\./.test(name)
    || name === 'Tauri.toml' || name === 'tauri.conf.json5')) throw new Error('unsupported_build_override');
}

export async function recordBuild({ root, evidenceParent = tmpdir(), execute = runBuild, onStarted = () => {} }) {
  if (process.platform !== 'darwin') throw new Error('recorded_build_requires_macos');
  root = path.resolve(root);
  if ((await lstat(root)).isSymbolicLink()) throw new Error('build_root_symlink');
  root = await realpath(root);
  evidenceParent = await realpath(evidenceParent);
  const relation = path.relative(root, evidenceParent);
  if (!relation || (!relation.startsWith(`..${path.sep}`) && relation !== '..' && !path.isAbsolute(relation)))
    throw new Error('evidence_directory_must_be_outside_project');
  const evidenceDir = await mkdtemp(path.join(evidenceParent, 'oviraptor-build-'));
  const receiptPath = path.join(evidenceDir, 'receipt.json');
  const targetDir = path.join(evidenceDir, 'target');
  const command = [process.execPath, path.join(root, 'node_modules/@tauri-apps/cli/tauri.js'),
    'build', '--bundles', 'app', '--ci', '--', '--locked', '--offline', '-j', '1'];
  const receipt = { schemaVersion: 1, status: 'in_progress', startedAt: new Date().toISOString(),
    root, command, nodeVersion: process.version, platform: process.platform, architecture: process.arch,
    buildPolicy: { cargoJobs: 1, cargoOffline: true, cargoLocked: true, freshTarget: true, inheritedEnvironment: 'allowlisted' },
    evidenceDir, logPath: path.join(evidenceDir, 'build.log'),
    sourceBinding: 'not_verified', runtimeValidation: 'not_performed', installedValidation: 'not_performed',
    assurance: 'unsigned_local_observation',
    limits: ['not_hermetic', 'dependencies_and_user_toolchain_config_not_snapshotted',
      'input_scope_is_explicit_not_dependency_closure', 'endpoint_snapshots_do_not_detect_change_then_restore',
      'not_signature_validation', 'not_release_acceptance'],
  };
  await writeFile(receiptPath, json(receipt), { flag: 'wx', mode: 0o600 });
  try {
    onStarted(receiptPath);
    await validateProject(root);
    const before = await snapshotBuildInputs(root);
    await writeFile(path.join(evidenceDir, 'inputs-before.json'), json(before), { flag: 'wx', mode: 0o600 });
    receipt.inputsBeforeSha256 = before.treeSha256;
    await mkdir(targetDir); // Fresh output: a successful no-op cannot adopt an old bundle.
    receipt.build = await execute(command, { root, targetDir, env: buildEnvironment(process.env, targetDir), logPath: receipt.logPath });
    const after = await snapshotBuildInputs(root);
    await writeFile(path.join(evidenceDir, 'inputs-after.json'), json(after), { flag: 'wx', mode: 0o600 });
    receipt.inputsAfterSha256 = after.treeSha256;
    receipt.inputsUnchanged = before.treeSha256 === after.treeSha256;
    if (receipt.build?.exitCode !== 0 || receipt.build.signal) throw new Error('build_did_not_succeed');
    if (!receipt.inputsUnchanged) throw new Error('build_inputs_changed');
    const bundle = await snapshotAppBundle(path.join(targetDir, 'release/bundle/macos/Oviraptor.app'));
    await writeFile(path.join(evidenceDir, 'bundle.json'), json(bundle), { flag: 'wx', mode: 0o600 });
    receipt.bundleRoot = bundle.root;
    receipt.bundleTreeSha256 = bundle.treeSha256;
    receipt.status = 'build_observed';
  } catch (error) {
    receipt.status = 'failed';
    // Keep diagnostics local; CLI reports only the status and evidence location.
    receipt.failure = String(error);
  }
  receipt.finishedAt = new Date().toISOString();
  await writeFile(receiptPath, json(receipt), { mode: 0o600 });
  return { ...receipt, receiptPath };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    if (process.argv.length !== 3) throw new Error('usage: build_receipt.mjs PROJECT_ROOT');
    const receipt = await recordBuild({ root: path.resolve(process.argv[2]),
      onStarted: receiptPath => console.log(JSON.stringify({ status: 'in_progress', receiptPath })) });
    console.log(JSON.stringify({ status: receipt.status, receiptPath: receipt.receiptPath }, null, 2));
    process.exitCode = receipt.status === 'build_observed' ? 0 : 1;
  } catch {
    console.error('Recorded build could not start; check platform, project path and evidence directory.');
    process.exitCode = 1;
  }
}
