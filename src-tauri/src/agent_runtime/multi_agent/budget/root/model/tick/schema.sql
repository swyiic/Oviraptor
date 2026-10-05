-- Native Root decision receipts. No legacy response/history adoption.
-- New schema: immutable financial identity survives deletion of live runs.
-- CREATE IF NOT EXISTS does not migrate any existing receipt table.
CREATE TABLE IF NOT EXISTS agent_root_tick_receipts (
 call_id TEXT NOT NULL,
 root_run_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(root_run_id) ON DELETE RESTRICT,
 lease_attempt_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id) ON DELETE RESTRICT,
 round INTEGER NOT NULL CHECK(round>0),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 basis_hash TEXT NOT NULL CHECK(length(basis_hash)=64),
 phase TEXT NOT NULL CHECK(phase IN ('request','decision','publication')),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 PRIMARY KEY(call_id,phase),
 UNIQUE(root_run_id,round,phase),
 UNIQUE(root_run_id,basis_hash,phase)
);
CREATE TRIGGER IF NOT EXISTS root_tick_no_update BEFORE UPDATE ON agent_root_tick_receipts
 BEGIN SELECT RAISE(ABORT,'root_tick_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_tick_no_delete BEFORE DELETE ON agent_root_tick_receipts
 BEGIN SELECT RAISE(ABORT,'root_tick_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_tick_no_replace BEFORE INSERT ON agent_root_tick_receipts
 WHEN EXISTS(SELECT 1 FROM agent_root_tick_receipts r WHERE
  (r.call_id=NEW.call_id AND r.phase=NEW.phase) OR
  (r.root_run_id=NEW.root_run_id AND r.round=NEW.round AND r.phase=NEW.phase) OR
  (r.root_run_id=NEW.root_run_id AND r.basis_hash=NEW.basis_hash AND r.phase=NEW.phase))
 BEGIN SELECT RAISE(ABORT,'root_tick_receipt_immutable'); END;

-- Independent chat cursor: original model sequence is never the timeline cursor.
-- Retain original publication JSON bytes. No history adoption or read-side repair.
CREATE TABLE IF NOT EXISTS agent_root_tick_timeline_receipts (
 call_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(root_run_id) ON DELETE RESTRICT,
 lease_attempt_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id) ON DELETE RESTRICT,
 model_event_sequence INTEGER NOT NULL CHECK(model_event_sequence>0),
 collaboration_sequence INTEGER NOT NULL UNIQUE CHECK(collaboration_sequence>0),
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 entity_id TEXT NOT NULL UNIQUE,
 payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
 event_created_at TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 UNIQUE(root_run_id,model_event_sequence)
);
CREATE TRIGGER IF NOT EXISTS root_tick_timeline_no_update
 BEFORE UPDATE ON agent_root_tick_timeline_receipts
 BEGIN SELECT RAISE(ABORT,'root_tick_timeline_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_tick_timeline_no_delete
 BEFORE DELETE ON agent_root_tick_timeline_receipts
 BEGIN SELECT RAISE(ABORT,'root_tick_timeline_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_tick_timeline_no_replace
 BEFORE INSERT ON agent_root_tick_timeline_receipts
 WHEN EXISTS(SELECT 1 FROM agent_root_tick_timeline_receipts r WHERE
  r.call_id=NEW.call_id OR r.collaboration_sequence=NEW.collaboration_sequence
  OR r.entity_id=NEW.entity_id
  OR (r.root_run_id=NEW.root_run_id AND r.model_event_sequence=NEW.model_event_sequence))
 BEGIN SELECT RAISE(ABORT,'root_tick_timeline_receipt_immutable'); END;

-- Independent original terminal physical proof; never backfill old receipts.
CREATE TABLE IF NOT EXISTS agent_root_tick_terminal_proofs (
 call_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(root_run_id) ON DELETE RESTRICT,
 lease_attempt_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id) ON DELETE RESTRICT,
 round INTEGER NOT NULL CHECK(round>0),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 basis_hash TEXT NOT NULL CHECK(length(basis_hash)=64),
 phase TEXT NOT NULL CHECK(phase IN ('received','uncertain','unsent')),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 UNIQUE(root_run_id,round), UNIQUE(root_run_id,basis_hash)
);
CREATE TRIGGER IF NOT EXISTS root_tick_terminal_proof_no_update BEFORE UPDATE ON agent_root_tick_terminal_proofs
 BEGIN SELECT RAISE(ABORT,'root_tick_terminal_proof_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_tick_terminal_proof_no_delete BEFORE DELETE ON agent_root_tick_terminal_proofs
 BEGIN SELECT RAISE(ABORT,'root_tick_terminal_proof_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_tick_terminal_proof_no_replace BEFORE INSERT ON agent_root_tick_terminal_proofs
 WHEN EXISTS(SELECT 1 FROM agent_root_tick_terminal_proofs p WHERE p.call_id=NEW.call_id OR (p.root_run_id=NEW.root_run_id AND (p.round=NEW.round OR p.basis_hash=NEW.basis_hash)))
 BEGIN SELECT RAISE(ABORT,'root_tick_terminal_proof_immutable'); END;
