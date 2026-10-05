//! New worker identities only. Existing runs are never backfilled with grants.
//! Audit rows survive explicit task deletion but cannot authorize a missing run.
pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_assignment_attempts (
 id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL UNIQUE,
 coordinator_epoch INTEGER NOT NULL CHECK(coordinator_epoch>0),
 coordinator_fencing_token TEXT NOT NULL,
 lease_epoch INTEGER NOT NULL CHECK(lease_epoch>0),
 fencing_token TEXT NOT NULL UNIQUE,
 worker_id TEXT NOT NULL UNIQUE,
 state TEXT NOT NULL CHECK(state IN ('leased','running','paused','completed','failed','cancelled','expired')),
 leased_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 heartbeat_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 expires_at TEXT NOT NULL,
 finished_at TEXT NOT NULL DEFAULT '',
 failure_class TEXT NOT NULL DEFAULT '',
 UNIQUE(assignment_id,lease_epoch)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_assignment_one_open_attempt
 ON agent_assignment_attempts(assignment_id) WHERE state IN ('leased','running','paused');
CREATE TRIGGER IF NOT EXISTS assignment_attempt_no_delete BEFORE DELETE ON agent_assignment_attempts
 BEGIN SELECT RAISE(ABORT,'assignment_attempt_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS assignment_attempt_identity_immutable BEFORE UPDATE ON agent_assignment_attempts
 WHEN NEW.id IS NOT OLD.id OR NEW.root_run_id IS NOT OLD.root_run_id
 OR NEW.assignment_id IS NOT OLD.assignment_id OR NEW.child_run_id IS NOT OLD.child_run_id
 OR NEW.coordinator_epoch IS NOT OLD.coordinator_epoch OR NEW.coordinator_fencing_token IS NOT OLD.coordinator_fencing_token
 OR NEW.lease_epoch IS NOT OLD.lease_epoch OR NEW.fencing_token IS NOT OLD.fencing_token
 OR NEW.worker_id IS NOT OLD.worker_id OR NEW.leased_at IS NOT OLD.leased_at
 BEGIN SELECT RAISE(ABORT,'assignment_attempt_identity_immutable'); END;
CREATE TRIGGER IF NOT EXISTS assignment_attempt_transition BEFORE UPDATE OF state ON agent_assignment_attempts
 WHEN NEW.state<>OLD.state AND NOT (
 (OLD.state='leased' AND NEW.state IN ('running','paused','failed','cancelled','expired')) OR
 (OLD.state='running' AND NEW.state IN ('paused','completed','failed','cancelled','expired')) OR
 (OLD.state='paused' AND NEW.state IN ('completed','failed','cancelled','expired')))
 BEGIN SELECT RAISE(ABORT,'assignment_attempt_transition_denied'); END;
CREATE TABLE IF NOT EXISTS agent_assignment_replacements (
 original_attempt_id TEXT PRIMARY KEY,
 replacement_attempt_id TEXT NOT NULL UNIQUE,
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 coordinator_epoch INTEGER NOT NULL CHECK(coordinator_epoch>0),
 coordinator_fencing_token TEXT NOT NULL,
 original_hash TEXT NOT NULL,
 assignment_hash TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS assignment_replacement_no_update BEFORE UPDATE ON agent_assignment_replacements
 BEGIN SELECT RAISE(ABORT,'assignment_replacement_immutable'); END;
CREATE TRIGGER IF NOT EXISTS assignment_replacement_no_delete BEFORE DELETE ON agent_assignment_replacements
 BEGIN SELECT RAISE(ABORT,'assignment_replacement_immutable'); END;
CREATE TRIGGER IF NOT EXISTS assignment_replacement_no_replace BEFORE INSERT ON agent_assignment_replacements
 WHEN EXISTS(SELECT 1 FROM agent_assignment_replacements
   WHERE original_attempt_id=NEW.original_attempt_id OR replacement_attempt_id=NEW.replacement_attempt_id)
 BEGIN SELECT RAISE(ABORT,'assignment_replacement_immutable'); END;
"#;
