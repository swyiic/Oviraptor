import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from './service.mjs';
import { serveHost } from './server.mjs';

const args = process.argv.slice(2);
function option(name, fallback) { const index = args.indexOf(name); return index < 0 ? fallback : args[index + 1]; }
const runtimeSelection = option('--runtime', undefined);
if (args.includes('--runtime') && !['none', 'grok-acp'].includes(runtimeSelection)) {
  process.stderr.write('Expected --runtime none or --runtime grok-acp.\n'); process.exit(2);
}
const dataDir = resolve(option('--data-dir', './.oviraptor-agent-host'));
let host;
const { GrokAcpRuntime } = await import('../../runtime-acp/index.mjs');
const runtimeFactory = () => new GrokAcpRuntime({ cwd: dataDir, permissionPolicy: 'ask', onEvent: event => host?.runtimeEvent(event) });
host = new AgentHost({ dataDir, runtimeSelection, runtimeFactory });
await host.ensureBundledApps();
const web = await serveHost(host, { port: Number(option('--port', '0')), uiDir: fileURLToPath(new URL('../../agent-ui/dist/', import.meta.url)) });
let closing = false;
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, async () => {
  if (closing) return; closing = true;
  try { await host.close(); await web.close(); process.exitCode = 0; }
  catch { process.stderr.write('Active application work prevents clean shutdown.\n'); closing = false; }
});
process.stdout.write(`Oviraptor host: ${web.origin}\nRuntime: ${host.settings().runtime.id === 'grok-acp' ? 'grok-acp (not connected until an explicit message)' : 'unconfigured (save-only chat)'}\n`);
