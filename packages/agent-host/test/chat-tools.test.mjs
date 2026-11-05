import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { browserExecutable, runHostShell, sourceSearch } from '../src/chat-tools.mjs';
import { permissionDirective } from '../src/quality-prompt.mjs';
import { runWorkspaceTool } from '../src/workspace-tools.mjs';

test('shell runs in the given directory and returns stdout', async () => {
  const cwd = await mkdtemp(joinTmp());
  try {
    const result = await runHostShell('printf ok', { cwd });
    assert.equal(result.exitCode, 0);
    assert.equal(result.stdout, 'ok');
  } finally {
    await rm(cwd, { recursive: true, force: true });
  }
});

test('full access tells the agent not to refuse a judged instruction', () => {
  assert.match(permissionDirective('full'), /不要拒绝/);
  assert.match(permissionDirective('ask'), /确认/);
});

test('source search reads a named directory without copying it', async () => {
  const dir = await mkdtemp(joinTmp());
  try {
    await writeFile(join(dir, 'app.js'), 'prefix AES.decrypt(data,key) suffix');
    const found = await sourceSearch(dir, 'AES.decrypt');
    assert.equal(found.matches.length, 1);
    assert.match(found.matches[0].text, /AES\.decrypt/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test('sql comparison stays closed until access is allowed', async () => {
  const root = await mkdtemp(joinTmp());
  try {
    const code = await runWorkspaceTool(root, 'workspace_probe_sql', { url: 'https://example.test/search?q=1', param: 'q', values: ["'"] }, { materialOpen: false })
      .then(() => '', error => error.code);
    assert.equal(code, 'workspace_material_closed');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('browser executable lookup does not throw', () => {
  assert.equal(typeof browserExecutable(), 'string');
});

function joinTmp() {
  return `${tmpdir()}/chat-shell-`;
}
