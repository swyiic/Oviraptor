import { mkdir, readFile, writeFile, rename, rm, lstat } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

const SCHEMA_VERSION = 1;
const MAX_NOTES = 32;

export function createFixtureAuthorityApplication(manifest) {
  let dataFile;
  let state = { schemaVersion: SCHEMA_VERSION, notes: [] };
  let queue = Promise.resolve();
  let pending = 0;
  async function persist(next) {
    const temporary = `${dataFile}.${randomUUID()}.tmp`;
    try {
      await writeFile(temporary, `${JSON.stringify(next)}\n`, { flag: 'wx', mode: 0o600 });
      await rename(temporary, dataFile);
    } finally { await rm(temporary, { force: true }); }
  }
  return {
    manifest,
    async onStart({ dataDir }) {
      await mkdir(dataDir, { recursive: true, mode: 0o700 });
      if ((await lstat(dataDir)).isSymbolicLink()) throw new Error('fixture_data_invalid');
      dataFile = join(dataDir, 'fixture-state.json');
      try {
        const raw = await readFile(dataFile, 'utf8');
        const parsed = JSON.parse(raw);
        if (parsed?.schemaVersion === SCHEMA_VERSION && Array.isArray(parsed.notes)) state = parsed;
        else { state = { schemaVersion: SCHEMA_VERSION, notes: [] }; await persist(state); }
      } catch (error) {
        if (error.code !== 'ENOENT') throw error;
        state = { schemaVersion: SCHEMA_VERSION, notes: [] };
        await persist(state);
      }
      return { dataSchemaVersion: SCHEMA_VERSION };
    },
    async onStop() { return { busy: pending > 0 }; },
    tools: {
      async ping(args, context) {
        pending++;
        const operation = queue.then(async () => {
          const next = {
            schemaVersion: SCHEMA_VERSION,
            notes: [...state.notes, { note: args.note, projectId: context.hostBoundProjectId, operationId: context.operationId }].slice(-MAX_NOTES),
          };
          await persist(next);
          state = next;
          return {
            content: [{ type: 'text', text: 'ok' }],
            structuredContent: { ok: true, total: state.notes.filter(row => row.projectId === context.hostBoundProjectId).length, projectId: context.hostBoundProjectId },
          };
        });
        queue = operation.catch(() => {});
        try { return await operation; } finally { pending--; }
      },
      async status(_args, context) {
        await queue;
        const notes = state.notes.filter(row => row.projectId === context.hostBoundProjectId);
        return {
          content: [{ type: 'text', text: JSON.stringify({ total: notes.length }) }],
          structuredContent: { total: notes.length, projectId: context.hostBoundProjectId },
        };
      },
    },
  };
}
