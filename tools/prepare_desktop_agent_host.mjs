// Stage the independent host produced by build:app-packages + build:agent-first.
// Only generated resources are published. Node remains an explicit external runtime.
import { cp, lstat, mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import { dirname, extname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const repositoryRoot = fileURLToPath(new URL('../', import.meta.url));
export const DESKTOP_HOST_RESOURCE = 'resources/agent-host';
export const DISTRIBUTION_RECEIPT = 'desktop-distribution.json';
const PACKAGE_NAMES = ['agent-host', 'agent-ui', 'app-sdk', 'runtime-acp', 'skills-library'];
const REQUIRED = [
  'packages/agent-host/src/cli.mjs', 'packages/agent-host/bundled-apps.json', 'packages/agent-host/shell-apps.json',
  'packages/agent-ui/dist/index.html', 'packages/agent-ui/dist/build-manifest.json',
  'packages/app-sdk/src/index.mjs', 'packages/app-sdk/src/worker.mjs', 'packages/app-sdk/bin/pack-app.mjs',
  'packages/runtime-acp/index.mjs', 'packages/skills-library/src/index.mjs',
  'packages/skills-library/resources/compound-engineering.json.gz', 'packages/skills-library/resources/COMPOUND-ENGINEERING-LICENSE',
];
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const json = async path => JSON.parse(await readFile(path, 'utf8'));
const inside = (root, path) => { const rel = relative(root, path); return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel)); };
const fail = message => { throw new Error(`desktop_agent_host: ${message}`); };
const validatePackageDirectory = async directory => (await import('../packages/app-sdk/src/index.mjs')).validatePackageDirectory(directory);

export function requireNode24(version = process.versions.node) {
  if (!/^\d+\./.test(version) || Number(version.split('.')[0]) < 24) fail('Node.js 24+ is required; no older-runtime fallback is supported');
}

async function inventory(root, base = root) {
  const info = await lstat(root);
  if (info.isSymbolicLink()) fail(`symlinks are not packaged: ${relative(base, root)}`);
  if (info.isFile()) {
    const bytes = await readFile(root);
    return [{ path: relative(base, root).split('\\').join('/'), sha256: hash(bytes), bytes: bytes.length }];
  }
  if (!info.isDirectory()) fail(`unsupported resource: ${relative(base, root)}`);
  const files = [];
  for (const entry of (await readdir(root)).sort()) files.push(...await inventory(join(root, entry), base));
  return files;
}

async function compareSource(source, built) {
  const info = await lstat(source);
  if (info.isDirectory()) {
    const expected = await inventory(source), actual = await inventory(built);
    if (JSON.stringify(expected) !== JSON.stringify(actual)) fail(`stale build directory: ${source}; run build:app-packages and build:agent-first`);
  } else if (hash(await readFile(source)) !== hash(await readFile(built))) fail(`stale build file: ${source}`);
}

async function checkBuild(repository, built) {
  for (const name of ['agent-host', 'app-sdk', 'skills-library']) {
    await compareSource(join(repository, 'packages', name, 'src'), join(built, 'packages', name, 'src'));
    await compareSource(join(repository, 'packages', name, 'package.json'), join(built, 'packages', name, 'package.json'));
  }
  for (const path of ['packages/app-sdk/bin', 'packages/runtime-acp/index.mjs', 'packages/runtime-acp/package.json', 'packages/agent-host/bundled-apps.json', 'packages/agent-host/shell-apps.json']) await compareSource(join(repository, path), join(built, path));
  await compareSource(join(repository, 'packages/agent-ui/src'), join(built, 'packages/agent-ui/dist/src'));
  await compareSource(join(repository, 'packages/agent-ui/index.html'), join(built, 'packages/agent-ui/dist/index.html'));
  for (const name of ['compound-engineering.json.gz', 'COMPOUND-ENGINEERING-LICENSE']) await compareSource(join(repository, 'src-tauri/resources/skill-packages', name), join(built, 'packages/skills-library/resources', name));
  const manifest = await json(join(built, 'packages/agent-host/bundled-apps.json'));
  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.apps) || manifest.apps.length !== 2) fail('expected the two explicitly listed builtin v1 applications');
  for (const entry of manifest.apps) {
    if (!/^apps\/[A-Za-z0-9._-]+$/.test(entry.package) || entry.package.includes('..')) fail('invalid bundled application path');
    const directory = join(built, entry.package), packaged = await validatePackageDirectory(directory);
    if (packaged.manifest.schemaVersion !== 1 || packaged.manifest.id !== entry.id) fail(`bundled application identity mismatch: ${entry.id}`);
    const shortName = entry.id.replace(/^org\.oviraptor\./, '');
    const source = join(repository, 'apps', shortName), spec = await json(join(source, 'app.package.json'));
    const { files, ...contract } = packaged.manifest;
    if (JSON.stringify(contract) !== JSON.stringify(spec.manifest)) fail(`stale application manifest: ${entry.id}`);
    for (const file of spec.files) await compareSource(join(source, file), join(directory, file));
    for (const file of files.filter(file => file.path.startsWith('sdk/'))) await compareSource(join(repository, 'packages/app-sdk/src', file.path.slice(4)), join(directory, file.path));
  }
  return manifest;
}

async function verifyContents(root) {
  for (const path of REQUIRED) if (!(await lstat(join(root, path))).isFile()) fail(`required resource missing: ${path}`);
  const files = await inventory(root);
  // Relative ESM dependencies must resolve entirely inside the staged tree.
  for (const file of files.filter(file => ['.mjs', '.js', '.html'].includes(extname(file.path)))) {
    const source = await readFile(join(root, file.path), 'utf8');
    for (const match of source.matchAll(/\b(?:from\s*|import\s*\(\s*|import\s*)['"](\.{1,2}\/[^'"]+)['"]/g)) {
      const target = resolve(dirname(join(root, file.path)), match[1]);
      if (!inside(root, target) || !(await lstat(target)).isFile()) fail(`unresolved bundled import: ${file.path} -> ${match[1]}`);
    }
  }
  const ui = await json(join(root, 'packages/agent-ui/dist/build-manifest.json'));
  for (const file of ui.files ?? []) {
    const path = resolve(root, 'packages/agent-ui/dist', file.path);
    if (!inside(join(root, 'packages/agent-ui/dist'), path) || hash(await readFile(path)) !== file.sha256) fail('agent-ui build manifest mismatch');
  }
  const bundled = await json(join(root, 'packages/agent-host/bundled-apps.json'));
  for (const app of bundled.apps) {
    if (!/^apps\/[A-Za-z0-9._-]+$/.test(app.package) || app.package.includes('..')) fail('invalid bundled application path');
    const packaged = await validatePackageDirectory(join(root, app.package));
    if (packaged.manifest.id !== app.id || packaged.manifest.schemaVersion !== 1) fail('builtin application identity mismatch');
  }
  return files;
}

export async function verifyDesktopDistribution(root) {
  const receipt = await json(join(root, DISTRIBUTION_RECEIPT));
  if (receipt.schemaVersion !== 1 || receipt.minimumNodeMajor !== 24 || receipt.nodeRuntimeBundled !== false) fail('invalid distribution receipt');
  const files = (await verifyContents(root)).filter(file => file.path !== DISTRIBUTION_RECEIPT);
  if (JSON.stringify(receipt.files) !== JSON.stringify(files)) fail('distribution digest mismatch');
  return receipt;
}

export async function prepareDesktopAgentHost({ repository = repositoryRoot, outputDirectory = join(repository, 'src-tauri', DESKTOP_HOST_RESOURCE) } = {}) {
  requireNode24();
  repository = resolve(repository); outputDirectory = resolve(outputDirectory);
  const built = join(repository, '.agent-build/agent-first');
  if (inside(repository, outputDirectory) && outputDirectory !== join(repository, 'src-tauri', DESKTOP_HOST_RESOURCE)) fail('repository output must use the ignored desktop resource directory');
  if (inside(outputDirectory, repository) || inside(built, outputDirectory) || inside(outputDirectory, built)) fail('stage output must be separate from source/build inputs');
  const manifest = await checkBuild(repository, built);
  const builtSnapshot = JSON.stringify(await inventory(built));
  const stage = `${outputDirectory}.staging-${randomUUID()}`;
  await mkdir(stage, { recursive: true });
  try {
    for (const name of PACKAGE_NAMES) await cp(join(built, 'packages', name), join(stage, 'packages', name), { recursive: true, dereference: false });
    await cp(join(repository, 'packages/agent-ui/package.json'), join(stage, 'packages/agent-ui/package.json'));
    for (const entry of manifest.apps) await cp(join(built, entry.package), join(stage, entry.package), { recursive: true, dereference: false });
    const files = await verifyContents(stage), application = await json(join(repository, 'package.json'));
    const packageVersions = {};
    for (const name of PACKAGE_NAMES) packageVersions[name] = (await json(join(stage, 'packages', name, 'package.json'))).version;
    const receipt = { schemaVersion: 1, applicationVersion: application.version, minimumNodeMajor: 24, nodeRuntimeBundled: false, runtimeDependency: 'Install Node.js 24+ or set OVIRAPTOR_NODE_EXECUTABLE to a compatible executable.', packageVersions, builtinApps: manifest.apps.map(app => app.id), files };
    await writeFile(join(stage, DISTRIBUTION_RECEIPT), JSON.stringify(receipt, null, 2) + '\n');
    await verifyDesktopDistribution(stage);
    // Check once more before publishing if another process edited build inputs.
    await checkBuild(repository, built);
    if (builtSnapshot !== JSON.stringify(await inventory(built))) fail('build artifacts changed during staging; retry after the build is stable');
    await rm(outputDirectory, { recursive: true, force: true }); await rename(stage, outputDirectory);
    return { outputDirectory, fileCount: files.length, applicationVersion: receipt.applicationVersion, nodeRuntimeBundled: false };
  } finally { await rm(stage, { recursive: true, force: true }); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = process.argv.includes('--verify')
      ? await verifyDesktopDistribution(join(repositoryRoot, 'src-tauri', DESKTOP_HOST_RESOURCE))
      : await prepareDesktopAgentHost();
    process.stdout.write(`${JSON.stringify({ staged: true, ...result, ...(result.files ? { files: undefined, fileCount: result.files.length } : {}) })}\n`);
  } catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
