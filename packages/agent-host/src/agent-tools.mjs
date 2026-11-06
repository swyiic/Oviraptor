// Agent tool library — advisory recall of PARAMETERIZED SKELETONS (design §3 D3,
// refined per operator: store target-agnostic building blocks, NOT whole one-off
// scripts, and not a frozen built-in set). On a new target the agent recalls a
// matching skeleton, fills in that target's params (key/iv/offsets/algorithm),
// RE-VERIFIES it in the sandbox, then reuses it. Same wall as memory/technique
// cards: a skeleton suggestion is advisory only — it never grants scope/auth and
// never feeds any gate. Sources: a small built-in seed of universal skeletons +
// operator-approved skeletons from the Tauri app DB (agent_tool_library,
// read READ-ONLY here via OVIRAPTOR_APP_DB).
import { DatabaseSync } from 'node:sqlite';
import { existsSync, lstatSync } from 'node:fs';

const MAX_SURFACED = 6;

function clip(s, n) { s = String(s ?? ''); return s.length > n ? s.slice(0, n) + '…' : s; }

// Built-in universal skeletons. Target-agnostic; take target specifics as argv
// params; stdlib only (no deps, no net) so they run in the workspace sandbox.
export const BUILTIN_SKELETONS = [
  {
    id: 'builtin:burp_carve',
    title: 'Burp/HAR 流式 carver(抠出请求/响应里的静态资源)',
    language: 'node',
    algoTags: ['burp', 'har', 'carver', '流量', 'traffic', 'pcap'],
    targetFingerprint: '多 GB 流量文件抠取前端 JS/资源',
    params: 'argv: <burpPath> <outDir> [urlRegex]',
    code: String.raw`// burp_carve.js <burpPath> <outDir> [urlRegex] —— 流式(32MB/块)抠匹配 URL 的 200 响应体
const fs=require('fs'),zlib=require('zlib'),path=require('path');
const [P,OUT,RE]=process.argv.slice(2);
if(!P||!OUT){console.error('usage: node burp_carve.js <burpPath> <outDir> [urlRegex]');process.exit(2);}
fs.mkdirSync(OUT,{recursive:true});
const re=new RegExp('(GET|POST) ([^\\r\\n\\x00]{1,3000}) HTTP/1\\.1\\r\\nHost: ([^\\r\\n\\x00]{1,200})','g');
const ext=RE?new RegExp(RE,'i'):/\.(js|css|html|json)$/i;
const fd=fs.openSync(P,'r'),size=fs.fstatSync(fd).size,CH=32<<20,buf=Buffer.alloc(CH);
let off=0,tail=Buffer.alloc(0);const want=new Map();
while(off<size){const n=fs.readSync(fd,buf,0,CH,off);if(n<=0)break;const reg=tail.length?Buffer.concat([tail,buf.slice(0,n)]):buf.slice(0,n);const s=reg.toString('latin1');let m;re.lastIndex=0;while((m=re.exec(s))){const u=m[2].split('?')[0];if(!ext.test(u))continue;const abs=off-tail.length+m.index;if(!want.has(u))want.set(u,abs);}tail=reg.slice(-8192);off+=n;}
fs.closeSync(fd);console.error('candidates',want.size);
const f=fs.openSync(P,'r');let ok=0;
for(const [u,o] of [...want].sort()){const raw=Buffer.alloc(1<<21),n=fs.readSync(f,raw,0,raw.length,o),sl=raw.slice(0,n),i=sl.indexOf('HTTP/1.1 200');if(i<0)continue;const he=sl.indexOf('\r\n\r\n',i);if(he<0)continue;const hdr=sl.slice(i,he).toString('latin1');let body=sl.slice(he+4);const cl=/Content-Length:\s*(\d+)/i.exec(hdr),ce=/Content-Encoding:\s*([\w-]+)/i.exec(hdr);if(ce&&/gzip/i.test(ce[1])){const gz=body.indexOf(Buffer.from([0x1f,0x8b]));try{body=zlib.gunzipSync(gz>=0?body.slice(gz):body);}catch{continue;}}else if(cl){body=body.slice(0,+cl[1]);}const name=decodeURIComponent((u.split('/').pop()||'f').replace(/[^\w.\-]/g,'_'))||'f';fs.writeFileSync(path.join(OUT,name),body);ok++;}
fs.closeSync(f);console.log('carved',ok,'files ->',OUT);
`,
  },
  {
    id: 'builtin:sym_decrypt',
    title: '对称解密台(AES/DES/3DES,Node stdlib)',
    language: 'node',
    algoTags: ['aes', 'des', '3des', 'cbc', 'ecb', 'decrypt', '对称', 'cryptojs'],
    targetFingerprint: 'AES/DES-CBC 响应解密(key/iv 已找到)',
    params: 'argv: <alg 如 aes-128-cbc|des-ede3-cbc> <keyHex> <ivHex或""> <dataBase64>',
    code: String.raw`// sym_decrypt.js <alg> <keyHex> <ivHex|""> <dataBase64> —— 填好 key/iv 试解,用可打印率做成功判据
// alg 例: aes-128-cbc / aes-256-cbc / des-ede3-cbc ; 国密 SM4 不在 stdlib,需单独骨架
const crypto=require('crypto');
const [ALG,KEY,IV,IN]=process.argv.slice(2);
if(!ALG||!KEY||IN==null){console.error('usage: node sym_decrypt.js <alg> <keyHex> <ivHex|""> <dataBase64>');process.exit(2);}
const key=Buffer.from(KEY,'hex'),iv=IV?Buffer.from(IV,'hex'):null,data=Buffer.from(IN,'base64');
let out;try{const d=crypto.createDecipheriv(ALG,key,iv);out=Buffer.concat([d.update(data),d.final()]);}catch(e){console.error('decrypt failed:',e.message);process.exit(1);}
const txt=out.toString('utf8');const ratio=[...txt].filter(c=>c>=' '||c==='\n'||c==='\t'||c==='\r').length/Math.max(1,txt.length);
console.log(JSON.stringify({bytes:out.length,printableRatio:+ratio.toFixed(3),preview:txt.slice(0,400)}));
`,
  },
  {
    id: 'builtin:js_crypto_finder',
    title: '前端 JS 加密点定位器(CryptoJS/JSEncrypt/国密/key-iv)',
    language: 'node',
    algoTags: ['js', 'crypto', 'cryptojs', 'jsencrypt', 'sm2', 'sm4', 'rsa', 'key', 'webpack', '加密点'],
    targetFingerprint: '在反编译/抠出的 JS 里找加密实现与硬编码 key/iv',
    params: 'argv: <dir>',
    code: String.raw`// js_crypto_finder.js <dir> —— 递归扫 JS/HTML,报加密库与疑似 key/iv 硬编码位置
const fs=require('fs'),path=require('path');
const DIR=process.argv[2]||'.';
const pats=[/CryptoJS/,/JSEncrypt/,/createDecipheriv|createCipheriv/,/\bSM[234]\b|\bsm[234]\b/,/encryptByAES|decryptByAES|aesEncrypt|aesDecrypt/,/publicEncrypt|getKey|selectPublicKey|queryKey/,/["'](key|iv)["']\s*[:=]/i,/[^\w](key|iv)\s*[:=]\s*["'][A-Za-z0-9+/=]{8,}["']/i];
let hits=0;
function walk(d){let es;try{es=fs.readdirSync(d,{withFileTypes:true});}catch{return;}for(const e of es){const p=path.join(d,e.name);if(e.isDirectory())walk(p);else if(/\.(js|mjs|ts|html|jsp)$/i.test(e.name)){let s;try{s=fs.readFileSync(p,'utf8');}catch{continue;}for(const re of pats){const m=re.exec(s);if(m){console.log(p+'  ::  '+re.source.slice(0,40)+'  @'+m.index);hits++;break;}}}}}
walk(DIR);console.error('hits',hits);
`,
  },
];

/** Read operator-approved skeletons from the Tauri app DB, read-only. [] on any failure. */
export function readApprovedAgentTools(appDbPath) {
  if (typeof appDbPath !== 'string' || !appDbPath || !existsSync(appDbPath)) return [];
  try { if (lstatSync(appDbPath).isSymbolicLink()) return []; } catch { return []; }
  let db;
  try {
    db = new DatabaseSync(appDbPath, { readOnly: true });
    const rows = db
      .prepare("SELECT id,title,language,code,target_fingerprint AS targetFingerprint,algo_tags AS algoTags FROM agent_tool_library WHERE status='approved' ORDER BY created_at DESC LIMIT 200")
      .all();
    return rows
      .map(r => ({
        id: String(r.id),
        title: String(r.title ?? ''),
        language: r.language === 'python' ? 'python' : 'node',
        code: String(r.code ?? ''),
        targetFingerprint: String(r.targetFingerprint ?? ''),
        algoTags: (() => { try { const a = JSON.parse(r.algoTags ?? '[]'); return Array.isArray(a) ? a.map(String) : []; } catch { return []; } })(),
      }))
      .filter(t => t.code.trim());
  } catch { return []; } finally { try { db?.close(); } catch { /* ignore */ } }
}

/** All skeletons: built-in seed + operator-approved DB entries. */
export function allAgentTools(appDbPath) {
  return [...BUILTIN_SKELETONS, ...readApprovedAgentTools(appDbPath)];
}

/** Rank skeletons whose algoTags / fingerprint tokens appear in the turn hint. */
export function matchAgentTools(tools, hintText, limit = MAX_SURFACED) {
  const hint = String(hintText ?? '').toLowerCase();
  return (tools || [])
    .map(t => {
      const tokens = [...(t.algoTags || []), ...String(t.targetFingerprint || '').split(/[\s,，、/]+/)]
        .map(x => String(x).toLowerCase().trim())
        .filter(x => x.length >= 2);
      const score = tokens.reduce((n, tok) => n + (hint.includes(tok) ? 1 : 0), 0);
      return { t, score };
    })
    .filter(x => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map(x => x.t);
}

/** Advisory block. Lists MATCHING skeletons (metadata + params, NOT full code). */
export function formatAgentToolsPromptBlock(matched, { total = 0 } = {}) {
  if (!Array.isArray(matched) || !matched.length) {
    return total > 0
      ? `工具库:库里有 ${total} 个可复用的参数化骨架(含内置)。提到具体算法/文件格式(AES/SM4/burp/JS 加密等)我会把对口的列出来;也可直接 recall_agent_tool(id) 取回。`
      : '';
  }
  const lines = matched.map(t =>
    `- [${t.id}] ${clip(t.title, 80)} 〈${(t.algoTags || []).slice(0, 6).join(', ')}〉${t.params ? ' 参数 ' + clip(t.params, 80) : ''}`);
  return `工具库(参数化骨架,按当前目标线索匹配到以下可复用构件——是参考不是授权,不授予任何权限;这些是目标无关的骨架、不是现成答案)。复用:recall_agent_tool(id) 取回代码 → 填入本目标发现的 key/iv/偏移/算法等参数 → 先用 workspace_run 在沙箱重跑验证(解出的字节可打印/含预期字段才算成)→ 再用。切勿不验证就套用别的目标的参数:\n${lines.join('\n')}`;
}

/** recall_agent_tool: return one skeleton's code (built-in, or operator-approved). null if not found. */
export function recallAgentToolCode(appDbPath, id) {
  const wanted = String(id ?? '');
  const builtin = BUILTIN_SKELETONS.find(t => t.id === wanted);
  if (builtin) return builtin.code;
  if (!wanted || typeof appDbPath !== 'string' || !appDbPath || !existsSync(appDbPath)) return null;
  try { if (lstatSync(appDbPath).isSymbolicLink()) return null; } catch { return null; }
  let db;
  try {
    db = new DatabaseSync(appDbPath, { readOnly: true });
    const row = db.prepare("SELECT code FROM agent_tool_library WHERE id=? AND status='approved'").get(wanted);
    return row && row.code ? String(row.code) : null;
  } catch { return null; } finally { try { db?.close(); } catch { /* ignore */ } }
}

export const AGENT_TOOL_RECALL_SPEC = [{
  name: 'recall_agent_tool',
  description: '从工具库取回一个参数化骨架的完整代码(按 id;工具库块会列出可用 id)。取回后写进工作区、填入本目标发现的 key/iv/偏移/算法等参数,先用 workspace_run 在沙箱重跑验证,再用于本目标。骨架是目标无关构件、不是现成答案,务必验证后再用。',
  inputSchema: { type: 'object', properties: { id: { type: 'string', description: '工具库块里列出的骨架 id,如 builtin:sym_decrypt' } }, required: ['id'], additionalProperties: false },
}];
