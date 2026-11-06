import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import {
  BUILTIN_SKELETONS, readApprovedAgentTools, allAgentTools,
  matchAgentTools, formatAgentToolsPromptBlock, recallAgentToolCode,
} from '../src/agent-tools.mjs';

test('built-in skeletons are syntactically valid JS (String.raw preserved escapes)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'skel-'));
  try {
    for (const s of BUILTIN_SKELETONS) {
      const f = join(dir, s.id.replace(/[^\w]/g, '_') + '.js');
      writeFileSync(f, s.code);
      execFileSync(process.execPath, ['--check', f]); // throws on syntax error
      assert.match(s.code, /\\r\\n|process\.argv|require\(/); // real runnable shape
    }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('matchAgentTools ranks by tag/fingerprint tokens in the hint', () => {
  const hint = '帮我解密这些 aes-cbc 响应,key 已经找到';
  const m = matchAgentTools(BUILTIN_SKELETONS, hint);
  assert.ok(m.some(t => t.id === 'builtin:sym_decrypt'));
  assert.equal(matchAgentTools(BUILTIN_SKELETONS, '随便聊聊天气').length, 0);
  assert.ok(matchAgentTools(BUILTIN_SKELETONS, '这个 burp 文件好大').some(t => t.id === 'builtin:burp_carve'));
});

test('format block: matches list ids + non-authority wall; empty+total shows a hint', () => {
  const block = formatAgentToolsPromptBlock(matchAgentTools(BUILTIN_SKELETONS, 'aes cbc 解密'), { total: 3 });
  assert.match(block, /builtin:sym_decrypt/);
  assert.match(block, /不是授权/);
  assert.match(block, /验证/);
  assert.equal(formatAgentToolsPromptBlock([], { total: 0 }), '');
  assert.match(formatAgentToolsPromptBlock([], { total: 5 }), /5 个/);
});

test('recall returns built-in code; unknown id is null', () => {
  assert.ok(recallAgentToolCode(undefined, 'builtin:sym_decrypt').includes('createDecipheriv'));
  assert.equal(recallAgentToolCode(undefined, 'nope'), null);
});

test('readApprovedAgentTools reads only approved; allAgentTools merges builtins', () => {
  const dir = mkdtempSync(join(tmpdir(), 'tools-'));
  const path = join(dir, 'oviraptor.sqlite3');
  const db = new DatabaseSync(path);
  db.exec(`CREATE TABLE agent_tool_library(id TEXT PRIMARY KEY,title TEXT,language TEXT,code TEXT,target_fingerprint TEXT,algo_tags TEXT,status TEXT,created_at TEXT);`);
  const ins = db.prepare('INSERT INTO agent_tool_library VALUES(?,?,?,?,?,?,?,?)');
  ins.run('a', 'approved sm4', 'node', 'console.log(1)', '国密SM4', '["sm4","cbc"]', 'approved', '2026-11-06');
  ins.run('b', 'pending one', 'node', 'console.log(2)', '', '[]', 'pending', '2026-11-06');
  db.close();
  try {
    const got = readApprovedAgentTools(path);
    assert.equal(got.length, 1);
    assert.equal(got[0].id, 'a');
    assert.deepEqual(got[0].algoTags, ['sm4', 'cbc']);
    assert.equal(allAgentTools(path).length, BUILTIN_SKELETONS.length + 1);
    // the approved DB skeleton's code is recallable
    assert.equal(recallAgentToolCode(path, 'a'), 'console.log(1)');
    assert.equal(recallAgentToolCode(path, 'b'), null); // pending not recallable
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
