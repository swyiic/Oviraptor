// Authoring-time build only. Installing an app never executes this script.
// The existing Assets owner remains the single source for both UI surfaces.
import {build} from 'vite';
import vue from '@vitejs/plugin-vue';
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {dirname,relative,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const app=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const root=resolve(app,'../..');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const inputs=new Set();
// Vite inlines CSS @imports before Rollup's module graph. Bind those source
// files too, rather than claiming the top-level stylesheet is a full closure.
async function cssInputs(filename){if(inputs.has(filename))return;inputs.add(filename);const source=await readFile(filename,'utf8');for(const match of source.matchAll(/@import\s+['"]([^'"]+)['"]\s*;/g))await cssInputs(resolve(dirname(filename),match[1]));}
await cssInputs(resolve(app,'ui/style.css'));
inputs.add(fileURLToPath(import.meta.url));inputs.add(resolve(app,'package.json'));inputs.add(resolve(app,'ui/tsconfig.json'));
const forbidden=id=>id.includes('/packages/agent-host/')||id.includes('/src/features/sentinel/')||id.includes('/node_modules/@tauri-apps/')||id===resolve(root,'src/i18n.ts');
const ports=new Map([[resolve(root,'src/features/assets/uiPorts'),resolve(app,'ui/ports/uiPorts.ts')],[resolve(root,'src/features/sentinel/api'),resolve(app,'ui/ports/scanning.ts')]]);
const output=await build({configFile:false,root:app,logLevel:'warn',define:{'process.env.NODE_ENV':JSON.stringify('production')},plugins:[{
  name:'assets-explicit-ui-ports',enforce:'pre',
  resolveId(source,importer){if(!importer||!source.startsWith('.'))return;return ports.get(resolve(dirname(importer.split('?')[0]),source).replace(/\.ts$/,''));},
},vue(),{
  name:'assets-owner-boundary',
  generateBundle(){for(const id of this.getModuleIds()){const filename=id.split('?')[0];if(forbidden(filename))throw Error(`Assets page crossed its runtime boundary: ${relative(root,filename)}`);if(filename.startsWith(root+'/')&&!filename.includes('/node_modules/'))inputs.add(filename);}},
}],build:{write:false,minify:true,cssCodeSplit:false,lib:{entry:resolve(app,'ui/main.ts'),name:'OviraptorAssetsPage',formats:['iife']},rollupOptions:{output:{inlineDynamicImports:true}},emptyOutDir:false}});
const bundles=Array.isArray(output)?output:[output];
if(bundles.length!==1)throw Error('Assets page requires exactly one bundle');
const files=bundles[0].output;
const chunks=files.filter(item=>item.type==='chunk');
if(chunks.length!==1||chunks[0].imports.length||chunks[0].dynamicImports.length)throw Error('Assets page must be one self-contained script');
const unexpected=files.filter(item=>item.type==='asset'&&!item.fileName.endsWith('.css'));
if(unexpected.length)throw Error(`Unexpected external Assets page resources: ${unexpected.map(item=>item.fileName).join(',')}`);
const css=files.filter(item=>item.type==='asset').map(item=>String(item.source)).join('\n');
const html=`<!doctype html><html lang="zh"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Oviraptor Assets</title><style>${css.replace(/<\/style/gi,'<\\/style')}</style></head><body><div id="app"></div><script>${chunks[0].code.replace(/<\/script/gi,'<\\/script')}</script></body></html>\n`;
const records=[];for(const path of [...inputs].sort()){const bytes=await readFile(path);records.push({path:relative(root,path),bytes:bytes.length,sha256:sha(bytes)});}
const directory=resolve(app,'ui/dist');await mkdir(directory,{recursive:true});await writeFile(resolve(directory,'page.html'),html);
const manifest={schemaVersion:1,kind:'assets-inline-ui-build',sharedBusinessSource:'src/features/assets',sandbox:'allow-scripts',externalScripts:0,externalStyles:0,inputs:records,output:{path:'apps/assets/ui/dist/page.html',bytes:Buffer.byteLength(html),sha256:sha(html)}};
await writeFile(resolve(directory,'build-manifest.json'),JSON.stringify(manifest,null,2)+'\n');
console.log(JSON.stringify({inputs:records.length,...manifest.output}));
