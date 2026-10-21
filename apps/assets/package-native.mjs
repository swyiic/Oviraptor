import {chmod,copyFile,lstat,mkdir,mkdtemp,readFile,realpath,rm} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {dirname,isAbsolute,join,resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {createDirectoryPackage} from '../../packages/app-sdk/src/index.mjs';
import {createNativeManifest} from './native-manifest.mjs';
const source=dirname(fileURLToPath(import.meta.url)),root=resolve(source,'../..');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
export async function verifyAssetsUiBuild(){
 const build=JSON.parse(await readFile(join(source,'ui/dist/build-manifest.json'),'utf8'));
 if(build.kind!=='assets-inline-ui-build'||!Array.isArray(build.inputs)||build.inputs.length<10||build.output?.path!=='apps/assets/ui/dist/page.html'||build.externalScripts!==0||build.externalStyles!==0)throw Error('assets_native_ui_build_invalid');
 const paths=new Set();
 for(const item of [...build.inputs,build.output]){
  if(typeof item.path!=='string'||paths.has(item.path)||isAbsolute(item.path)||item.path.split(/[\\/]/).includes('..'))throw Error('assets_native_ui_input_invalid');paths.add(item.path);
  const path=resolve(root,item.path),meta=await lstat(path);if(!meta.isFile()||meta.isSymbolicLink()||await realpath(path)!==path)throw Error('assets_native_ui_input_invalid');
  const bytes=await readFile(path);if(meta.size!==item.bytes||bytes.length!==item.bytes||sha(bytes)!==item.sha256)throw Error('assets_native_ui_build_stale');
 }
 return build;
}
/** Authoring-only packaging. Never execute an input binary or build an installed package. */
export async function packageNativeAssets({binary,outputDirectory,platform=process.platform,arch=process.arch}){
 if(!isAbsolute(binary)||!isAbsolute(outputDirectory))throw Error('native_package_paths_must_be_absolute');
 const meta=await lstat(binary);if(!meta.isFile()||meta.isSymbolicLink()||meta.size<1||meta.size>128*1024*1024)throw Error('native_package_binary_invalid');
 await verifyAssetsUiBuild();
 const staging=await realpath(await mkdtemp(join(tmpdir(),'oviraptor-assets-native-package-'))),manifest=createNativeManifest({platform,arch});
 try{
  await mkdir(join(staging,'bin'),{mode:0o700});await copyFile(binary,join(staging,manifest.entry));await chmod(join(staging,manifest.entry),0o500);
  await copyFile(join(source,'ui/dist/page.html'),join(staging,'page.html'));
  await verifyAssetsUiBuild();
  return await createDirectoryPackage({sourceDirectory:staging,outputDirectory,manifest,files:[manifest.entry,'page.html'],vendorSdk:false});
 }finally{await rm(staging,{recursive:true,force:true});}
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){
 const args=process.argv.slice(2),options={},names=new Map([['--binary','binary'],['--output','outputDirectory'],['--platform','platform'],['--arch','arch']]);if(args.length%2)throw Error('native_package_arguments_invalid');
 for(let i=0;i<args.length;i+=2){const name=names.get(args[i]);if(!name||Object.hasOwn(options,name))throw Error('native_package_arguments_invalid');options[name]=args[i+1];}
 await packageNativeAssets(options);console.log(JSON.stringify({outputDirectory:options.outputDirectory,appId:'org.oviraptor.assets'}));
}
