-- New Native diagnostic data only; no import, deletion or rewrite of old logs.
CREATE TABLE IF NOT EXISTS native_process_log_executions (
    execution_id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    branch TEXT NOT NULL CHECK(branch IN ('source','web')),
    dispatch_claim_id TEXT NOT NULL,
    invocation_key TEXT NOT NULL,
    stage TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'open',
    row_count INTEGER NOT NULL DEFAULT 0,
    byte_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS native_process_log_rows (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    execution_id TEXT NOT NULL REFERENCES native_process_log_executions(execution_id),
    stream TEXT NOT NULL CHECK(stream IN ('stdout','stderr','status','gap')),
    stream_sequence INTEGER NOT NULL CHECK(stream_sequence>0),
    message TEXT NOT NULL,
    gap INTEGER NOT NULL DEFAULT 0 CHECK(gap IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(execution_id,stream,stream_sequence)
);
CREATE INDEX IF NOT EXISTS idx_native_process_log_scope
ON native_process_log_executions(scan_id,attempt_number,execution_id);
CREATE INDEX IF NOT EXISTS idx_native_process_log_replay
ON native_process_log_rows(execution_id,sequence);
CREATE TRIGGER IF NOT EXISTS native_process_log_binding_immutable
BEFORE UPDATE OF execution_id,scan_id,attempt_number,branch,dispatch_claim_id,invocation_key,stage ON native_process_log_executions
BEGIN SELECT RAISE(ABORT,'native_process_log_binding_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_process_log_row_immutable
BEFORE UPDATE ON native_process_log_rows
BEGIN SELECT RAISE(ABORT,'native_process_log_row_immutable'); END;

-- Reject REPLACE's implicit delete even with recursive_triggers=OFF.
CREATE TRIGGER IF NOT EXISTS native_process_log_execution_no_replace
BEFORE INSERT ON native_process_log_executions
WHEN EXISTS(SELECT 1 FROM native_process_log_executions WHERE execution_id=NEW.execution_id)
BEGIN SELECT RAISE(ABORT,'native_process_log_execution_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_process_log_row_no_replace
BEFORE INSERT ON native_process_log_rows
WHEN EXISTS(SELECT 1 FROM native_process_log_rows WHERE sequence=NEW.sequence
 OR (execution_id=NEW.execution_id AND stream=NEW.stream AND stream_sequence=NEW.stream_sequence))
BEGIN SELECT RAISE(ABORT,'native_process_log_row_immutable'); END;
