import { cp, mkdir, rm, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const output = join(repository, '.agent-build', 'agent-first');
await rm(output, { recursive: true, force: true });
await mkdir(join(output, 'packages'), { recursive: true });
for (const name of ['agent-host', 'app-sdk', 'skills-library']) {
  await cp(join(repository, 'packages', name, 'src'), join(output, 'packages', name, 'src'), { recursive: true });
  await cp(join(repository, 'packages', name, 'package.json'), join(output, 'packages', name, 'package.json'));
}
await mkdir(join(output, 'packages', 'skills-library', 'resources'), { recursive: true });
await cp(join(repository, 'src-tauri', 'resources', 'skill-packages', 'compound-engineering.json.gz'), join(output, 'packages', 'skills-library', 'resources', 'compound-engineering.json.gz'));
await cp(join(repository, 'src-tauri', 'resources', 'skill-packages', 'COMPOUND-ENGINEERING-LICENSE'), join(output, 'packages', 'skills-library', 'resources', 'COMPOUND-ENGINEERING-LICENSE'));
await mkdir(join(output, 'packages', 'runtime-acp'), { recursive: true });
await cp(join(repository, 'packages', 'runtime-acp', 'index.mjs'), join(output, 'packages', 'runtime-acp', 'index.mjs'));
await cp(join(repository, 'packages', 'runtime-acp', 'package.json'), join(output, 'packages', 'runtime-acp', 'package.json'));
await cp(join(repository, 'packages', 'agent-ui', 'dist'), join(output, 'packages', 'agent-ui', 'dist'), { recursive: true });
process.stdout.write(`Independent host built: ${output}\n`);
