import { constants } from 'node:fs';
import { lstat, open, readdir, realpath } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
import { AppSdkError, fail } from './protocol.mjs';
import { MANIFEST_LIMITS, NATIVE_MANIFEST_LIMITS, manifestLimits, validateManifest, validatePackagePath } from './manifest.mjs';

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export async function readRegularFile(path, maximum = MANIFEST_LIMITS.fileBytes) {
  const before = await lstat(path);
  if (!before.isFile() || before.isSymbolicLink() || before.size > maximum) fail('app_package_invalid', 'Package contains a nonregular or oversized file');
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const opened = await handle.stat();
    if (opened.dev !== before.dev || opened.ino !== before.ino || opened.size !== before.size) fail('app_package_changed', 'Package changed during validation');
    // Never let readFile allocate an unbounded buffer if a mutable source grows.
    const buffer = Buffer.alloc(opened.size + 1);
    let length = 0;
    while (length < buffer.length) {
      const read = await handle.read(buffer, length, buffer.length - length, null);
      if (read.bytesRead === 0) break;
      length += read.bytesRead;
    }
    const bytes = buffer.subarray(0, length);
    const after = await handle.stat();
    if (bytes.length !== opened.size || bytes.length > maximum || after.size !== opened.size || after.mtimeMs !== opened.mtimeMs || after.ctimeMs !== opened.ctimeMs) fail('app_package_changed', 'Package changed during validation');
    return bytes;
  } finally { await handle.close(); }
}
export function validateNativeExecutable(bytes, runtime) {
  let matches = false;
  if (runtime.platform === 'linux' && bytes.length >= 64 && bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])) && bytes[4] === 2 && bytes[5] === 1) {
    matches = bytes.readUInt16LE(18) === (runtime.arch === 'x64' ? 62 : 183);
  } else if (runtime.platform === 'darwin' && bytes.length >= 32 && bytes.readUInt32LE(0) === 0xfeedfacf) {
    matches = bytes.readUInt32LE(4) === (runtime.arch === 'x64' ? 0x01000007 : 0x0100000c);
  } else if (runtime.platform === 'win32' && bytes.length >= 64 && bytes[0] === 0x4d && bytes[1] === 0x5a) {
    const offset = bytes.readUInt32LE(0x3c);
    matches = offset <= bytes.length - 24 && bytes.readUInt32LE(offset) === 0x00004550 && bytes.readUInt16LE(offset + 4) === (runtime.arch === 'x64' ? 0x8664 : 0xaa64);
  }
  if (!matches) fail('app_package_executable_invalid', 'Native executable does not match its declared platform and architecture');
}
export async function listPackageFiles(directory, limits = MANIFEST_LIMITS) {
  const found = [];
  let entriesVisited = 0;
  async function visit(relative) {
    const entries = await readdir(join(directory, relative), { withFileTypes: true });
    for (const entry of entries) {
      if (++entriesVisited > limits.files * 4) fail('app_package_invalid', 'Package directory tree exceeds its entry budget');
      const path = relative ? `${relative}/${entry.name}` : entry.name;
      validatePackagePath(path);
      const info = await lstat(join(directory, path));
      if (info.isSymbolicLink()) fail('app_package_invalid', 'Package symlinks are forbidden');
      if (info.isDirectory()) await visit(path);
      else if (info.isFile()) found.push(path);
      else fail('app_package_invalid', 'Package contains a nonregular file');
      if (found.length > limits.files + 1) fail('app_package_invalid', 'Package has too many files');
    }
  }
  await visit('');
  return found.sort();
}
export async function validatePackageDirectory(directory) {
  try {
    const absolute = resolve(directory);
    const info = await lstat(absolute);
    if (!info.isDirectory() || info.isSymbolicLink()) fail('app_package_invalid', 'Package must be a real directory');
    const canonical = await realpath(absolute);
    const manifestBytes = await readRegularFile(join(canonical, 'manifest.json'), NATIVE_MANIFEST_LIMITS.manifestBytes);
    let parsed;
    try { parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(manifestBytes)); }
    catch { fail('app_manifest_invalid', 'Manifest must be UTF-8 JSON'); }
    const manifest = validateManifest(parsed);
    const limits = manifestLimits(manifest);
    const actual = await listPackageFiles(canonical, limits);
    const expected = ['manifest.json', ...manifest.files.map(item => item.path)].sort();
    if (JSON.stringify(actual) !== JSON.stringify(expected)) fail('app_package_invalid', 'Package contains missing or unlisted files');
    let totalBytes = manifestBytes.length;
    const files = [];
    for (const declared of manifest.files) {
      const bytes = await readRegularFile(join(canonical, declared.path), limits.fileBytes);
      totalBytes += bytes.length;
      if (totalBytes > limits.packageBytes) fail('app_package_invalid', 'Package exceeds its byte budget');
      const digest = sha256(bytes);
      if (digest !== declared.sha256) fail('app_package_hash_mismatch', 'Package file hash does not match manifest');
      if (manifest.schemaVersion === 2 && declared.path === manifest.entry) validateNativeExecutable(bytes, manifest.runtime);
      files.push({ path: declared.path, sha256: digest, size: bytes.length });
    }
    return { manifest, directory: canonical, files, totalBytes, manifestSha256: sha256(manifestBytes) };
  } catch (error) {
    if (error instanceof AppSdkError) throw error;
    fail('app_package_invalid', 'Package cannot be read safely');
  }
}
