CREATE TABLE IF NOT EXISTS agent_directive_ordered_actions (
 action_id TEXT PRIMARY KEY,
 directive_id TEXT NOT NULL REFERENCES agent_user_directives(id) ON DELETE CASCADE,
 action_order INTEGER NOT NULL CHECK(action_order IN (1,2)),
 plan_hash TEXT NOT NULL, previous_action_id TEXT NOT NULL DEFAULT '',
 assignment_id TEXT NOT NULL UNIQUE, child_run_id TEXT NOT NULL UNIQUE,
 request_message_id TEXT NOT NULL UNIQUE, input_json TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared','executing','received','completed','failed')),
 response_json TEXT NOT NULL DEFAULT '{}', usage_json TEXT NOT NULL DEFAULT '{}',
 result_message_id TEXT NOT NULL DEFAULT '',
 UNIQUE(directive_id,action_order)
);
CREATE TABLE IF NOT EXISTS agent_directive_ordered_receipts (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 receipt_id TEXT NOT NULL UNIQUE,
 action_id TEXT NOT NULL UNIQUE REFERENCES agent_directive_ordered_actions(action_id) ON DELETE CASCADE,
 directive_id TEXT NOT NULL REFERENCES agent_user_directives(id) ON DELETE CASCADE,
 action_order INTEGER NOT NULL, child_run_id TEXT NOT NULL UNIQUE,
 receipt_json TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 UNIQUE(directive_id,action_order)
);
CREATE TRIGGER IF NOT EXISTS ordered_action_frozen BEFORE UPDATE ON agent_directive_ordered_actions
 WHEN OLD.action_id<>NEW.action_id OR OLD.directive_id<>NEW.directive_id OR OLD.action_order<>NEW.action_order
 OR OLD.plan_hash<>NEW.plan_hash OR OLD.previous_action_id<>NEW.previous_action_id
 OR OLD.assignment_id<>NEW.assignment_id OR OLD.child_run_id<>NEW.child_run_id
 OR OLD.request_message_id<>NEW.request_message_id OR OLD.input_json<>NEW.input_json
 BEGIN SELECT RAISE(ABORT,'ordered_action_frozen'); END;
CREATE TRIGGER IF NOT EXISTS ordered_action_response_frozen BEFORE UPDATE OF response_json,usage_json ON agent_directive_ordered_actions
 WHEN OLD.state<>'executing' OR NEW.state<>'received'
 BEGIN SELECT RAISE(ABORT,'ordered_action_response_frozen'); END;
CREATE TRIGGER IF NOT EXISTS ordered_action_phase BEFORE UPDATE OF state ON agent_directive_ordered_actions
 WHEN OLD.state<>NEW.state AND NOT ((OLD.state='prepared' AND NEW.state='executing')
 OR (OLD.state='executing' AND NEW.state='received') OR (OLD.state='received' AND NEW.state IN ('completed','failed')))
 BEGIN SELECT RAISE(ABORT,'ordered_action_phase'); END;
CREATE TRIGGER IF NOT EXISTS ordered_action_no_replace BEFORE INSERT ON agent_directive_ordered_actions
 WHEN EXISTS(SELECT 1 FROM agent_directive_ordered_actions WHERE action_id=NEW.action_id
 OR (directive_id=NEW.directive_id AND action_order=NEW.action_order)
 OR assignment_id=NEW.assignment_id OR child_run_id=NEW.child_run_id OR request_message_id=NEW.request_message_id)
 BEGIN SELECT RAISE(ABORT,'ordered_action_no_replace'); END;
CREATE TRIGGER IF NOT EXISTS ordered_action_no_delete BEFORE DELETE ON agent_directive_ordered_actions
 WHEN EXISTS(SELECT 1 FROM agent_user_directives WHERE id=OLD.directive_id)
 BEGIN SELECT RAISE(ABORT,'ordered_action_no_delete'); END;
CREATE TRIGGER IF NOT EXISTS ordered_receipt_frozen BEFORE UPDATE ON agent_directive_ordered_receipts
 BEGIN SELECT RAISE(ABORT,'ordered_receipt_frozen'); END;
CREATE TRIGGER IF NOT EXISTS ordered_receipt_no_replace BEFORE INSERT ON agent_directive_ordered_receipts
 WHEN EXISTS(SELECT 1 FROM agent_directive_ordered_receipts WHERE sequence=NEW.sequence OR receipt_id=NEW.receipt_id
 OR action_id=NEW.action_id OR child_run_id=NEW.child_run_id
 OR (directive_id=NEW.directive_id AND action_order=NEW.action_order))
 BEGIN SELECT RAISE(ABORT,'ordered_receipt_no_replace'); END;
CREATE TRIGGER IF NOT EXISTS ordered_receipt_no_delete BEFORE DELETE ON agent_directive_ordered_receipts
 WHEN EXISTS(SELECT 1 FROM agent_user_directives WHERE id=OLD.directive_id)
 BEGIN SELECT RAISE(ABORT,'ordered_receipt_no_delete'); END;
CREATE TRIGGER IF NOT EXISTS ordered_action_collaboration_insert AFTER INSERT ON agent_directive_ordered_actions
 BEGIN INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
 SELECT scan_id,attempt_number,'user_directive',id,'user_directive',
 json_object('orderedActionId',NEW.action_id,'orderedOrder',NEW.action_order,'orderedState',NEW.state)
 FROM agent_user_directives WHERE id=NEW.directive_id; END;
CREATE TRIGGER IF NOT EXISTS ordered_action_collaboration_update AFTER UPDATE OF state ON agent_directive_ordered_actions
 WHEN OLD.state<>NEW.state
 BEGIN INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
 SELECT scan_id,attempt_number,'user_directive',id,'user_directive',
 json_object('orderedActionId',NEW.action_id,'orderedOrder',NEW.action_order,'orderedState',NEW.state)
 FROM agent_user_directives WHERE id=NEW.directive_id; END;
CREATE TRIGGER IF NOT EXISTS ordered_receipt_collaboration_insert AFTER INSERT ON agent_directive_ordered_receipts
 BEGIN INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
 SELECT scan_id,attempt_number,'user_directive',id,'user_directive',
 json_object('orderedActionId',NEW.action_id,'orderedOrder',NEW.action_order,'orderedReceiptId',NEW.receipt_id)
 FROM agent_user_directives WHERE id=NEW.directive_id; END;
