-- Additive: preserve every original Native value and physical invoice row.
-- SQLite REPLACE can delete conflicts without firing DELETE triggers when
-- recursive_triggers is OFF. Reject collisions before conflict resolution.
CREATE TRIGGER IF NOT EXISTS budget_clock_no_replace
 BEFORE INSERT ON agent_budget_clock_origins
 WHEN EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=NEW.root_run_id)
 BEGIN SELECT RAISE(ABORT,'budget_clock_immutable'); END;
CREATE TRIGGER IF NOT EXISTS budget_limit_no_replace
 BEFORE INSERT ON agent_budget_limits
 WHEN EXISTS(SELECT 1 FROM agent_budget_limits
   WHERE root_run_id=NEW.root_run_id AND dimension=NEW.dimension)
 BEGIN SELECT RAISE(ABORT,'budget_limit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS budget_entry_no_replace
 BEFORE INSERT ON agent_budget_entries
 WHEN EXISTS(SELECT 1 FROM agent_budget_entries
   WHERE entry_id=NEW.entry_id
      OR (root_run_id=NEW.root_run_id AND idempotency_key=NEW.idempotency_key))
 BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END;
