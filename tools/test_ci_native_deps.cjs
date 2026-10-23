const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

function runInstaller(failPhase) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ovi-ci-apt-test-'));
  try {
    const log = path.join(dir, 'calls.jsonl');
    const writeExecutable = (name, body) => fs.writeFileSync(path.join(dir, name), body, { mode: 0o700 });
    writeExecutable('timeout', '#!/bin/sh\nprintf "%s\\n" "$*" >> "$TEST_TIMEOUT_LOG"\nshift 3\nexec "$@"\n');
    writeExecutable('sudo', '#!/bin/sh\nshift 2\nexec "$@"\n');
    writeExecutable('apt-get', `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
const source = args.find(v => v.startsWith('Dir::Etc::sourcelist=')).split('=').slice(1).join('=');
fs.appendFileSync(process.env.TEST_APT_LOG, JSON.stringify({args, source, content:fs.readFileSync(source,'utf8'), frontend:process.env.DEBIAN_FRONTEND})+'\\n');
process.exit(args.includes(process.env.TEST_FAIL_PHASE) ? 42 : 0);
`);
    const result = spawnSync('bash', [path.join(__dirname, 'install_ci_native_deps.sh')], {
      encoding: 'utf8', timeout: 10000,
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, TEST_APT_LOG: log,
        TEST_TIMEOUT_LOG: path.join(dir, 'timeouts'), TEST_FAIL_PHASE: failPhase ?? '' },
    });
    const calls = fs.existsSync(log) ? fs.readFileSync(log, 'utf8').trim().split('\n').map(JSON.parse) : [];
    return { result, calls, timeouts: fs.readFileSync(path.join(dir, 'timeouts'), 'utf8') };
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
}

test('CI installer isolates signed Ubuntu sources, bounds network/operations and removes temporary source', () => {
  const { result, calls, timeouts } = runInstaller();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(calls.length, 2);
  for (const call of calls) {
    assert.match(call.content, /https:\/\/archive\.ubuntu\.com\/ubuntu noble main universe/);
    assert.match(call.content, /https:\/\/security\.ubuntu\.com\/ubuntu noble-security/);
    assert.equal(call.content.split('\n').filter(Boolean).length, 3);
    assert.equal((call.content.match(/signed-by=/g) ?? []).length, 3);
    for (const option of ['Dir::Etc::sourceparts=-', 'Acquire::Retries=2', 'Acquire::http::Timeout=30',
      'Acquire::https::Timeout=30', 'APT::Update::Error-Mode=any', 'Acquire::ForceIPv4=true']) assert.ok(call.args.includes(option));
    assert.equal(call.frontend, 'noninteractive');
    assert.equal(fs.existsSync(call.source), false);
  }
  assert.ok(calls[0].args.includes('update'));
  assert.ok(calls[1].args.includes('--no-install-recommends'));
  assert.ok(calls[1].args.includes('libwebkit2gtk-4.1-dev'));
  assert.match(timeouts, /--kill-after=15s 120s/);
  assert.match(timeouts, /--kill-after=15s 360s/);
});
for (const phase of ['update', 'install']) test(`CI installer propagates ${phase} failure without concealing it`, () => {
  const { result, calls } = runInstaller(phase);
  assert.equal(result.status, 42, result.stderr);
  assert.equal(calls.length, phase === 'update' ? 1 : 2);
  for (const call of calls) assert.equal(fs.existsSync(call.source), false);
});
