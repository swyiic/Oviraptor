-- Financial observations only. This table never issues or restores a grant.
CREATE TABLE IF NOT EXISTS agent_root_elapsed_facts (
 fact_id TEXT PRIMARY KEY NOT NULL,
 root_run_id TEXT NOT NULL UNIQUE,
 owner_kind TEXT NOT NULL CHECK(owner_kind IN ('root_control','worker')),
 owner_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 target_key TEXT NOT NULL,
 coordinator_epoch INTEGER NOT NULL CHECK(coordinator_epoch>0),
 coordinator_fence TEXT NOT NULL,
 origin TEXT NOT NULL,
 cutoff TEXT NOT NULL,
 hard_limit_ms INTEGER NOT NULL CHECK(hard_limit_ms>0),
 elapsed_ms INTEGER NOT NULL CHECK(elapsed_ms>=0),
 journaled_ms INTEGER NOT NULL CHECK(journaled_ms>=0 AND journaled_ms<=elapsed_ms),
 unsettled_ms INTEGER NOT NULL CHECK(unsettled_ms=elapsed_ms-journaled_ms),
 exhausted INTEGER NOT NULL CHECK(exhausted IN (0,1) AND exhausted=(elapsed_ms>=hard_limit_ms)),
 binding_hash TEXT NOT NULL CHECK(length(binding_hash)=64),
 wall_hash TEXT NOT NULL CHECK(length(wall_hash)=64),
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS root_elapsed_fact_no_update BEFORE UPDATE ON agent_root_elapsed_facts
 BEGIN SELECT RAISE(ABORT,'root_elapsed_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_elapsed_fact_no_delete BEFORE DELETE ON agent_root_elapsed_facts
 BEGIN SELECT RAISE(ABORT,'root_elapsed_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_elapsed_fact_no_replace BEFORE INSERT ON agent_root_elapsed_facts
 WHEN EXISTS(SELECT 1 FROM agent_root_elapsed_facts WHERE fact_id=NEW.fact_id OR root_run_id=NEW.root_run_id)
 BEGIN SELECT RAISE(ABORT,'root_elapsed_fact_immutable'); END;
