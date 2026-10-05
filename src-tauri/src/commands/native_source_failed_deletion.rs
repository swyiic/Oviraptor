// Read-only proof of one exact paid failure. Never reclassify it as completed,
// execute a child, renew C, synthesize a review or release unknown obligations.
fn verify_exhausted_source_for_deletion(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    plan: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::source;
    if db.is_autocommit() {
        return Err("deleted_audit_snapshot_required".into());
    }
    let outcome =
        AgentTargetOutcome::incomplete("源码多智能体执行未完成，子任务回执和未决预算保留待核对");
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status='terminal'
        AND terminal_state=?2 AND terminal_code=?3 AND terminal_reason=?4 AND cancel_requested_at='' AND finished_at<>'')",
        params![actor.root_run_id,outcome.terminal_status(),outcome.terminal_code(),
            crate::agent_runtime::secrets::redact_text_with(&outcome.detail(),None)],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        return Err("deleted_audit_source_exhausted_original_outcome_required".into());
    }
    let parsed: JsonValue =
        serde_json::from_str(plan).map_err(|_| "source_completion_plan_invalid")?;
    let runtime: String = db
        .query_row(
            "SELECT contract_json FROM source_runtime_contracts
        WHERE scan_id=?1 AND attempt_number=?2 AND schema_version=1",
            params![actor.scan_id, actor.attempt_number],
            |r| r.get(0),
        )
        .map_err(|_| "deleted_audit_source_runtime_missing")?;
    if parsed["runtime"]
        != serde_json::from_str::<JsonValue>(&runtime)
            .map_err(|_| "deleted_audit_source_runtime_invalid")?
    {
        return Err("deleted_audit_source_runtime_changed".into());
    }
    // Audits original frozen material, completed predecessors, the unique
    // failed worker, all three actual SDK/tool receipts and both budget books.
    // The returned child is deliberately discarded; no cold actor escapes.
    audit_recovered_source_exhausted_root(db, actor)?;
    let roots = source_branch_original_roots(db, &actor.scan_id, actor.attempt_number)?;
    if roots != vec![actor.root_run_id.clone()] {
        return Err("source_branch_original_root_ambiguous".into());
    }
    let (status, checkpoint, raw): (String, String, String) = db
        .query_row(
            "SELECT status,checkpoint,report_json FROM native_scan_branches
        WHERE scan_id=?1 AND attempt_number=?2 AND branch='source'",
            params![actor.scan_id, actor.attempt_number],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "deleted_audit_source_branch_missing")?;
    let report: JsonValue =
        serde_json::from_str(&raw).map_err(|_| "deleted_audit_source_branch_invalid")?;
    let expected =
        source_branch_original_outcome(db, &actor.scan_id, actor.attempt_number, &report, &roots)?;
    if status != "partial" || expected != (status.as_str(), checkpoint) {
        return Err("deleted_audit_source_branch_changed".into());
    }
    let failure = "source_tool_phase_round_budget_exhausted_without_finish";
    let model = &report["sourceMultiAgent"];
    let known_report = if report["error"].is_null() {
        model["status"] == "incomplete" && model["reason"] == failure
    } else {
        report["error"] == failure && model.is_null()
    };
    if !known_report
        || model["independentReviewCompleted"] == true
        || model["independentCandidateReviewCompleted"] == true
        || report["independentReviewCompleted"] == true
        || !report["sourceCoverageDecision"].is_null()
        || !report["sourceDecisionProjection"].is_null()
    {
        return Err("deleted_audit_source_exhausted_report_changed".into());
    }
    let current: i64 = db
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&actor.scan_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if actor.attempt_number > current {
        return Err("deleted_audit_source_exhausted_attempt_changed".into());
    }
    if actor.attempt_number == current {
        // The current mutable target must describe this original failed Root.
        // This is pure provenance; it never issues a live actor or repairs rows.
        let path = source_pause_original_target_in(db, actor)?;
        let target: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1
            AND url=?2 AND status='partial' AND last_attempt_number=?3)",
                params![actor.scan_id, path, actor.attempt_number],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !target {
            return Err("deleted_audit_source_exhausted_target_changed".into());
        }
    }
    // Historical material validation never grants execution on an old attempt.
    let _ = source::completion_task_slices(db, actor)?;
    let source_plan = NativeSourcePlan::load(db, &actor.scan_id, actor.attempt_number)?
        .ok_or("source_ci_plan_missing")?;
    if source_plan.scan_type == "cicd" {
        let policy = load_historical_source_ci_policy(db, &actor.scan_id, actor.attempt_number)?;
        if policy != GatePolicy::from_workbench(&parsed["runtime"]["operatorPolicy"])? {
            return Err("deleted_audit_source_exhausted_ci_policy_changed".into());
        }
    }
    let no_completion:bool=db.query_row("SELECT NOT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1
        AND event_type='terminal_reduced' AND json_extract(payload_json,'$.sourceClosureVersion') IS NOT NULL)",
        [&actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !no_completion {
        return Err("deleted_audit_source_exhausted_completion_conflict".into());
    }
    Ok(())
}
