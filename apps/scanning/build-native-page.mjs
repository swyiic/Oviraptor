/** Author-time only. The Host installer never executes application hooks. */
import { build } from 'esbuild';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const directory = dirname(fileURLToPath(import.meta.url));
const inputs = ['native.html', 'native.css', 'native-page.mjs', 'native-controller.mjs', '../../packages/app-sdk/src/page-client.mjs'];
export const nativePageArtifact = join(directory, 'native-bundle.html');

export async function nativePageSourceDigest() {
  const hash = createHash('sha256');
  for (const path of inputs) { hash.update(path); hash.update('\0'); hash.update(await readFile(join(directory, path))); hash.update('\0'); }
  return hash.digest('hex');
}

export async function buildNativePage({ write = true } = {}) {
  const bundled = await build({ entryPoints: [join(directory, 'native-page.mjs')], bundle: true, format: 'iife', platform: 'browser', target: 'es2022', write: false, legalComments: 'none' });
  const javascript = bundled.outputFiles[0].text.replace(/<\/script/giu, '<\\/script');
  const css = (await readFile(join(directory, 'native.css'), 'utf8')).replace(/<\/style/giu, '<\\/style');
  const source = await readFile(join(directory, 'native.html'), 'utf8');
  const styleTag = '<link rel="stylesheet" href="native.css">', scriptTag = '<script type="module" src="native-page.mjs"></script>';
  if (source.split(styleTag).length !== 2 || source.split(scriptTag).length !== 2) throw Error('scanning_page_author_template_changed');
  const html = `<!-- scanning-page-source-sha256:${await nativePageSourceDigest()} -->\n` + source.replace(styleTag, `<style>${css}</style>`).replace(scriptTag, `<script>${javascript}</script>`);
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
