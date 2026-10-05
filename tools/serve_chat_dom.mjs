// Local-only acceptance fixture; never imported by the production application.
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';

const repo = fileURLToPath(new URL('../', import.meta.url));
const component = path.join(repo, 'src/features/sentinel/components/AgentDialog.vue');
const transport = path.join(repo, 'tools/chat_dom/transport.mjs');
export const chatDomTransport = {
  name: 'chat-dom-fixture-transport', enforce: 'pre',
  resolveId(source, importer) {
    const file = importer?.split('?')[0];
    const chatModule = file === component || (file
      && path.dirname(file) === path.join(repo, 'src/features/sentinel/composables')
      && /^useAgentDialog[A-Za-z]+\.ts$/.test(path.basename(file)));
    if (chatModule && ['../api', '@tauri-apps/api/event'].includes(source)) return transport;
    // Fail closed if a future refactor introduces an unmocked native import.
    if (source.startsWith('@tauri-apps/')) throw new Error('fixture_native_transport_forbidden');
  },
};
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const server = await createServer({
    configFile: false,
    root: path.join(repo, 'tools/chat_dom'),
    publicDir: false,
    plugins: [chatDomTransport, vue()],
    server: {
      host: '127.0.0.1', port: 1439, strictPort: true,
      fs: { allow: [repo] },
      watch: { ignored: ['**/src-tauri/**'] },
    },
  });
  await server.listen();
  console.log('Chat DOM fixture: http://127.0.0.1:1439 — synthetic data, no Tauri/target/model calls');
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, async () => {
    await server.close();
    process.exit(0);
  });
}
