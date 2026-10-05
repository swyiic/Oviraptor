import type { LogScope } from "./useLiveRunnerLog";

export interface NativeProcessLogRow {
  scanId: string; attempt: number; branch: "source" | "web";
  dispatchClaimId: string; invocationKey: string; executionId: string; stage: string;
  sequence: number; stream: "stdout" | "stderr" | "status" | "gap";
  streamSequence: number; message: string; gap: boolean; time: string;
}
export interface NativeProcessLogPage {
  schemaVersion: 1; scanId: string; attempt: number; requestedAfterSequence: number;
  resetCursor: boolean; rows: NativeProcessLogRow[]; more: boolean; afterSequence: number;
  available: boolean; executionState: string | null; latestSequence: number;
}
export interface NativeReplayState {
  attempt?: number; cursor: number; rows: NativeProcessLogRow[];
  evicted: number; hasGap: boolean; more: boolean;
}
export const emptyNativeReplay = (): NativeReplayState => ({ cursor: 0, rows: [], evicted: 0, hasGap: false, more: false });

const safeInt = (n: unknown): n is number => Number.isSafeInteger(n) && (n as number) >= 0;
const id = (s: unknown): s is string => typeof s === "string" && /^[a-z0-9_:.-]{1,128}$/i.test(s);
const uuid = (s: unknown): s is string => typeof s === "string" && /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(s);
const hash = (s: unknown): s is string => typeof s === "string" && /^[a-f0-9]{64}$/i.test(s);
const streams = new Set(["stdout", "stderr", "status", "gap"]);
function rowIdentity(row: Partial<NativeProcessLogRow>, scope: LogScope): boolean {
  return row.scanId === scope.scanId && safeInt(row.attempt) && row.attempt > 0
    && (scope.attempt === 0 || row.attempt === scope.attempt) && ["source", "web"].includes(row.branch ?? "")
    && uuid(row.dispatchClaimId) && hash(row.invocationKey) && uuid(row.executionId) && id(row.stage)
    && safeInt(row.sequence) && row.sequence > 0 && streams.has(row.stream ?? "")
    && safeInt(row.streamSequence) && row.streamSequence > 0;
}
export function acceptsNativeHint(raw: unknown, scope: LogScope | undefined): boolean {
  return Boolean(scope && raw && typeof raw === "object" && rowIdentity(raw as Partial<NativeProcessLogRow>, scope));
}

// Sequence is global to this journal: gaps from OTHER executions are expected.
// Text is never a deduplication key; repeated messages are legitimate records.
export function acceptNativePage(raw: unknown, scope: LogScope, state: NativeReplayState): NativeReplayState {
  if (!raw || typeof raw !== "object") throw Error("native_log_contract");
  const page = raw as NativeProcessLogPage;
  if (page.schemaVersion !== 1 || page.scanId !== scope.scanId || !safeInt(page.attempt) || page.attempt <= 0
    || (scope.attempt > 0 && page.attempt !== scope.attempt) || page.requestedAfterSequence !== state.cursor
    || typeof page.resetCursor !== "boolean" || typeof page.more !== "boolean" || typeof page.available !== "boolean"
    || !safeInt(page.latestSequence) || !safeInt(page.afterSequence) || !Array.isArray(page.rows) || page.rows.length > 300
    || !(page.executionState === null || ["open", "completed", "failed", "cancelled", "timeout", "gap"].includes(page.executionState))) {
    throw Error("native_log_contract");
  }
  const switched = state.attempt !== undefined && state.attempt !== page.attempt;
  if (page.resetCursor !== switched || (switched && scope.attempt !== 0)) throw Error("native_log_cursor_scope");
  const prior = switched ? emptyNativeReplay() : state;
  let cursor = prior.cursor, hasGap = prior.hasGap;
  const bound = { ...scope, attempt: page.attempt }, rows = [...prior.rows];
  const ordinals = new Map<string, number>();
  const bindings = new Map<string, string>();
  for (const row of rows) {
    ordinals.set(JSON.stringify([row.executionId, row.stream]), row.streamSequence);
    bindings.set(row.executionId, JSON.stringify([row.branch, row.dispatchClaimId, row.invocationKey, row.stage]));
  }
  for (const row of page.rows) {
    if (!rowIdentity(row, bound) || row.sequence <= cursor || row.sequence > page.latestSequence
      || typeof row.message !== "string" || new TextEncoder().encode(row.message).length > 65536
      || typeof row.gap !== "boolean" || typeof row.time !== "string" || row.time.length > 128) throw Error("native_log_row_contract");
    const binding = JSON.stringify([row.branch, row.dispatchClaimId, row.invocationKey, row.stage]);
    if (bindings.has(row.executionId) && bindings.get(row.executionId) !== binding) throw Error("native_log_execution_scope");
    bindings.set(row.executionId, binding);
    const streamKey = JSON.stringify([row.executionId, row.stream]), ordinal = ordinals.get(streamKey);
    if (ordinal !== undefined && row.streamSequence <= ordinal) throw Error("native_log_stream_cursor");
    // Lost observer records are diagnosed from stream ordinals and explicit gap
    // rows; global sequence jumps alone are not invented as durable loss.
    hasGap ||= row.gap || (ordinal !== undefined && row.streamSequence > ordinal + 1);
    ordinals.set(streamKey, row.streamSequence); rows.push(row); cursor = row.sequence;
  }
  if (page.afterSequence !== cursor || (page.more && (!page.rows.length || cursor >= page.latestSequence))
    || (!page.available && page.rows.length > 0)) throw Error("native_log_page_cursor");
  const evicted = Math.max(0, rows.length - 300);
  return { attempt: page.attempt, cursor, rows: rows.slice(-300), evicted: prior.evicted + evicted, hasGap, more: page.more };
}
