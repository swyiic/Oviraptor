-- Only the new owned human dispatch; no legacy row/value migration.
-- REPLACE bypasses DELETE triggers when recursive_triggers=OFF. Check the
-- existing original PK before conflict resolution, even if NEW removes proof.
CREATE TRIGGER IF NOT EXISTS human_owned_specialist_no_replace
BEFORE INSERT ON agent_specialist_calls
WHEN EXISTS(SELECT 1 FROM agent_specialist_calls c WHERE c.child_run_id=NEW.child_run_id
  AND json_type(c.request_json,'$.humanDirectiveDispatch') IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'human_owned_dispatch_immutable'); END;
CREATE TRIGGER IF NOT EXISTS human_owned_specialist_no_delete
BEFORE DELETE ON agent_specialist_calls
WHEN json_type(OLD.request_json,'$.humanDirectiveDispatch') IS NOT NULL
 AND EXISTS(SELECT 1 FROM agent_runs WHERE id=OLD.root_run_id)
BEGIN SELECT RAISE(ABORT,'human_owned_dispatch_immutable'); END;
