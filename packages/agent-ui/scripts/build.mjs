import { cp, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import crypto from 'node:crypto';
const root=fileURLToPath(new URL('../',import.meta.url));
// This package has no dependency on the legacy renderer, any business application,
// a bundler, or the root node_modules. Source remains native browser ESM.
await rm(path.join(root,'dist'),{recursive:true,force:true});
await mkdir(path.join(root,'dist'),{recursive:true});
await cp(path.join(root,'src'),path.join(root,'dist/src'),{recursive:true});
await cp(path.join(root,'index.html'),path.join(root,'dist/index.html'));
const files=[];
async function walk(dir){for(const entry of await readdir(dir,{withFileTypes:true})){const file=path.join(dir,entry.name);if(entry.isDirectory())await walk(file);else files.push({path:path.relative(path.join(root,'dist'),file),sha256:crypto.createHash('sha256').update(await readFile(file)).digest('hex')});}}
await walk(path.join(root,'dist'));
await writeFile(path.join(root,'dist/build-manifest.json'),JSON.stringify({package:'@oviraptor/agent-ui',format:'native-esm',files},null,2)+'\n');
console.log(`Built standalone agent-ui: ${files.length} files, zero external dependencies.`);
