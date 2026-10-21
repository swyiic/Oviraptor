import supplemental from '../native-supplemental-actions.json' with {type:'json'};
import {readFile,writeFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {createHash} from 'node:crypto';
import {dirname,resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {manualOverrides,assetFilter,string,integer,boolean,array,object,optionalNull} from '../native-schema-overrides.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../../..');
const require=createRequire(join(root,'package.json')),ts=require('typescript');
const derived=JSON.parse(await readFile(process.argv[2],'utf8')),output=process.argv[3];if(!output)throw Error('Explicit input and exclusive output required');
const schemas=derived.schemas;
for(const [name,properties]of Object.entries(manualOverrides)){if(!schemas[name])throw Error('Missing facade command:'+name);for(const [key,value]of Object.entries(properties))schemas[name].properties[key]=value;}
schemas.export_exposure_findings=object({projectId:integer(),kind:{type:'string',enum:['findings','ledger']}},['projectId']);
const configPath='src/features/configuration/configProfileDefaults.ts',source=await readFile(join(root,configPath),'utf8');
const sourceFile=ts.createSourceFile(configPath,source,ts.ScriptTarget.Latest,true);
let defaults;function find(node){if(ts.isVariableDeclaration(node)&&node.name.getText(sourceFile)==='CONFIG_PROFILE_SETTING_DEFAULTS')defaults=node.initializer;ts.forEachChild(node,find);}find(sourceFile);if(!defaults||!ts.isObjectLiteralExpression(defaults))throw Error('Config defaults missing');
const model=object({id:string(128),name:string(),llm:string(),apiBase:string(),apiKey:string(),localApiKey:string(),deployment:{type:'string',enum:['cloud','local']}},['id','name','llm','apiBase','apiKey','localApiKey','deployment']);
function syntax(node,key){
 if(ts.isAsExpression(node)||ts.isParenthesizedExpression(node))return syntax(node.expression,key);
 if(key==='modelProfiles')return array(model,128);
 if(ts.isStringLiteral(node))return string();
 if(ts.isNumericLiteral(node))return{type:'number',minimum:-Number.MAX_SAFE_INTEGER,maximum:Number.MAX_SAFE_INTEGER};
 if([ts.SyntaxKind.TrueKeyword,ts.SyntaxKind.FalseKeyword].includes(node.kind))return boolean();
 if(ts.isArrayLiteralExpression(node)){if(!node.elements.length)return array(string());return array(syntax(node.elements[0],key));}
 if(ts.isObjectLiteralExpression(node)){const props={};for(const p of node.properties){if(!ts.isPropertyAssignment(p))throw Error('Unsupported config default');props[p.name.getText(sourceFile).replace(/^['"]|['"]$/g,'')]=syntax(p.initializer,p.name.getText(sourceFile));}return object(props);}
 throw Error('Unresolved config field:'+key);
}
const settings={};for(const p of defaults.properties){if(!ts.isPropertyAssignment(p))throw Error('Unsupported config defaults shape');const key=p.name.getText(sourceFile);settings[key]=syntax(p.initializer,key);}
const nativePath='src-tauri/src/db_initialize_budgets.rs',nativeSource=await readFile(join(root,nativePath),'utf8');const raw=nativeSource.split('let defaults = json!(')[1]?.split('\n        });')[0];if(!raw)throw Error('Native defaults missing');const nativeDefaults=JSON.parse(raw+'}');
function valueSchema(value){return typeof value==='boolean'?boolean():typeof value==='number'?{type:'number',minimum:-Number.MAX_SAFE_INTEGER,maximum:Number.MAX_SAFE_INTEGER}:typeof value==='string'?string():Array.isArray(value)?array(value.length?valueSchema(value[0]):string()):(()=>{throw Error('Unsupported native default')})();}
for(const [key,value]of Object.entries(nativeDefaults))if(!settings[key])settings[key]=valueSchema(value);
const jobsPath='src-tauri/src/jobs.rs',jobsSource=await readFile(join(root,jobsPath),'utf8');
for(const m of jobsSource.matchAll(/setting_(bool|i64|f64|string)\([^\n]*?,\s*"([A-Za-z0-9_]+)"/g)){const key=m[2];if(!settings[key])settings[key]=m[1]==='bool'?boolean():m[1]==='string'?string():m[1]==='f64'?{type:'number',minimum:-Number.MAX_SAFE_INTEGER,maximum:Number.MAX_SAFE_INTEGER}:integer();}
settings.collectionConfigurationId=string(128);settings.modelName=string();settings.configPath={type:'string',maxLength:0};
// Existing JSON profiles can explicitly contain null (notably modelProfiles).
// Keep that distinction; do not replace it with invented application defaults.
for(const key of Object.keys(settings))settings[key]=optionalNull(settings[key]);
const settingsSchema=object(settings);
Object.assign(schemas,{
 assets_context:object({}),list_asset_job_profiles:object({}),list_config_profiles:object({}),
 save_config_profile:object({input:object({id:optionalNull(integer()),name:string(),description:string(),isDefault:boolean(),settings:settingsSchema},['name','settings'])},['input']),
 delete_config_profile:object({profileId:integer()},['profileId']),
 import_asset_collection_configuration:object({content:string(65536)},['content']),
 read_asset_artifact:object({artifactId:string(128),offset:{type:'integer',minimum:0},maxBytes:{type:'integer',minimum:1,maximum:196608}},['artifactId','offset','maxBytes']),
 saved_assets_stats:object({filter:assetFilter}),saved_assets_search:object({filter:assetFilter,limit:{type:'integer',minimum:1,maximum:20},cursor:string(2048)}),
});
for(const action of supplemental.actions){if(schemas[action.name])throw Error('Duplicate finite action:'+action.name);schemas[action.name]=action.inputSchema;}
function verify(value,path){if(value===null||value===undefined)throw Error('Unresolved typed schema:'+path);if(value.type==='object'){if(Object.keys(value.properties).length>256)throw Error('Too many fields:'+path);for(const [key,child]of Object.entries(value.properties))verify(child,path+'.'+key);}if(value.type==='array')verify(value.items,path+'[]');}
for(const [name,value]of Object.entries(schemas))verify(value,name);
const inputs=[];for(const path of [...derived.sourceFiles,configPath,nativePath,jobsPath,'apps/assets/native-schema-overrides.mjs','apps/assets/native-supplemental-actions.json']){const bytes=await readFile(join(root,path));inputs.push({path,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')});}
await writeFile(output,JSON.stringify({schemaVersion:1,derivation:'Actual TS facade argument types; finite JsonLike overrides from named Rust owners; complete known profile settings from current desktop/native defaults plus actual Jobs setting reads. No generic JSON payloads.',inputs,profileSettingsFields:Object.keys(settings).sort(),schemas},null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({actions:Object.keys(schemas).length,profileSettingsFields:Object.keys(settings).length,output}));
