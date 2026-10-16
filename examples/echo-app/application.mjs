import { mkdir, readFile, writeFile, rename, rm, lstat } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

const SCHEMA_VERSION = 2;
const MAX_RECORDS = 128;
const MAX_STATE_BYTES = 512 * 1024;
const own = (value, key) => Object.hasOwn(value, key);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
function problem(message) { throw new Error(message); }
function closedKeys(value, allowed) { return object(value) && Object.keys(value).every(key => allowed.includes(key)); }
function recordValid(record) {
  return closedKeys(record, ['projectId', 'operationId', 'text', 'sequence'])
    && Number.isSafeInteger(record.projectId) && record.projectId > 0
    && typeof record.operationId === 'string' && record.operationId.length > 0 && record.operationId.length <= 128
    && typeof record.text === 'string' && [...record.text].length <= 2048
    && Number.isSafeInteger(record.sequence) && record.sequence > 0;
}
function validateCurrent(value) {
  if (!closedKeys(value, ['schemaVersion', 'records']) || value.schemaVersion !== SCHEMA_VERSION
    || !Array.isArray(value.records) || value.records.length > MAX_RECORDS || !value.records.every(recordValid)) problem('echo_data_invalid');
  const operations = value.records.map(row => `${row.projectId}:${row.operationId}`);
  if (new Set(operations).size !== operations.length || value.records.some((row, index) => row.sequence !== index + 1)) problem('echo_data_invalid');
  return value;
}
function migrate(value) {
  if (value?.schemaVersion === SCHEMA_VERSION) return { state: validateCurrent(value), changed: false };
  // Version 1 was an app-owned flat echo list. Preserve every operation and its project.
  if (!closedKeys(value, ['schemaVersion', 'echoes']) || value.schemaVersion !== 1 || !Array.isArray(value.echoes)
    || value.echoes.length > MAX_RECORDS) problem('echo_data_version_unsupported');
  const state = { schemaVersion: SCHEMA_VERSION, records: value.echoes.map((row, index) => ({ ...row, sequence: index + 1 })) };
  return { state: validateCurrent(state), changed: true };
}
export function createEchoApplication(manifest) {
  let dataFile;
  let state;
  let queue = Promise.resolve();
  let pending = 0;
  async function persist(next) {
    const bytes = `${JSON.stringify(next)}\n`;
    if (Buffer.byteLength(bytes) > MAX_STATE_BYTES) problem('echo_data_limit');
    const temporary = `${dataFile}.${randomUUID()}.tmp`;
    try {
      await writeFile(temporary, bytes, { flag: 'wx', mode: 0o600 });
      await rename(temporary, dataFile);
    } finally { await rm(temporary, { force: true }); }
  }
  return {
    manifest,
    async onStart({ dataDir }) {
      await mkdir(dataDir, { recursive: true, mode: 0o700 });
      if ((await lstat(dataDir)).isSymbolicLink()) problem('echo_data_invalid');
      dataFile = join(dataDir, 'echo-state.json');
      let raw;
      try {
        const info = await lstat(dataFile);
        if (!info.isFile() || info.isSymbolicLink() || info.size > MAX_STATE_BYTES) problem('echo_data_invalid');
        raw = await readFile(dataFile, 'utf8');
      } catch (error) { if (error.code !== 'ENOENT') throw error; }
      if (raw === undefined) {
        state = { schemaVersion: SCHEMA_VERSION, records: [] };
        await persist(state);
      } else {
        const migrated = migrate(JSON.parse(raw));
        state = migrated.state;
        if (migrated.changed) await persist(state);
      }
      return { dataSchemaVersion: SCHEMA_VERSION };
    },
    async onStop() { return { busy: pending > 0 }; },
    tools: {
      async echo(args, context) {
        pending++;
        const operation = queue.then(async () => {
          const existing = state.records.find(row => row.projectId === context.hostBoundProjectId && row.operationId === context.operationId);
          if (existing) {
            if (existing.text !== args.text) problem('echo_operation_conflict');
            return { content: [{ type: 'text', text: existing.text }], structuredContent: { ...existing, replayed: true } };
          }
          if (state.records.length >= MAX_RECORDS) problem('echo_data_limit');
          const row = { projectId: context.hostBoundProjectId, operationId: context.operationId, text: args.text, sequence: state.records.length + 1 };
          const next = { schemaVersion: SCHEMA_VERSION, records: [...state.records, row] };
          await persist(next);
          state = next;
          return { content: [{ type: 'text', text: row.text }], structuredContent: { ...row, replayed: false } };
        });
        queue = operation.catch(() => {});
        try { return await operation; } finally { pending--; }
      },
      async history(_args, context) {
        await queue;
        const rows = state.records.filter(row => row.projectId === context.hostBoundProjectId);
        // Keep the transport bounded even with all 128 maximum-length records.
        const tail = rows.slice(-16);
        return { content: [{ type: 'text', text: `${rows.length} saved echo operation(s).` }],
          structuredContent: { records: tail, total: rows.length, truncated: tail.length < rows.length } };
      },
    },
  };
}
