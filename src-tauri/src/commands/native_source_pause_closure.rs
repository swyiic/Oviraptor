// A pause request grants no new work. Select only the original, known and
// completely settled exhausted Source failure. Other obligations stay pending.
fn known_source_pause_root_in(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<Option<crate::agent_runtime::multi_agent::lease::CoordinatorLease>, String> {
    let roots = source_branch_original_roots(db, scan, attempt)?;
    let mut selected = None;
    for root in roots {
        let candidate:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND (status<>'terminal'
            OR EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?2 AND attempt_number=?3 AND branch='source' AND status='pending')))
            AND EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
            WHERE a.coordinator_run_id=?1 AND a.state='failed' AND a.trigger_code='source_tools_ready'
            AND r.terminal_reason='source_tool_phase_round_budget_exhausted_without_finish')",
            params![root,scan,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !candidate {
            continue;
        }
        if selected.is_some() {
            return Err("source_pause_original_root_ambiguous".into());
        }
        let actor = native_source_fresh_finance::original_for_financial_exit(db, &root)?;
        if actor.scan_id != scan || actor.attempt_number != attempt {
            return Err("source_pause_original_scope_changed".into());
        }
        source_pause_original_proof_in(
            db,
            &actor,
            "pausing",
            source_pause_root_closed_in(db, &actor)?,
        )?;
        selected = Some(actor);
    }
    Ok(selected)
}

// An already closed Root is consumed through the existing pure terminal
// replay: original cutoff, fees and Exit must verify; nothing is backfilled.
fn source_pause_root_closed_in(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<bool, String> {
    db.query_row(
        "SELECT status='terminal' FROM agent_runs WHERE id=?1",
        [&actor.root_run_id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

// Historical material reads and original C identity confer no execution.
// The private writer independently rechecks this after owning the SQLite lock.
fn source_pause_original_proof_in(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    status: &str,
    closed: bool,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{lease, source};
    if db.is_autocommit() {
        return Err("source_pause_transaction_required".into());
    }
    let same = native_source_fresh_finance::original_for_financial_exit(db, &actor.root_run_id)?;
    if &same != actor {
        return Err("source_pause_original_coordinator_changed".into());
    }
    lease::validate_coordinator_lease(db, actor)?;
    let valid: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a
        ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
        WHERE s.id=?1 AND s.attempt_count=?2 AND s.status=?3 AND a.status=?3)
        AND EXISTS(SELECT 1 FROM native_scan_branches b JOIN native_branch_dispatches d
        ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
        WHERE b.scan_id=?1 AND b.attempt_number=?2 AND b.branch='source' AND b.status=?6
        AND d.claim_id<>'' AND d.claimed_at<>''
        AND (?6<>'pending' OR (b.checkpoint='' AND b.report_json='{}')))
        AND EXISTS(SELECT 1 FROM agent_runs WHERE id=?4 AND cancel_requested_at=''
        AND CASE WHEN ?5 THEN status='terminal' ELSE status IN ('prepared','running') END)
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![
                actor.scan_id,
                actor.attempt_number,
                status,
                actor.root_run_id,
                closed,
                if status == "paused" {
                    "partial"
                } else {
                    "pending"
                }
            ],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !valid
        || source_branch_original_roots(db, &actor.scan_id, actor.attempt_number)?
            != vec![actor.root_run_id.clone()]
    {
        return Err("source_pause_original_state_conflict".into());
    }
    let text: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let plan: JsonValue = serde_json::from_str(&text).map_err(|_| "source_pause_plan_invalid")?;
    source::SourcePhaseContract::from_plan(&plan)?;
    let runtime: String = db
        .query_row(
            "SELECT contract_json FROM source_runtime_contracts WHERE scan_id=?1
        AND attempt_number=?2 AND schema_version=1",
            params![actor.scan_id, actor.attempt_number],
            |r| r.get(0),
        )
        .map_err(|_| "source_pause_runtime_missing")?;
    if plan["runtime"]
        != serde_json::from_str::<JsonValue>(&runtime)
            .map_err(|_| "source_pause_runtime_invalid")?
    {
        return Err("source_pause_original_runtime_changed".into());
    }
    let original = NativeSourcePlan::load(db, &actor.scan_id, actor.attempt_number)?
        .ok_or("source_pause_plan_missing")?;
    if original.scan_type == "cicd"
        && load_historical_source_ci_policy(db, &actor.scan_id, actor.attempt_number)?
            != GatePolicy::from_workbench(&plan["runtime"]["operatorPolicy"])?
    {
        return Err("source_pause_original_ci_policy_changed".into());
    }
    source_pause_original_target_in(db, actor)?;
    if status == "paused" {
        verify_known_source_pause_branch_in(db, actor)?;
    }
    audit_recovered_source_exhausted_root(db, actor)?;
    let no_completion:bool=db.query_row("SELECT NOT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1
        AND event_type='terminal_reduced' AND json_extract(payload_json,'$.sourceClosureVersion') IS NOT NULL)",
        [&actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !no_completion {
        return Err("source_pause_original_completion_conflict".into());
    }
    Ok(())
}

fn finish_known_source_pause(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<bool, String> {
    use crate::agent_runtime::multi_agent::budget;
    budget::clock::source_pause_write(db, actor, |tx| {
        source_pause_original_proof_in(
            tx,
            actor,
            "pausing",
            source_pause_root_closed_in(tx, actor)?,
        )?;
        let outcome = AgentTargetOutcome::incomplete(
            "源码多智能体执行未完成，子任务回执和未决预算保留待核对",
        );
        let clock = finish_coordinator_run_in_transaction(tx, actor, &outcome)?;
        source_pause_original_proof_in(tx, actor, "pausing", true)?;
        publish_known_source_pause_branch_in(tx, actor)?;
        publish_sentinel_pause_in(tx, &actor.scan_id, actor.attempt_number)?;
        source_pause_original_proof_in(tx, actor, "paused", true)?;
        clock.verify_closed(
            tx,
            outcome.terminal_status(),
            outcome.terminal_code(),
            &crate::agent_runtime::secrets::redact_text_with(&outcome.detail(), None),
        )?;
        Ok(true)
    })
}

include!("native_source_pause_branch.rs");
