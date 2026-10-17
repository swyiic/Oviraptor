// Shared app-owned SQLite helpers for Node workers (assets, scanning, …).
// Vendored into installed packages as sdk/sqlite-data.mjs when vendorSdk is true.
import { DatabaseSync } from 'node:sqlite';
import { existsSync, lstatSync, realpathSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
export const object=v=>v!==null&&typeof v==='object'&&!Array.isArray(v);
export function check(value,code='app_arguments_invalid'){if(!value)throw Error(code);}
export function text(value,max=256,{empty=false}={}){check(typeof value==='string'&&value.length<=max&&!/[\u0000-\u001f\u007f]/u.test(value)&&(empty||value.trim().length>0));return value;}
export function context(ctx){check(object(ctx)&&Number.isSafeInteger(ctx.hostBoundProjectId)&&ctx.hostBoundProjectId>0&&Number.isSafeInteger(ctx.grantRevision)&&ctx.grantRevision>=0,'app_context_invalid');text(ctx.operationId,128);return ctx.hostBoundProjectId;}
export function exact(args,keys){check(object(args)&&Object.keys(args).every(k=>keys.includes(k)));}
export function open(dataDir,name){check(typeof dataDir==='string'&&resolve(dataDir)===realpathSync(dataDir),'app_data_directory_invalid');const path=join(dataDir,name);check(!existsSync(path)||(!lstatSync(path).isSymbolicLink()&&lstatSync(path).isFile()),'app_data_file_invalid');const db=new DatabaseSync(path);db.exec('PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA busy_timeout=2000;CREATE TABLE IF NOT EXISTS operations(project_id INTEGER NOT NULL,operation_id TEXT NOT NULL,tool TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,result_json TEXT NOT NULL,PRIMARY KEY(project_id,operation_id));');return db;}
const canonical=v=>Array.isArray(v)?v.map(canonical):object(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,canonical(v[k])])):v;
export function operation(db,ctx,tool,args,write){const project=context(ctx);const hash=createHash('sha256').update(JSON.stringify(canonical(args))).digest('hex');db.exec('BEGIN IMMEDIATE');try{const old=db.prepare('SELECT * FROM operations WHERE project_id=? AND operation_id=?').get(project,ctx.operationId);if(old){check(old.tool===tool&&old.arguments_sha256===hash,'app_operation_conflict');db.exec('COMMIT');return JSON.parse(old.result_json);}const result=write(project);db.prepare('INSERT INTO operations VALUES(?,?,?,?,?)').run(project,ctx.operationId,tool,hash,JSON.stringify(result));db.exec('COMMIT');return result;}catch(error){db.exec('ROLLBACK');throw error;}}
export const result=value=>({content:[{type:'text',text:JSON.stringify(value)}],structuredContent:value});
export function scope(project){return{hostBoundProjectId:project,asOf:new Date().toISOString(),storage:'app_owned_sqlite',liveVerification:false};}
