// Authoring-only schema extraction. Runtime packages contain the reviewed JSON
// snapshot; neither the Host nor an installed app parses desktop source.
import {createRequire} from 'node:module';
import {readFile,writeFile} from 'node:fs/promises';
import {dirname,resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../../..');
const require=createRequire(join(root,'package.json')),ts=require('typescript');
const paths=['src/features/assets/api.ts','src/features/assets/ownershipDirectoryApi.ts','src/features/assets/exposure/api.ts'];
const program=ts.createProgram(paths.map(p=>join(root,p)),{strict:true,noEmit:true,target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,moduleResolution:ts.ModuleResolutionKind.Bundler,skipLibCheck:true});
const checker=program.getTypeChecker();
const unresolved=[];
function schema(type,at,depth=0){
 if(depth>8)throw Error('action_schema_depth:'+at);
 const nullable=type.isUnion()&&type.types.some(t=>t.flags&ts.TypeFlags.Null);
 const choices=type.isUnion()?type.types.filter(t=>!(t.flags&(ts.TypeFlags.Undefined|ts.TypeFlags.Null))):[type];
 let value;
 if(choices.length===0)value={type:'null'};
 else if(choices.every(t=>t.flags&ts.TypeFlags.StringLike)){value={type:'string',maxLength:65536};if(choices.every(t=>t.isStringLiteral()))value.enum=[...new Set(choices.map(t=>t.value))];}
 else if(choices.every(t=>t.flags&ts.TypeFlags.NumberLike))value={type:'integer',minimum:-Number.MAX_SAFE_INTEGER,maximum:Number.MAX_SAFE_INTEGER};
 else if(choices.every(t=>t.flags&ts.TypeFlags.BooleanLike))value={type:'boolean'};
 else if(choices.length!==1){unresolved.push({at,type:checker.typeToString(type)});return null;}
 else if(checker.isArrayType(choices[0]))value={type:'array',maxItems:16384,items:schema(checker.getTypeArguments(choices[0])[0],at+'[]',depth+1)};
 else if(choices[0].flags&ts.TypeFlags.Object){
  if(checker.getIndexInfosOfType(choices[0]).length){unresolved.push({at,type:checker.typeToString(type)});return null;}
  const properties={},required=[];
  for(const property of checker.getPropertiesOfType(choices[0])){
   const child=checker.getTypeOfSymbolAtLocation(property,property.valueDeclaration??property.declarations[0]);
   properties[property.name]=schema(child,at+'.'+property.name,depth+1);
   if(!(property.flags&ts.SymbolFlags.Optional)&&!(child.isUnion()&&child.types.some(t=>t.flags&ts.TypeFlags.Undefined)))required.push(property.name);
  }
  value={type:'object',properties,required,additionalProperties:false};
 }else {unresolved.push({at,type:checker.typeToString(type)});return null;}
 if(nullable&&value.type!=='null')value.type=[value.type,'null'];
 return value;
}
const schemas={};
for(const path of paths){const source=program.getSourceFile(join(root,path));function visit(node){
 if(ts.isCallExpression(node)&&node.expression.getText(source)==='invoke'&&ts.isStringLiteral(node.arguments[0])){
  const name=node.arguments[0].text;schemas[name]=node.arguments[1]?schema(checker.getTypeAtLocation(node.arguments[1]),name):{type:'object',properties:{},required:[],additionalProperties:false};
 }ts.forEachChild(node,visit);
}visit(source);}
const result={schemaVersion:1,sourceFiles:paths,schemas,unresolved};
const output=process.argv[2];if(!output)throw Error('Explicit output required; source snapshots are never silently overwritten');
await writeFile(output,JSON.stringify(result,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({commands:Object.keys(schemas).length,unresolved}));
