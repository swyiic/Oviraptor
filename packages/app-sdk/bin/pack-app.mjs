#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createDirectoryPackage } from '../src/packager.mjs';
import { keys } from '../src/protocol.mjs';
const [sourceDirectory, outputDirectory, ...extra] = process.argv.slice(2);
if (!sourceDirectory || !outputDirectory || extra.length) {
  process.stderr.write('Usage: node pack-app.mjs SOURCE_DIRECTORY OUTPUT_DIRECTORY\n');
  process.exitCode = 2;
} else {
  try {
    const spec = JSON.parse(await readFile(join(resolve(sourceDirectory), 'app.package.json'), 'utf8'));
    keys(spec, ['manifest', 'files', 'vendorSdk'], ['manifest', 'files'], 'package specification');
    const result = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
    process.stdout.write(`${JSON.stringify({ directory: result.directory, appId: result.manifest.id, version: result.manifest.version, manifestSha256: result.manifestSha256, totalBytes: result.totalBytes })}\n`);
  } catch (error) {
    process.stderr.write(`${error.code?.startsWith('app_') ? error.code : 'app_package_invalid'}\n`);
    process.exitCode = 1;
  }
}
