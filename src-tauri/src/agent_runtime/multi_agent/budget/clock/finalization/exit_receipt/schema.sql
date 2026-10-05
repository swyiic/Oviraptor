-- Original Multi financial exit. No run FK, grant, resume or old backfill.
CREATE TABLE IF NOT EXISTS agent_multi_exit_receipts (
 receipt_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL UNIQUE,
 control_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id) ON DELETE RESTRICT,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%d %H:%M:%f','now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS multi_exit_no_update BEFORE UPDATE ON agent_multi_exit_receipts
 BEGIN SELECT RAISE(ABORT,'multi_exit_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS multi_exit_no_delete BEFORE DELETE ON agent_multi_exit_receipts
 BEGIN SELECT RAISE(ABORT,'multi_exit_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS multi_exit_no_replace BEFORE INSERT ON agent_multi_exit_receipts
 WHEN EXISTS(SELECT 1 FROM agent_multi_exit_receipts r
  WHERE r.receipt_id=NEW.receipt_id OR r.root_run_id=NEW.root_run_id)
 BEGIN SELECT RAISE(ABORT,'multi_exit_receipt_immutable'); END;
