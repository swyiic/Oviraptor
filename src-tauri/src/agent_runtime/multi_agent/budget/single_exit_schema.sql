-- Original Single financial exit only: no new owner, origin, limit or grant.
CREATE TABLE IF NOT EXISTS agent_single_exit_receipts (
 receipt_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL UNIQUE,
 control_id TEXT NOT NULL UNIQUE,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS single_exit_no_update BEFORE UPDATE ON agent_single_exit_receipts
 BEGIN SELECT RAISE(ABORT,'single_exit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS single_exit_no_delete BEFORE DELETE ON agent_single_exit_receipts
 BEGIN SELECT RAISE(ABORT,'single_exit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS single_exit_no_replace BEFORE INSERT ON agent_single_exit_receipts
 WHEN EXISTS(SELECT 1 FROM agent_single_exit_receipts WHERE receipt_id=NEW.receipt_id
  OR root_run_id=NEW.root_run_id OR control_id=NEW.control_id)
 BEGIN SELECT RAISE(ABORT,'single_exit_immutable'); END;
