-- Only a new explicit declaration changes budget semantics; no data backfill.
CREATE TABLE IF NOT EXISTS agent_root_budget_definitions (
 root_run_id TEXT PRIMARY KEY NOT NULL,
 declaration_id TEXT NOT NULL UNIQUE,
 definition_json TEXT NOT NULL,
 native_plan_text_hash TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS root_budget_definition_no_update
 BEFORE UPDATE ON agent_root_budget_definitions
 BEGIN SELECT RAISE(ABORT,'root_budget_definition_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_budget_definition_no_delete
 BEFORE DELETE ON agent_root_budget_definitions
 BEGIN SELECT RAISE(ABORT,'root_budget_definition_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_budget_definition_no_replace
 BEFORE INSERT ON agent_root_budget_definitions
 WHEN EXISTS(SELECT 1 FROM agent_root_budget_definitions
   WHERE root_run_id=NEW.root_run_id OR declaration_id=NEW.declaration_id)
 BEGIN SELECT RAISE(ABORT,'root_budget_definition_immutable'); END;
