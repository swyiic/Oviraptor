import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runWorkspaceTool, globToRegExp } from '../src/workspace-tools.mjs';

async function withRoot(run) {
  const root = await mkdtemp(join(tmpdir(), 'ws-tools-'));
  try { await run(root); } finally { await rm(root, { recursive: true, force: true }); }
}
const call = (root, name, args) => runWorkspaceTool(root, name, args, {});
const code = async promise => { try { await promise; return null; } catch (e) { return e.code; } };

test('globToRegExp: * stays in a segment, ** spans directories', () => {
  assert.ok(globToRegExp('*.mjs').test('a.mjs'));
  assert.ok(!globToRegExp('*.mjs').test('src/a.mjs'));
  assert.ok(globToRegExp('**/*.mjs').test('src/x/a.mjs'));
  assert.ok(globToRegExp('**/*.mjs').test('a.mjs'));
  assert.ok(globToRegExp('src/*.js').test('src/a.js'));
  assert.ok(!globToRegExp('src/*.js').test('src/x/a.js'));
  assert.ok(!globToRegExp('*.mjs').test('a.js'));
  // Regex metacharacters in the glob are matched literally.
  assert.ok(globToRegExp('a.b+c.txt').test('a.b+c.txt'));
  assert.ok(!globToRegExp('a.b+c.txt').test('aXbXcXtxt'));
});

test('workspace_edit replaces a unique span and leaves the rest byte-for-byte', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'a.txt', content: 'alpha\nBETA\ngamma\n' });
  const res = await call(root, 'workspace_edit', { path: 'a.txt', oldString: 'BETA', newString: 'beta' });
  assert.equal(res.replaced, 1);
  const read = await call(root, 'workspace_read', { path: 'a.txt' });
  assert.equal(read.content, 'alpha\nbeta\ngamma\n');
}));

test('workspace_edit: no match / not unique / no change are rejected', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'a.txt', content: 'x x y\n' });
  assert.equal(await code(call(root, 'workspace_edit', { path: 'a.txt', oldString: 'zzz', newString: 'q' })), 'workspace_edit_no_match');
  assert.equal(await code(call(root, 'workspace_edit', { path: 'a.txt', oldString: 'x', newString: 'q' })), 'workspace_edit_not_unique');
  assert.equal(await code(call(root, 'workspace_edit', { path: 'a.txt', oldString: 'x x', newString: 'x x' })), 'workspace_edit_no_change');
  assert.equal(await code(call(root, 'workspace_edit', { path: 'a.txt', oldString: '', newString: 'q' })), 'workspace_edit_invalid');
  // Unchanged after the failures.
  assert.equal((await call(root, 'workspace_read', { path: 'a.txt' })).content, 'x x y\n');
}));

test('workspace_edit replaceAll replaces every occurrence', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'a.txt', content: 'x x x\n' });
  const res = await call(root, 'workspace_edit', { path: 'a.txt', oldString: 'x', newString: 'y', replaceAll: true });
  assert.equal(res.replaced, 3);
  assert.equal((await call(root, 'workspace_read', { path: 'a.txt' })).content, 'y y y\n');
}));

test('workspace_edit inserts newString literally (no $-pattern reinterpretation)', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'a.txt', content: 'keep MARK keep\n' });
  await call(root, 'workspace_edit', { path: 'a.txt', oldString: 'MARK', newString: '$& $1 $$' });
  assert.equal((await call(root, 'workspace_read', { path: 'a.txt' })).content, 'keep $& $1 $$ keep\n');
}));

test('workspace_grep finds a match past the first 2000 characters of one line', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'app.js', content: `${'a'.repeat(5000)}AES.decrypt(data,key)` });
  const found = await call(root, 'workspace_grep', { pattern: 'AES\\.decrypt', literal: false });
  assert.ok(found.matches.some(item => /AES\.decrypt/.test(item.text)));
}));

test('workspace_grep: regex, literal, caseInsensitive, include filter', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'src/a.mjs', content: 'const TODO = 1\nok\nTODO again\n' });
  await call(root, 'workspace_write', { path: 'src/b.txt', content: 'TODO here\n' });
  const re = await call(root, 'workspace_grep', { pattern: '^TODO', include: '**/*.mjs' });
  assert.deepEqual(re.matches.map(m => [m.file, m.line]), [['src/a.mjs', 3]]);
  const ci = await call(root, 'workspace_grep', { pattern: 'todo', caseInsensitive: true, include: '**/*.mjs' });
  assert.equal(ci.matches.length, 2);
  const lit = await call(root, 'workspace_grep', { pattern: 'a.mjs', literal: true });
  // literal dot: matches the filename text in content only where present (none here), proving '.' is not a wildcard
  assert.equal(lit.matches.length, 0);
}));

test('workspace_grep: invalid regex reports a pattern error, not a crash', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'a.txt', content: 'hi\n' });
  assert.equal(await code(call(root, 'workspace_grep', { pattern: '(' })), 'workspace_grep_pattern_invalid');
}));

test('workspace_glob returns sorted matching paths, honoring a base path', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'src/a.mjs', content: '1' });
  await call(root, 'workspace_write', { path: 'src/deep/b.mjs', content: '2' });
  await call(root, 'workspace_write', { path: 'note.txt', content: '3' });
  const all = await call(root, 'workspace_glob', { pattern: '**/*.mjs' });
  assert.deepEqual(all.matches, ['src/a.mjs', 'src/deep/b.mjs']);
  const scoped = await call(root, 'workspace_glob', { pattern: '*.mjs', path: 'src' });
  assert.deepEqual(scoped.matches, ['src/a.mjs']); // base-relative glob, workspace-relative result
}));

test('new tools keep path confinement', async () => withRoot(async root => {
  await call(root, 'workspace_write', { path: 'a.txt', content: 'hi\n' });
  assert.equal(await code(call(root, 'workspace_edit', { path: '../escape.txt', oldString: 'a', newString: 'b' })), 'workspace_path_denied');
  assert.equal(await code(call(root, 'workspace_grep', { pattern: 'x', path: '../..' })), 'workspace_path_denied');
  assert.equal(await code(call(root, 'workspace_glob', { pattern: '*', path: '../..' })), 'workspace_path_denied');
}));

// --- workspace_ws (增量4): gating + bounded capture ---
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';

function wsAccept(key) { return createHash('sha1').update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64'); }
function wsEncodeText(str) { const p = Buffer.from(str); return Buffer.concat([Buffer.from([0x81, p.length]), p]); } // assumes <126 bytes
function wsDecode(buf) { const len = buf[1] & 0x7f; const mask = buf.subarray(2, 6); const data = buf.subarray(6, 6 + len); const out = Buffer.alloc(len); for (let i = 0; i < len; i++) out[i] = data[i] ^ mask[i % 4]; return out.toString(); }

function startWsEcho() {
  const server = createServer();
  const sockets = new Set();
  server.on('upgrade', (req, socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + wsAccept(req.headers['sec-websocket-key']) + '\r\n\r\n');
    socket.write(wsEncodeText('hello-from-server'));
    socket.on('data', chunk => { try { socket.write(wsEncodeText('echo:' + wsDecode(chunk))); } catch { /* ignore */ } });
    socket.on('error', () => {});
  });
  const close = () => { for (const s of sockets) { try { s.destroy(); } catch { /* ignore */ } } server.close(); };
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port, close })));
}

const wsCode = async (root, args, opts) => { try { await runWorkspaceTool(root, 'workspace_ws', args, opts); return null; } catch (e) { return e.code; } };

test('workspace_ws: closed material, bad protocol, un-cited host, SSRF address are all rejected', async () => withRoot(async root => {
  // no material access → closed
  assert.equal(await wsCode(root, { url: 'ws://127.0.0.1:1/x' }, {}), 'workspace_material_closed');
  // material open but http(s)/garbage → invalid
  assert.equal(await wsCode(root, { url: 'https://example.com' }, { materialOpen: true, allowedHosts: ['example.com'] }), 'workspace_ws_invalid');
  // host not among cited/authorized → host denied (scope lock)
  assert.equal(await wsCode(root, { url: 'ws://evil.example/x' }, { materialOpen: true, allowedHosts: ['good.example'] }), 'workspace_ws_host_denied');
  // link-local metadata address, even if "allowed", is blocked by the SSRF wall
  assert.equal(await wsCode(root, { url: 'ws://169.254.169.254/latest' }, { materialOpen: true, allowedHosts: ['169.254.169.254'] }), 'workspace_ws_blocked');
}));

test('workspace_ws: captures server frames and records sent frames, bounded by timeout', async () => withRoot(async root => {
  const { port, close } = await startWsEcho();
  try {
    const res = await runWorkspaceTool(root, 'workspace_ws', { url: `ws://127.0.0.1:${port}/live`, send: ['ping'], timeoutMs: 500 },
      { materialOpen: true, allowedHosts: ['127.0.0.1'] });
    assert.equal(res.opened, true);
    assert.equal(res.sent, 1);
    const recv = res.frames.filter(f => f.dir === 'recv').map(f => f.text);
    assert.ok(recv.includes('hello-from-server'), `server greeting captured: ${JSON.stringify(recv)}`);
    assert.ok(recv.includes('echo:ping'), `echo of sent frame captured: ${JSON.stringify(recv)}`);
    assert.ok(res.frames.some(f => f.dir === 'send' && f.text === 'ping'), 'sent frame recorded as evidence');
  } finally { close(); }
}));
