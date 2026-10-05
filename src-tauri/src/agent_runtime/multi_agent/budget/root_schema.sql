-- Root accounting is its own authority, never a fabricated child assignment.
-- Audit rows survive task deletion; they cannot authorize a missing live run.
CREATE TABLE IF NOT EXISTS agent_root_budget_attempts (
 id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL UNIQUE,
 contract_json TEXT NOT NULL CHECK(json_valid(contract_json)),
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS root_budget_attempt_no_update BEFORE UPDATE ON agent_root_budget_attempts
 BEGIN SELECT RAISE(ABORT,'root_budget_attempt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_budget_attempt_no_delete BEFORE DELETE ON agent_root_budget_attempts
 BEGIN SELECT RAISE(ABORT,'root_budget_attempt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_budget_attempt_no_replace BEFORE INSERT ON agent_root_budget_attempts
 WHEN EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE id=NEW.id OR root_run_id=NEW.root_run_id)
 BEGIN SELECT RAISE(ABORT,'root_budget_attempt_immutable'); END;
CREATE TABLE IF NOT EXISTS agent_root_model_journal (
 call_id TEXT NOT NULL,
 root_run_id TEXT NOT NULL,
 lease_attempt_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id),
 round INTEGER NOT NULL CHECK(round>0),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 phase TEXT NOT NULL CHECK(phase IN ('dispatch','received','uncertain','unsent')),
 receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)),
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 PRIMARY KEY(call_id,phase),
 UNIQUE(root_run_id,round,phase)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_root_model_terminal ON agent_root_model_journal(call_id) WHERE phase<>'dispatch';
CREATE TRIGGER IF NOT EXISTS root_model_no_update BEFORE UPDATE ON agent_root_model_journal
 BEGIN SELECT RAISE(ABORT,'root_model_journal_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_model_no_delete BEFORE DELETE ON agent_root_model_journal
 BEGIN SELECT RAISE(ABORT,'root_model_journal_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_model_no_replace BEFORE INSERT ON agent_root_model_journal
 WHEN EXISTS(SELECT 1 FROM agent_root_model_journal WHERE call_id=NEW.call_id AND phase=NEW.phase)
 BEGIN SELECT RAISE(ABORT,'root_model_journal_immutable'); END;
-- SQLite REPLACE also deletes through the round key and partial terminal
-- index. An additive guard protects existing Native rows without rewriting
-- their receipts or relying on recursive_triggers being enabled.
CREATE TRIGGER IF NOT EXISTS root_model_no_replace_unique BEFORE INSERT ON agent_root_model_journal
 WHEN EXISTS(SELECT 1 FROM agent_root_model_journal
   WHERE (root_run_id=NEW.root_run_id AND round=NEW.round AND phase=NEW.phase)
      OR (call_id=NEW.call_id AND phase<>'dispatch' AND NEW.phase<>'dispatch'))
 BEGIN SELECT RAISE(ABORT,'root_model_journal_immutable'); END;
