// Ratchet: agent_runtime must not gain NEW direct crate::scanning or
// crate::commands::assets edges. Full shared-crate extraction is deferred;
// this only records today's counts and fails if they rise.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.join(__dirname, '..', 'src-tauri', 'src', 'agent_runtime');

/** Baseline recorded 2026-10-17 (Asia/Shanghai) on feature/agent-first-app-platform-20261016. */
const BASELINE = Object.freeze({
  'crate::scanning': 24,
  'crate::commands::assets': 0,
});

const PATTERNS = Object.freeze({
  'crate::scanning': /\bcrate::scanning\b/g,
  'crate::commands::assets': /\bcrate::commands::assets\b/g,
});

function rustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const filename = path.join(directory, entry.name);
    return entry.isDirectory() ? rustFiles(filename)
      : entry.isFile() && entry.name.endsWith('.rs') ? [filename] : [];
  });
}

function stripLineComment(line) {
  // Enough for this ratchet: ignore // comments. Block comments are rare on import lines.
  const index = line.indexOf('//');
  return index === -1 ? line : line.slice(0, index);
}

function countOccurrences(pattern) {
  let total = 0;
  const byFile = [];
  for (const filename of rustFiles(root)) {
    const text = fs.readFileSync(filename, 'utf8');
    let fileCount = 0;
    for (const line of text.split('\n')) {
      const code = stripLineComment(line);
      const matches = code.match(pattern);
      if (matches) fileCount += matches.length;
    }
    if (fileCount > 0) {
      total += fileCount;
      byFile.push({ file: path.relative(path.join(__dirname, '..'), filename).split(path.sep).join('/'), count: fileCount });
    }
  }
  byFile.sort((a, b) => b.count - a.count || a.file.localeCompare(b.file));
  return { total, byFile };
}

for (const [label, pattern] of Object.entries(PATTERNS)) {
  test(`agent_runtime ${label} import count must not rise above baseline ${BASELINE[label]}`, () => {
    const { total, byFile } = countOccurrences(pattern);
    assert.ok(
      total <= BASELINE[label],
      `${label} occurrences rose from baseline ${BASELINE[label]} to ${total}. Offenders:\n` +
        byFile.map(row => `  ${row.count}\t${row.file}`).join('\n'),
    );
    if (total < BASELINE[label]) {
      console.log(`[ratchet] ${label} improved ${BASELINE[label]} -> ${total}; consider lowering BASELINE`);
    }
  });
}
