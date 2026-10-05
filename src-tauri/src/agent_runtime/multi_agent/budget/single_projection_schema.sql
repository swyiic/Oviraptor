-- Audit only. An exit projection never grants resume, a new C, or a new budget.
CREATE TABLE IF NOT EXISTS agent_single_projection_receipts (
 receipt_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL UNIQUE,
 control_id TEXT NOT NULL UNIQUE,
 financial_receipt_id TEXT NOT NULL UNIQUE,
 proof_json TEXT NOT NULL CHECK(json_valid(proof_json)),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS single_projection_no_update BEFORE UPDATE ON agent_single_projection_receipts
 BEGIN SELECT RAISE(ABORT,'single_projection_immutable'); END;
CREATE TRIGGER IF NOT EXISTS single_projection_no_delete BEFORE DELETE ON agent_single_projection_receipts
 BEGIN SELECT RAISE(ABORT,'single_projection_immutable'); END;
CREATE TRIGGER IF NOT EXISTS single_projection_no_replace BEFORE INSERT ON agent_single_projection_receipts
 WHEN EXISTS(SELECT 1 FROM agent_single_projection_receipts WHERE receipt_id=NEW.receipt_id
 OR root_run_id=NEW.root_run_id OR control_id=NEW.control_id OR financial_receipt_id=NEW.financial_receipt_id)
 BEGIN SELECT RAISE(ABORT,'single_projection_immutable'); END;
