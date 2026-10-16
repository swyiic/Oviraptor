/** Author-time only. The Host installer never executes application hooks. */
import { build } from 'esbuild';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const directory = dirname(fileURLToPath(import.meta.url));
const repository = resolve(directory, '../..');
const authorInputs = ['native.html', 'native.css', 'build-native-page.mjs'];
export const nativePageArtifact = join(directory, 'native-bundle.html');

function checkedInputs(inputs) {
  if (!Array.isArray(inputs) || !inputs.length || inputs.length > 128 ||
      inputs.some(path => typeof path !== 'string' || !path || path.length > 1024 || isAbsolute(path) || path.includes('\\') ||
        !resolve(directory, path).startsWith(repository + sep)) ||
      new Set(inputs).size !== inputs.length || JSON.stringify([...inputs].sort()) !== JSON.stringify(inputs)) throw Error('scanning_native_page_input_manifest_invalid');
  for (const path of authorInputs) if (!inputs.includes(path)) throw Error('scanning_native_page_input_manifest_invalid');
  return inputs;
}

export function nativePageInputManifest(html) {
  const encoded = /^<!-- scanning-page-source-sha256:[a-f0-9]{64} -->\n<!-- scanning-page-inputs:([A-Za-z0-9+/=]+) -->\n/u.exec(html)?.[1];
  if (!encoded || encoded.length > 32 * 1024) throw Error('scanning_native_page_input_manifest_invalid');
  let inputs;
  try { inputs = JSON.parse(Buffer.from(encoded, 'base64').toString('utf8')); } catch { throw Error('scanning_native_page_input_manifest_invalid'); }
  return checkedInputs(inputs);
}

async function digestInputs(inputs) {
  const hash = createHash('sha256');
  for (const path of checkedInputs(inputs)) { hash.update(path); hash.update('\0'); hash.update(await readFile(resolve(directory, path))); hash.update('\0'); }
  return hash.digest('hex');
}

export async function nativePageSourceDigest() {
  // Packaging only reads the already-built artifact and hashes its closed input
  // set. A changed import statement changes an included input, so it is stale.
  return digestInputs(nativePageInputManifest(await readFile(nativePageArtifact, 'utf8')));
}

export async function buildNativePage({ write = true } = {}) {
  const bundled = await build({ absWorkingDir: repository, entryPoints: [join(directory, 'native-page.mjs')], bundle: true, format: 'iife', platform: 'browser', target: 'es2022', write: false, legalComments: 'none', metafile: true });
  const inputs = [...new Set([...authorInputs, ...Object.keys(bundled.metafile.inputs).map(path => relative(directory, resolve(repository, path)).split(sep).join('/'))])].sort();
  const javascript = bundled.outputFiles[0].text.replace(/<\/script/giu, '<\\/script');
  const css = (await readFile(join(directory, 'native.css'), 'utf8')).replace(/<\/style/giu, '<\\/style');
  const source = await readFile(join(directory, 'native.html'), 'utf8');
  const styleTag = '<link rel="stylesheet" href="native.css">', scriptTag = '<script type="module" src="native-page.mjs"></script>';
  if (source.split(styleTag).length !== 2 || source.split(scriptTag).length !== 2) throw Error('scanning_page_author_template_changed');
  const html = `<!-- scanning-page-source-sha256:${await digestInputs(inputs)} -->\n<!-- scanning-page-inputs:${Buffer.from(JSON.stringify(inputs)).toString('base64')} -->\n` + source.replace(styleTag, `<style>${css}</style>`).replace(scriptTag, `<script>${javascript}</script>`);
  if (write) await writeFile(nativePageArtifact, html);
  return html;
}

export async function readNativePageArtifact() {
  const html = await readFile(nativePageArtifact, 'utf8');
  if (!html.startsWith(`<!-- scanning-page-source-sha256:${await nativePageSourceDigest()} -->\n`)) throw Error('scanning_native_page_stale_run_author_build');
  return html;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length !== 2) throw Error('usage: node apps/scanning/build-native-page.mjs');
  const html = await buildNativePage();
  console.log(JSON.stringify({ artifact: nativePageArtifact, bytes: Buffer.byteLength(html), sourceSha256: await nativePageSourceDigest() }));
}
