-- BEFORE INSERT protects SQLite REPLACE even when recursive_triggers is OFF.
-- Other collaboration rows and existing task deletion semantics are untouched;
-- Root paid/publication receipts already independently forbid their removal.
CREATE TRIGGER IF NOT EXISTS root_tick_timeline_event_no_replace
 BEFORE INSERT ON agent_collaboration_events
 WHEN EXISTS(SELECT 1 FROM agent_root_tick_timeline_receipts r WHERE
  r.collaboration_sequence=NEW.sequence
  OR ((NEW.event_type='root_decision' OR NEW.entity_type='root_decision')
    AND r.entity_id=NEW.entity_id))
 BEGIN SELECT RAISE(ABORT,'root_tick_timeline_event_immutable'); END;
