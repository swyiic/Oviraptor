import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, symlink, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { citedPathTokens, importCitedFiles, readFileRange } from '../src/cited-files.mjs';

async function withDirs(run) {
  const root = await mkdtemp(join(tmpdir(), 'cited-ws-'));
  const outside = await mkdtemp(join(tmpdir(), 'cited-src-'));
  try { await run(root, outside); }
  finally {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
}

test('a path glued to Chinese is still a cited token', () => {
  const text = '你能分析这个文件吗/Users/swyiic/Documents/MobileE/鼓楼水务_1.14.37.mec';
  assert.deepEqual(citedPathTokens(text), ['/Users/swyiic/Documents/MobileE/鼓楼水务_1.14.37.mec']);
});

test('a file over 1MB stays at its original path', async () => withDirs(async (root, outside) => {
  const file = join(outside, 'big.bin');
  await writeFile(file, Buffer.alloc(1024 * 1024 + 8, 0xff));
  const result = await importCitedFiles(file, root);
  assert.equal(result.imported.length, 0);
  assert.equal(result.external[0].source, file);
  assert.equal(result.external[0].bytes, 1024 * 1024 + 8);
  const slice = await readFileRange(file, 1024 * 1024, 4);
  assert.equal(slice.encoding, 'hex');
  assert.equal(slice.content, 'ffffffff');
  assert.equal(slice.size, 1024 * 1024 + 8);
}));

test('import copies only the file named in the message', async () => withDirs(async (root, outside) => {
  const file = join(outside, 'sample.mec');
  await writeFile(file, Buffer.from('MEC\0header-and-body'));
  const linked = join(outside, 'linked.mec');
  await symlink(file, linked);
  const text = `你能分析这个文件吗${file} 另外看看 ${linked}`;
  const result = await importCitedFiles(text, root);
  assert.equal(result.imported.length, 1);
  assert.equal(result.imported[0].path, 'cited/sample.mec');
  assert.equal(result.imported[0].bytes, 19);
  assert.equal(await readFile(join(root, 'cited/sample.mec'), 'utf8'), 'MEC\0header-and-body');
  assert.equal(result.imported.some(item => item.source === linked), false);
  assert.match(result.note, /已复制 cited\/sample\.mec/);
}));
