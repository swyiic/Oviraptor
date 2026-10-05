-- Per-call facts survive process interruption and settled task deletion.
-- Hashes and billing metadata only: no prompts, credentials or response text.
CREATE TABLE IF NOT EXISTS agent_web_model_journal (
 call_id TEXT NOT NULL,
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 lease_epoch INTEGER NOT NULL,
 fencing_token TEXT NOT NULL,
 round INTEGER NOT NULL CHECK(round>0),
 request_hash TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('dispatch','received','uncertain','unsent')),
 receipt_json TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 PRIMARY KEY(call_id,phase),
 UNIQUE(child_run_id,round,phase)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_web_model_terminal
 ON agent_web_model_journal(call_id) WHERE phase<>'dispatch';
CREATE INDEX IF NOT EXISTS idx_web_model_root ON agent_web_model_journal(root_run_id,phase);
CREATE TRIGGER IF NOT EXISTS web_model_journal_no_update BEFORE UPDATE ON agent_web_model_journal
 BEGIN SELECT RAISE(ABORT,'web_model_journal_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_model_journal_no_delete BEFORE DELETE ON agent_web_model_journal
 BEGIN SELECT RAISE(ABORT,'web_model_journal_immutable'); END;
