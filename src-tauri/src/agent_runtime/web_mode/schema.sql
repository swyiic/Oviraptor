-- New ordinary Web mode declarations only; old Native/Source JSON unchanged.
CREATE TABLE IF NOT EXISTS native_web_mode_drafts (
 scan_id TEXT PRIMARY KEY,
 draft_id TEXT NOT NULL UNIQUE,
 mode TEXT NOT NULL CHECK(mode IN ('single','multi')),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS native_web_mode_receipts (
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 receipt_id TEXT NOT NULL UNIQUE,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 PRIMARY KEY(scan_id,attempt_number)
);
CREATE TABLE IF NOT EXISTS agent_root_mode_definitions (
 root_run_id TEXT PRIMARY KEY,
 mode_id TEXT NOT NULL UNIQUE,
 definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
 native_plan_text_hash TEXT NOT NULL CHECK(length(native_plan_text_hash)=64),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS web_mode_draft_immutable_update BEFORE UPDATE ON native_web_mode_drafts BEGIN SELECT RAISE(ABORT,'web_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_draft_immutable_delete BEFORE DELETE ON native_web_mode_drafts BEGIN SELECT RAISE(ABORT,'web_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_draft_immutable_replace BEFORE INSERT ON native_web_mode_drafts
 WHEN EXISTS(SELECT 1 FROM native_web_mode_drafts WHERE scan_id=NEW.scan_id OR draft_id=NEW.draft_id) BEGIN SELECT RAISE(ABORT,'web_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_receipt_immutable_update BEFORE UPDATE ON native_web_mode_receipts BEGIN SELECT RAISE(ABORT,'web_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_receipt_immutable_delete BEFORE DELETE ON native_web_mode_receipts BEGIN SELECT RAISE(ABORT,'web_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_receipt_immutable_replace BEFORE INSERT ON native_web_mode_receipts
 WHEN EXISTS(SELECT 1 FROM native_web_mode_receipts WHERE receipt_id=NEW.receipt_id OR (scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)) BEGIN SELECT RAISE(ABORT,'web_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_mode_immutable_update BEFORE UPDATE ON agent_root_mode_definitions BEGIN SELECT RAISE(ABORT,'root_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_mode_immutable_delete BEFORE DELETE ON agent_root_mode_definitions BEGIN SELECT RAISE(ABORT,'root_mode_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_mode_immutable_replace BEFORE INSERT ON agent_root_mode_definitions
 WHEN EXISTS(SELECT 1 FROM agent_root_mode_definitions WHERE root_run_id=NEW.root_run_id OR mode_id=NEW.mode_id) BEGIN SELECT RAISE(ABORT,'root_mode_immutable'); END;
CREATE TABLE IF NOT EXISTS native_web_mode_actions (
 scan_id TEXT PRIMARY KEY,draft_id TEXT NOT NULL UNIQUE,
 action_json TEXT NOT NULL CHECK(json_valid(action_json)),action_hash TEXT NOT NULL CHECK(length(action_hash)=64),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS web_mode_action_no_update BEFORE UPDATE ON native_web_mode_actions BEGIN SELECT RAISE(ABORT,'web_mode_action_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_action_no_delete BEFORE DELETE ON native_web_mode_actions BEGIN SELECT RAISE(ABORT,'web_mode_action_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_mode_action_no_replace BEFORE INSERT ON native_web_mode_actions
 WHEN EXISTS(SELECT 1 FROM native_web_mode_actions WHERE scan_id=NEW.scan_id OR draft_id=NEW.draft_id) BEGIN SELECT RAISE(ABORT,'web_mode_action_immutable'); END;
