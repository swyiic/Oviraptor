import { readdir, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createDirectoryPackage } from '../packages/app-sdk/src/index.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
for (const group of ['examples', 'apps']) {
  for (const entry of await readdir(join(root, group), { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const sourceDirectory = join(root, group, entry.name);
    let spec;
    try { spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8')); }
    catch (error) { if (error.code === 'ENOENT') continue; throw error; }
    const outputDirectory = join(root, '.agent-build', 'apps', `${spec.manifest.id}-${spec.manifest.version}`);
    await rm(outputDirectory, { recursive: true, force: true });
    const result = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
    process.stdout.write(JSON.stringify({ id: result.manifest.id, version: result.manifest.version, path: result.directory, sha256: result.manifestSha256 }) + '\n');
  }
}
