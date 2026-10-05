// Local release evidence, not a boundary against concurrent hostile writers.
import { constants, createReadStream } from 'node:fs';
import { lstat, open, readdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';

export const digest = value => createHash('sha256').update(value).digest('hex');

export async function fileEntry(file, relative) {
  const handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile()) throw new Error(`inventory_not_regular_file:${relative}`);
    const hash = createHash('sha256');
    for await (const chunk of createReadStream(file, { fd: handle.fd, autoClose: false })) hash.update(chunk);
    const after = await handle.stat({ bigint: true });
    if (before.size !== after.size || before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs)
      throw new Error(`inventory_file_changed_during_read:${relative}`);
    return { path: relative, sha256: hash.digest('hex'), size: String(after.size),
      executionMode: Number(after.mode & 0o111n) };
  } finally {
    await handle.close();
  }
}

export async function inventoryFiles(root, roots = [''], prefix = 'inventory') {
  const entries = [];
  let visited = 0;
  async function visit(relative, depth = 0) {
    if (++visited > 100000 || depth > 64) throw new Error(`${prefix}_entry_limit`);
    const file = path.join(root, relative);
    const info = await lstat(file);
    if (info.isSymbolicLink()) throw new Error(`${prefix}_entry_symlink:${relative}`);
    if (info.isDirectory()) {
      for (const name of (await readdir(file)).sort()) {
        await visit(relative ? `${relative}/${name}` : name, depth + 1);
      }
    } else if (info.isFile()) entries.push(await fileEntry(file, relative));
    else throw new Error(`${prefix}_entry_not_regular_file:${relative}`);
  }
  for (const relative of roots) await visit(relative);
  entries.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  return entries;
}
