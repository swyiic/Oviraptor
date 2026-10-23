import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { resolveInside, runWorkspaceTool } from '../src/workspace-tools.mjs';
import { compressSessionMessages } from '../src/memory.mjs';
import { humanizeToolTitle, QUALITY_SYSTEM_PROMPT } from '../src/quality-prompt.mjs';

test('workspace paths cannot escape the conversation directory', async () => {
  const root = await mkdtemp(join(tmpdir(), 'oviraptor-ws-'));
  try {
    assert.equal(resolveInside(root, 'notes/a.txt').startsWith(root), true);
    assert.throws(() => resolveInside(root, '../secret'), /workspace_path_denied/);
    const written = await runWorkspaceTool(root, 'workspace_write', { path: 'tool.js', content: 'console.log(1)\n' });
    assert.equal(written.bytes > 0, true);
    await assert.rejects(runWorkspaceTool(root, 'workspace_run', { command: 'node', args: ['-e', '1'] }), error => error.code === 'workspace_execution_invalid');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('compression keeps older user requests and tool facts', () => {
  const messages = [];
  for (let i = 0; i < 6; i++) {
    messages.push({ role: 'user', content: `要求${i}：把脚本写到工作区` });
    messages.push({ role: 'tool', name: 'workspace_write', content: `wrote step ${i}` });
  }
  const packed = compressSessionMessages(messages, { keepRecentUserTurns: 2 });
  const marker = packed.messages.find(item => String(item.content).includes('上下文已压缩'));
  assert.ok(marker);
  assert.match(marker.content, /要求0/);
  assert.match(marker.content, /workspace_write/);
  const again = compressSessionMessages(packed.messages, { keepRecentUserTurns: 2 });
  const markerAgain = again.messages.find(item => String(item.content).includes('上下文已压缩'));
  assert.match(markerAgain.content, /要求0/);
  assert.equal(humanizeToolTitle('workspace_write'), '正在写入工作区文件');
  assert.match(QUALITY_SYSTEM_PROMPT, /workspace_write/);
  assert.match(QUALITY_SYSTEM_PROMPT, /workspace_run/);
});
