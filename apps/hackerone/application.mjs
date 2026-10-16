import { mkdir, readFile, writeFile, rename, rm, lstat } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { MAX_TOOL_RESULT_BYTES } from './sdk/protocol.mjs';

const SCHEMA_VERSION = 1;
const MAX_STATE_BYTES = 512 * 1024;
const MAX_PROGRAMS = 256;
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const closedKeys = (value, allowed) => object(value) && Object.keys(value).every(key => allowed.includes(key));
function problem(message) { throw new Error(message); }

function validateState(value) {
  if (!closedKeys(value, ['schemaVersion', 'programs', 'note']) || value.schemaVersion !== SCHEMA_VERSION
    || typeof value.note !== 'string' || [...value.note].length > 512
    || !Array.isArray(value.programs) || value.programs.length > MAX_PROGRAMS) problem('hackerone_data_invalid');
  for (const row of value.programs) {
    if (!closedKeys(row, ['handle', 'name', 'summary', 'updatedAt'])
      || typeof row.handle !== 'string' || !row.handle || row.handle.length > 128
      || typeof row.name !== 'string' || row.name.length > 256
      || typeof row.summary !== 'string' || row.summary.length > 2048
      || typeof row.updatedAt !== 'string' || row.updatedAt.length > 64) problem('hackerone_data_invalid');
  }
  return value;
}

export function createHackerOneApplication(manifest) {
  let dataFile;
  let state;
  return {
    manifest,
    async onStart({ dataDir }) {
      await mkdir(dataDir, { recursive: true, mode: 0o700 });
      if ((await lstat(dataDir)).isSymbolicLink()) problem('hackerone_data_invalid');
      dataFile = join(dataDir, 'hackerone-cache.json');
      let raw;
      try {
        const info = await lstat(dataFile);
        if (!info.isFile() || info.isSymbolicLink() || info.size > MAX_STATE_BYTES) problem('hackerone_data_invalid');
        raw = await readFile(dataFile, 'utf8');
      } catch (error) { if (error.code !== 'ENOENT') throw error; }
      if (raw === undefined) {
        state = {
          schemaVersion: SCHEMA_VERSION,
          programs: [],
          note: 'Local cache empty. Desktop shell sync remains the source of program records until a controlled bridge is added.',
        };
        const temporary = `${dataFile}.${randomUUID()}.tmp`;
        try {
          await writeFile(temporary, `${JSON.stringify(state)}\n`, { flag: 'wx', mode: 0o600 });
          await rename(temporary, dataFile);
        } finally { await rm(temporary, { force: true }); }
      } else {
        state = validateState(JSON.parse(raw));
      }
      return { dataSchemaVersion: SCHEMA_VERSION };
    },
    async onStop() { return { busy: false }; },
    tools: {
      async programs_list(args = {}) {
        const search = typeof args.search === 'string' ? args.search.trim().toLowerCase() : '';
        const offset = Number.isSafeInteger(args.offset) ? args.offset : 0;
        const limit = Number.isSafeInteger(args.limit) ? args.limit : 20;
        let rows = state.programs.map(({ handle, name, summary, updatedAt }) => ({ handle, name, summary, updatedAt }));
        if (search) rows = rows.filter(row => row.handle.toLowerCase().includes(search) || row.name.toLowerCase().includes(search));
        const total = rows.length;
        const page = rows.slice(offset, offset + limit);
        const result = () => ({
          content: [{ type: 'text', text: `${total} cached program(s). Read-only local cache; no remote sync performed.` }],
          structuredContent: {
            programs: page,
            total,
            offset,
            nextOffset: offset + page.length < total ? offset + page.length : null,
            note: state.note,
            authority: 'local-cache-read-only',
          },
        });
        while (page.length && Buffer.byteLength(JSON.stringify(result())) > MAX_TOOL_RESULT_BYTES) page.pop();
        return result();
      },
      async program_get(args) {
        const handle = String(args.handle || '').trim();
        const row = state.programs.find(item => item.handle === handle);
        if (!row) {
          return {
            content: [{ type: 'text', text: `No local cache entry for handle ${handle}.` }],
            structuredContent: { found: false, handle, note: state.note, authority: 'local-cache-read-only' },
          };
        }
        return {
          content: [{ type: 'text', text: `${row.name} (${row.handle})` }],
          structuredContent: { found: true, program: row, authority: 'local-cache-read-only' },
        };
      },
    },
  };
}
