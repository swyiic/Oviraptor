// A real subprocess protocol peer. This is deliberately not a Rust SDK or
// business-engine acceptance fixture; it exercises Host process ownership.
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';

const [directory, mode] = process.argv.slice(2);
const bytes = await readFile(`${directory}/manifest.json`);
const manifest = JSON.parse(bytes);
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
const keepAlive = setInterval(() => {}, 1000);
let backgroundBusy = mode === 'busy';
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  const reply = result => send({ jsonrpc: '2.0', id: request.id, result });
  if (request.method === 'describe') reply({ protocolVersion: 2, appId: manifest.id, version: manifest.version,
    manifestSha256: mode === 'bad-description' ? '0'.repeat(64) : createHash('sha256').update(bytes).digest('hex'),
    toolNames: [], actionNames: ['read'], pageIds: ['main'] });
  else if (request.method === 'lifecycle/start') reply({ dataSchemaVersion: 1 });
  else if (request.method === 'lifecycle/status') {
    const status = { busy: backgroundBusy, privateDomainValue: 'never-forward-this' };
    if (mode === 'uncertain-status') status.uncertain = true;
    if (mode === 'uncertain-string') status.uncertain = 'unknown';
    for (const key of ['outcomeUnknown', 'ownerUnresolved', 'quarantined']) if (mode === key) status[key] = true;
    if (mode === 'oversized-status') status.privateDomainValue = 'x'.repeat(5000);
    reply(mode === 'unknown-status' ? {} : status);
  }
  else if (request.method === 'action/call') {
    if (mode === 'timeout') return;
    if (mode === 'dead-owner') process.exit(0);
    if (mode === 'malformed') return send({ jsonrpc: '2.0', id: request.id, result: {}, error: { code: -1, message: 'invalid' } });
    if (mode === 'quiet-read' && request.params.name === 'begin') backgroundBusy = true;
    if (mode === 'quiet-read' && request.params.name === 'finish') backgroundBusy = false;
    if (mode !== 'quiet-read') send({ jsonrpc: '2.0', method: 'application/event', params: { name: 'changed', payload: { privateDomainValue: 'never-forward-this' } } });
    reply({ rawLegacyDto: 7 });
  } else if (request.method === 'lifecycle/stop') {
    if (mode === 'busy') reply({ busy: true });
    else { reply({ busy: false }); clearInterval(keepAlive); process.stdin.destroy(); }
  }
});
