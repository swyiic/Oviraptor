-- Agent stage-adjudication timeline (§9.3 of AUTONOMOUS_OSS_SOURCE_AUDIT_DESIGN).
--
-- Append-only record of every time the agent adjudicated a pipeline stage: what
-- the stage produced (`outcome`), what the agent decided to do next
-- (`decision`), which gate tier that fell under (`gate_tier`, §9.0), and why
-- (`rationale`). This is the DATA + SURFACING half of §9. It carries NO
-- authority: a row here never continues, retries or unblocks a scan — the
-- reducer and the sealed budget contracts remain the only things that move
-- control flow. That is what makes this table safe to ship ahead of the §9.1
-- adjudicator.
--
-- Purely additive. It creates one new table and one index and touches no
-- existing table, so an existing database gains it on next open with no data
-- rewrite. `ON DELETE CASCADE` keeps it tidy when a scan is deleted.
CREATE TABLE IF NOT EXISTS agent_stage_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL DEFAULT 0,
    stage TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT 'ok'
        CHECK(outcome IN ('ok','degraded','blocked','gap')),
    decision TEXT NOT NULL DEFAULT 'continue'
        CHECK(decision IN ('continue','retry_adjusted','degrade','skip','needs_user')),
    gate_tier INTEGER NOT NULL DEFAULT 1
        CHECK(gate_tier IN (1,2,3)),
    rationale TEXT NOT NULL DEFAULT '',
    detail_json TEXT NOT NULL DEFAULT '{}'
        CHECK(json_valid(detail_json)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_agent_stage_decisions_scan
    ON agent_stage_decisions(scan_id, attempt_number, id);
