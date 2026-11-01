#!/usr/bin/env node
// Blocks new untranslated Chinese in the frontend. User-facing text belongs in
// src/locales/<lang>.ts (or, while migrating, in an inline tr('中文', 'English')
// pair). Existing untranslated strings are recorded per file in
// tools/i18n-baseline.json; a file may only go down from its recorded count.
//
//   node tools/check_i18n.mjs            check (part of npm run build)
//   node tools/check_i18n.mjs --update   re-record the baseline after migrating text
//   node tools/check_i18n.mjs --report src/features/sentinel/   remaining strings per file
//
// A line that must keep Chinese for a non-display reason (for example matching
// a backend message) can carry an `i18n-ignore` comment.
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const src = join(root, "src");
const baselinePath = join(root, "tools", "i18n-baseline.json");
const CJK = /[一-鿿]/;
const TR_PAIR = /\btr\(\s*(['"`])(?:\\.|(?!\1)[\s\S])*?\1\s*,\s*(['"`])(?:\\.|(?!\2)[\s\S])*?\2\s*,?\s*\)/g;
// A ["中文", "English"] tuple is a translated label resolved later with tr(...).
const PAIR_TUPLE = /\[\s*(['"`])(?:\\.|(?!\1)[^\n])*?\1\s*,\s*(['"`])((?:\\.|(?!\2)[^\n])*?)\2\s*\]/g;

function sourceFiles(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "locales" || name === "__tests__" ? [] : sourceFiles(path);
    return /\.(vue|ts)$/.test(name) && !name.endsWith(".d.ts") ? [path] : [];
  });
}

function untranslated(path) {
  let text = readFileSync(path, "utf8")
    .split("\n").filter((line) => !line.includes("i18n-ignore")).join("\n")
    .replace(TR_PAIR, "tr()")
    .replace(PAIR_TUPLE, (match, _a, _b, english) => (CJK.test(english) ? match : "[pair]"))
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[\s;{}(),])\/\/[^\n]*/g, "$1");
  let template = "";
  let script = text;
  if (path.endsWith(".vue")) {
    template = text.match(/<template>([\s\S]*)<\/template>/)?.[1] ?? "";
    script = [...text.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].map((match) => match[1]).join("\n");
  }
  const templateText = template.match(/[一-鿿][^<>{}"'`\n]*/g) ?? [];
  const scriptText = (script.match(/(['"`])(?:\\.|(?!\1)[^\n])*?\1/g) ?? []).filter((literal) => CJK.test(literal));
  return templateText.length + scriptText.length;
}

const counts = Object.fromEntries(sourceFiles(src)
  .map((path) => [relative(root, path), untranslated(path)])
  .filter(([, count]) => count > 0)
  .sort(([a], [b]) => a.localeCompare(b)));
const total = Object.values(counts).reduce((sum, count) => sum + count, 0);

if (process.argv.includes("--report")) {
  const prefix = process.argv[process.argv.indexOf("--report") + 1] || "src/";
  const rows = Object.entries(counts).filter(([file]) => file.startsWith(prefix)).sort(([, a], [, b]) => b - a);
  for (const [file, count] of rows) console.log(`${String(count).padStart(5)}  ${file}`);
  console.log(`${rows.reduce((sum, [, count]) => sum + count, 0)} untranslated strings under ${prefix}`);
  process.exit(0);
}
if (process.argv.includes("--update")) {
  writeFileSync(baselinePath, `${JSON.stringify(counts, null, 2)}\n`);
  console.log(`i18n baseline: ${total} untranslated strings in ${Object.keys(counts).length} files.`);
  process.exit(0);
}

let baseline = {};
try { baseline = JSON.parse(readFileSync(baselinePath, "utf8")); } catch { /* first run: everything is new */ }
const grown = Object.entries(counts).filter(([file, count]) => count > (baseline[file] ?? 0));
if (grown.length) {
  console.error("Untranslated Chinese was added. Put the text in src/locales/zh.ts and en.ts (t.…),");
  console.error("or at least wrap it as tr('中文', 'English'):");
  for (const [file, count] of grown) console.error(`  ${file}: ${baseline[file] ?? 0} → ${count}`);
  process.exit(1);
}
const before = Object.values(baseline).reduce((sum, count) => sum + count, 0);
console.log(`i18n check: ${total} untranslated strings left${total < before ? ` (down from ${before}; run npm run i18n:baseline to record)` : ""}.`);
