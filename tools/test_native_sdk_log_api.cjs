const assert=require('node:assert/strict'),test=require('node:test'),fs=require('node:fs'),path=require('node:path'),ts=require('typescript');
test('SDK log actual public API dispatches dedicated IPC with original scope and cursor only',async()=>{
 const exports={},calls=[];new Function('require','exports',ts.transpileModule(fs.readFileSync(path.join(__dirname,'../src/features/sentinel/api.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText)(name=>{
  assert.equal(name,'@tauri-apps/api/core');return {invoke:async(...args)=>{calls.push(args);return {real:'transport-fixture'};}};
 },exports);
 assert.equal(typeof exports.sentinelApi.readNativeSdkLog,'function','existing public API must expose the separate committed SDK reader');
 const value=await exports.sentinelApi.readNativeSdkLog('A',2,2,19,100,'a'.repeat(64));
 assert.deepEqual(calls,[['read_native_sdk_log',{scanId:'A',attempt:2,cursorAttempt:2,ownerId:'a'.repeat(64),afterSequence:19,limit:100}]]);assert.deepEqual(value,{real:'transport-fixture'});
});
