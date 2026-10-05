-- New human choices only. Do not import/rewrite historical confirmations.
CREATE TABLE IF NOT EXISTS agent_directive_human_reviews (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id TEXT NOT NULL UNIQUE,
    draft_id TEXT NOT NULL REFERENCES agent_directive_drafts(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision>0),
    draft_hash TEXT NOT NULL,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    root_run_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('approve','revise','reject')),
    argument_hash TEXT NOT NULL,
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(draft_id,revision)
);
CREATE INDEX IF NOT EXISTS agent_directive_human_review_scope
ON agent_directive_human_reviews(scan_id,attempt_number,sequence);
CREATE TRIGGER IF NOT EXISTS agent_directive_human_review_immutable
BEFORE UPDATE ON agent_directive_human_reviews
BEGIN SELECT RAISE(ABORT,'directive_human_review_immutable'); END;
CREATE TRIGGER IF NOT EXISTS agent_directive_human_review_no_replace
BEFORE INSERT ON agent_directive_human_reviews
WHEN EXISTS(SELECT 1 FROM agent_directive_human_reviews WHERE draft_id=NEW.draft_id AND revision=NEW.revision)
BEGIN SELECT RAISE(ABORT,'directive_human_review_immutable'); END;

-- All unique identities, including PK sequence, deny REPLACE implicit deletes.
-- Distinct names also protect databases that already installed the first guard.
CREATE TRIGGER IF NOT EXISTS agent_directive_human_review_no_replace_receipt_id
BEFORE INSERT ON agent_directive_human_reviews
WHEN EXISTS(SELECT 1 FROM agent_directive_human_reviews WHERE receipt_id=NEW.receipt_id)
BEGIN SELECT RAISE(ABORT,'directive_human_review_immutable'); END;
CREATE TRIGGER IF NOT EXISTS agent_directive_human_review_no_replace_sequence
BEFORE INSERT ON agent_directive_human_reviews
WHEN EXISTS(SELECT 1 FROM agent_directive_human_reviews WHERE sequence=NEW.sequence)
BEGIN SELECT RAISE(ABORT,'directive_human_review_immutable'); END;
