import { chmod, copyFile, lstat, mkdir, mkdtemp, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createDirectoryPackage } from '../../packages/app-sdk/src/index.mjs';
import { createNativeManifest } from './native-manifest.mjs';
import { readNativePageArtifact } from './build-native-page.mjs';

const source = dirname(fileURLToPath(import.meta.url));
const repository = resolve(source, '../..');
const resources = [
  'workers/9_frontend_runtime_probe.cjs', 'workers/8_js_ast_analyzer.cjs', 'workers/babel-parser.cjs',
  'workers/BABEL_PARSER_LICENSE.txt', 'config/browser-locations.json',
];

/** Package only; never execute an input binary or contact a target/provider. */
export async function packageNativeScanning({ binary, outputDirectory, platform = process.platform, arch = process.arch }) {
  if (!isAbsolute(binary) || !isAbsolute(outputDirectory)) throw Error('native_package_paths_must_be_absolute');
  const metadata = await lstat(binary);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size < 1 || metadata.size > 128 * 1024 * 1024) throw Error('native_package_binary_invalid');
  const staging = await realpath(await mkdtemp(join(tmpdir(), 'oviraptor-scanning-native-package-')));
  const manifest = createNativeManifest({ platform, arch });
  try {
    await mkdir(join(staging, 'bin'), { mode: 0o700 });
    await copyFile(binary, join(staging, manifest.entry));
    await chmod(join(staging, manifest.entry), 0o500);
    const files = [manifest.entry, 'native.html'];
    // This author CLI copies a prebuilt artifact. Package installation never
    // runs the builder or any application hook, and the opaque page makes no
    // external module, CSS, fetch or token request.
    await writeFile(join(staging, 'native.html'), await readNativePageArtifact());
    for (const name of resources) {
      const target = `resources/${name}`;
      await mkdir(dirname(join(staging, target)), { recursive: true, mode: 0o700 });
      await copyFile(join(repository, 'src-tauri/resources', name), join(staging, target));
      files.push(target);
    }
    return await createDirectoryPackage({ sourceDirectory: staging, outputDirectory, manifest, files, vendorSdk: false });
  } finally { await rm(staging, { recursive: true, force: true }); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2), options = {};
  if (args.length % 2) throw Error('usage: --binary ABS --output ABS [--platform linux|darwin|win32] [--arch x64|arm64]');
  const names = new Map([['--binary', 'binary'], ['--output', 'outputDirectory'], ['--platform', 'platform'], ['--arch', 'arch']]);
  for (let i = 0; i < args.length; i += 2) {
    const name = names.get(args[i]);
    if (!name || Object.hasOwn(options, name)) throw Error('native_package_arguments_invalid');
    options[name] = args[i + 1];
  }
  const result = await packageNativeScanning(options);
  console.log(JSON.stringify({ outputDirectory: options.outputDirectory, appId: result.manifest?.id ?? 'org.oviraptor.scanning' }));
}
