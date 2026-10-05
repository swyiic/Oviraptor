// Source branch projection consumes original facts only; never creates a Root,
// lease, fee, mailbox ACK or execution permission from caller report fields.
fn source_branch_original_roots(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<Vec<String>, String> {
    // The immutable financial scope survives lost mutable run/C labels. It
    // only locates original obligations; the consumer still verifies identity,
    // material and exit, and cannot create execution authority from this read.
    let mut query=db.prepare("SELECT root_run_id FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key LIKE 'source:%'
        UNION SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND role='coordinator'
        AND (target_url LIKE 'source:%' OR json_extract(plan_json,'$.surface')='source')
        UNION SELECT root_run_id FROM agent_root_budget_attempts
        WHERE json_extract(contract_json,'$.root.scan')=?1 AND json_extract(contract_json,'$.root.attempt')=?2
        AND json_extract(contract_json,'$.root.policy')='multi' AND json_extract(contract_json,'$.root.target') LIKE 'source:%'
        ORDER BY 1").map_err(|e|e.to_string())?;
    let roots = query
        .query_map(params![scan, attempt], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(roots)
}

fn source_branch_original_outcome(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    report: &JsonValue,
    roots: &[String],
) -> Result<(&'static str, String), String> {
    use crate::agent_runtime::multi_agent::{budget, source};
    let error = report["error"].as_str().filter(|s| !s.is_empty());
    let model = &report["sourceMultiAgent"];
    if roots.is_empty() {
        if error.is_some() {
            return Ok(("failed", "源码分支未完成；没有原Source Root执行结果".into()));
        }
        if model["status"] == "incomplete" && model["rootRunId"].is_null() {
            return Ok((
                "partial",
                "源码材料已生成；多智能体执行未完成，不能作为完成覆盖".into(),
            ));
        }
        return Err("source_branch_completion_requires_original_root".into());
    }
    if roots.len() != 1 {
        return Err("source_branch_original_root_ambiguous".into());
    }
    if report["executionStarted"] == false {
        return Err("source_branch_preexecution_claim_conflicts_with_root".into());
    }
    let root = &roots[0];
    let claimed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2
        AND branch='source' AND claim_id<>'' AND claimed_at<>'')",params![scan,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !claimed {
        return Err("source_branch_original_dispatch_unclaimed".into());
    }
    let actor = native_source_fresh_finance::original_for_financial_exit(db, root)?;
    if actor.scan_id != scan || actor.attempt_number != attempt {
        return Err("source_branch_original_scope_changed".into());
    }
    let bases = source::completion_task_slices(db, &actor)?;
    budget::clock::FinalClock::verify_original_exit(db, root)?;
    let (state,terminal,code,reason):(String,String,String,String)=db.query_row("SELECT status,terminal_state,terminal_code,terminal_reason FROM agent_runs WHERE id=?1",[root],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    if state != "terminal" {
        return Err("source_branch_original_root_not_terminal".into());
    }
    if !model["rootRunId"].is_null() && model["rootRunId"] != root.as_str() {
        return Err("source_branch_report_root_changed".into());
    }
    if error.is_none() {
        let (_, results, _) = source::historical_materials(db, scan, attempt)?;
        if report["analysisResultsDigest"] != results.digest() {
            return Err("source_branch_report_results_changed".into());
        }
    }
    let status = match terminal.as_str() {
        "completed" | "completed_with_gaps" => {
            if error.is_some() {
                return Err("source_branch_completed_root_missing_report".into());
            }
            let proof = source_assessment_completion(db, &actor, &bases)?;
            let outcome = source_determined_terminal_outcome(&proof);
            if outcome.terminal_status() != terminal
                || outcome.terminal_code() != code
                || crate::agent_runtime::secrets::redact_text_with(&outcome.detail(), None)
                    != reason
            {
                return Err("source_branch_original_outcome_changed".into());
            }
            if model["rootRunId"] != root.as_str()
                || model["independentReviewCompleted"] != proof.coverage_decision.is_some()
                || model["independentCandidateReviewCompleted"] != proof.candidate_review.is_some()
                || model["modelRequests"] != proof.completion.model_requests
                || model["totalTokens"] != proof.completion.total_tokens
            {
                return Err("source_branch_report_completion_changed".into());
            }
            if terminal == "completed" {
                "completed"
            } else {
                "completed_with_gaps"
            }
        }
        "paused" | "protected_stop" | "resume_incompatible" | "cancelled" => {
            if error.is_none() && model["status"] != "incomplete" {
                return Err("source_branch_incomplete_report_conflict".into());
            }
            "partial"
        }
        "failed" => "failed",
        _ => return Err("source_branch_original_terminal_invalid".into()),
    };
    Ok((
        status,
        crate::agent_runtime::secrets::redact_text_with(&reason, None),
    ))
}

fn finish_native_source_branch(
    db_path: &Path,
    scan: &str,
    attempt: i64,
    report: &JsonValue,
) -> Result<bool, String> {
    let mut db = db::open(db_path)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // Same attempt and branch transaction fences late callbacks and duplicates
    // before reading original facts, including after the original scan closes.
    if !native_source_attempt_active(&tx, scan, attempt) {
        return Ok(false);
    }
    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2
        AND branch='source' AND status='pending')",params![scan,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !pending {
        return Ok(false);
    }
    let roots = source_branch_original_roots(&tx, scan, attempt)?;
    let (status, reason) = source_branch_original_outcome(&tx, scan, attempt, report, &roots)?;
    let siblings = workbench_admission_siblings(&tx, scan, attempt, "source")?;
    let changed = finish_native_branch_in(&tx, scan, attempt, "source", status, &reason, report)?;
    let published:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_scan_branches b
        JOIN sentinel_scans s ON s.id=b.scan_id JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
        WHERE b.scan_id=?1 AND b.attempt_number=?2 AND b.branch='source' AND b.status=?3 AND b.checkpoint=?4 AND b.report_json=?5
        AND s.attempt_count=?2 AND a.status=s.status AND a.checkpoint=s.current_checkpoint
        AND s.status=CASE WHEN EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status='pending') THEN 'scanning'
            WHEN NOT EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status<>'failed') THEN 'failed'
            WHEN EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status IN ('failed','partial')) THEN 'partial'
            WHEN EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status='completed_with_gaps') THEN 'completed_with_gaps' ELSE 'completed' END)
        AND EXISTS(SELECT 1 FROM sentinel_targets t JOIN sentinel_scans s ON s.id=t.scan_id WHERE t.scan_id=?1 AND t.url=s.source_path AND t.status=?3 AND t.last_attempt_number=?2)
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",params![scan,attempt,status,reason,report.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !changed || !published {
        return Err("source_branch_projection_unconfirmed".into());
    }
    if workbench_admission_siblings(&tx, scan, attempt, "source")? != siblings {
        return Err("source_branch_sibling_changed_during_projection".into());
    }
    if source_branch_original_roots(&tx, scan, attempt)? != roots {
        return Err("source_branch_root_changed_during_projection".into());
    }
    if !roots.is_empty() {
        // Triggers cannot alter identity, material, original fees or exit proof
        // after authorization. A failure rolls back every projection write.
        let after = source_branch_original_outcome(&tx, scan, attempt, report, &roots)?;
        if after != (status, reason) {
            return Err("source_branch_outcome_changed_during_projection".into());
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}
