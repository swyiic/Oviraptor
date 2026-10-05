// Consume the audited original failure, never a caller/model completion label.
// The same dedicated pause transaction owns Root, branch, target and scan.
fn source_pause_original_target_in(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<String, String> {
    let path: String = db
        .query_row(
            "SELECT p.source_path FROM source_scope_contracts p JOIN sentinel_scans s
        ON s.id=p.scan_id AND s.attempt_count=p.attempt_number JOIN agent_runs r ON r.id=?3
        WHERE p.scan_id=?1 AND p.attempt_number=?2 AND s.source_path=p.source_path
        AND p.source_path=json_extract(r.plan_json,'$.runtime.scope.sourcePath')
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![actor.scan_id, actor.attempt_number, actor.root_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_pause_original_target_scope_changed")?;
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM sentinel_targets WHERE scan_id=?1 AND url=?2
        AND NOT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND url=?2
            AND (last_attempt_number<0 OR last_attempt_number>?3))",
            params![actor.scan_id, path, actor.attempt_number],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if path.is_empty() || count != 1 {
        return Err("source_pause_original_target_missing_or_ambiguous".into());
    }
    Ok(path)
}

fn known_source_pause_branch_outcome_in(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(String, String), String> {
    // This immutable proof is rechecked by the consumer before and after its
    // projection. A paused scan or an error string alone is never sufficient.
    audit_recovered_source_exhausted_root(db, actor)?;
    let report = json!({"error":"source_tool_phase_round_budget_exhausted_without_finish"});
    let (status, reason) = source_branch_original_outcome(
        db,
        &actor.scan_id,
        actor.attempt_number,
        &report,
        std::slice::from_ref(&actor.root_run_id),
    )?;
    if status != "partial" {
        return Err("source_pause_original_branch_outcome_changed".into());
    }
    Ok((reason, report.to_string()))
}

fn verify_known_source_pause_branch_in(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    let path = source_pause_original_target_in(db, actor)?;
    let (reason, report) = known_source_pause_branch_outcome_in(db, actor)?;
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1
        AND attempt_number=?2 AND branch='source' AND status='partial' AND checkpoint=?3 AND report_json=?4)
        AND EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND url=?5 AND status='partial' AND last_attempt_number=?2)",
        params![actor.scan_id,actor.attempt_number,reason,report,path],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("source_pause_original_branch_projection_unconfirmed".into());
    }
    Ok(())
}

fn publish_known_source_pause_branch_in(
    tx: &rusqlite::Transaction<'_>,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    let path = source_pause_original_target_in(tx, actor)?;
    let (reason, report) = known_source_pause_branch_outcome_in(tx, actor)?;
    let siblings =
        workbench_admission_siblings(tx, &actor.scan_id, actor.attempt_number, "source")?;
    let changed=tx.execute("UPDATE native_scan_branches SET status='partial',checkpoint=?3,report_json=?4,
        updated_at=datetime('now','localtime') WHERE scan_id=?1 AND attempt_number=?2 AND branch='source' AND status='pending'",
        params![actor.scan_id,actor.attempt_number,reason,report]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("source_pause_original_branch_projection_unconfirmed".into());
    }
    let changed = tx
        .execute(
            "UPDATE sentinel_targets SET status='partial',last_attempt_number=?3
        WHERE scan_id=?1 AND url=?2",
            params![actor.scan_id, path, actor.attempt_number],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("source_pause_original_branch_projection_unconfirmed".into());
    }
    verify_known_source_pause_branch_in(tx, actor)?;
    if workbench_admission_siblings(tx, &actor.scan_id, actor.attempt_number, "source")? != siblings
    {
        return Err("source_pause_original_sibling_changed".into());
    }
    Ok(())
}
