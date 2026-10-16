import readline from 'node:readline';

const origin = process.env.OVIRAPTOR_HOST_ORIGIN;
const token = process.env.OVIRAPTOR_MCP_TOKEN;
const conversation = process.env.OVIRAPTOR_CONVERSATION;
if (!origin || !token || !conversation || !/^http:\/\/127\.0\.0\.1:\d+$/.test(origin)) process.exit(2);
const lines = readline.createInterface({ input: process.stdin });
for await (const line of lines) {
  if (Buffer.byteLength(line) > 65536) process.exit(2);
  let request;
  try { request = JSON.parse(line); } catch { process.exit(2); }
  if (request.id === undefined) continue;
  try {
    const response = await fetch(`${origin}/mcp/${encodeURIComponent(conversation)}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Oviraptor-Mcp': token }, body: JSON.stringify({ method: request.method, params: request.params ?? {} }), signal: AbortSignal.timeout(30000) });
    if (!response.ok) throw Error('unavailable');
    const result = await response.json();
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\n');
  } catch { process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, error: { code: -32000, message: 'Scoped application capability unavailable' } }) + '\n'); }
}
