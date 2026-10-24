const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const source = fs.readFileSync(path.join(__dirname,'../src/features/assets/uiPorts.ts'),'utf8');
function ports(window, invoke) {
 const exports = {};
 const code = ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
 new Function('require','exports','window',code)(name=>name==='@tauri-apps/api/core'?{invoke}: {},exports,window);
 return exports;
}
test('desktop confirmations reach the native dialog and grant only an explicit affirmative reply',async()=>{
 const calls=[];let answer='取消';
 const p=ports({__TAURI_INTERNALS__:{},confirm(){throw Error('unsupported browser confirm must not run')}},async(...args)=>{calls.push(args);return answer;});
 assert.equal(await p.assetConfirm('Confirm these exact origins?'),false);
 answer='确认';assert.equal(await p.assetConfirm('Confirm these exact origins?'),true);
 for(answer of [undefined,null,true,'Ok','Yes',''])assert.equal(await p.assetConfirm('bounded confirmation'),false);
 assert.equal(calls[0][0],'plugin:dialog|message');
 assert.deepEqual(calls[0][1].buttons,{OkCancelCustom:['确认','取消']});
 const capabilities=JSON.parse(fs.readFileSync(path.join(__dirname,'../src-tauri/capabilities/default.json'),'utf8'));
 assert(capabilities.permissions.includes('dialog:allow-confirm'));
});
test('native dialog failure does not fall back to an implicit approval',async()=>{
 const p=ports({__TAURI_INTERNALS__:{},confirm:()=>true},async()=>{throw Error('native permission unavailable')});
 await assert.rejects(p.assetConfirm('sensitive action'),/native permission unavailable/);
});
test('browser previews keep the ordinary browser confirmation',async()=>{
 let seen;const p=ports({confirm:message=>{seen=message;return false}},()=>{throw Error('native IPC in browser')});
 assert.equal(await p.assetConfirm('preview only'),false);assert.equal(seen,'preview only');
});
