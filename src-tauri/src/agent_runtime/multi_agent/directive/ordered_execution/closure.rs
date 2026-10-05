//! Original terminal transaction only; no new grant, receipt, worker or schema.
use super::*;
mod started;
mod transport;
#[derive(Clone, Copy)]
pub(crate) enum ClosureKind { NotApplied, ReconciliationRequired }

// Prove all nonterminal actions before making any release. A started/received
// action with a durable dispatch stays in reconciliation. Local start alone
// can close only with the complete original no-dispatch proof.
pub(crate) fn close_for_terminal(tx: &rusqlite::Transaction<'_>, scope: &CoordinatorLease, id: &str) -> Result<Option<ClosureKind>, String> {
    let eligible: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE id=?1 AND json_extract(payload_json,'$.readonlyAssessmentPlan.schemaVersion')=3)", [id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !eligible { return Ok(None); }
    let (_, original, state) = source(tx, id)?;
    if state != "assigned" { return Ok(None); }
    let owner = budget::root::RootOwner::load_original(tx, &original.root_run_id)?;
    owner.require_original_coordinator(tx, scope)?;
    let mut expected = scope.clone(); expected.lease_expires_at.clear();
    if original != expected { return Err("ordered_closure_original_scope_conflict".into()); }
    project(tx, id)?.ok_or("ordered_closure_projection_missing")?;
    let first = load(tx, id, 1)?.ok_or("ordered_closure_checkpoint_missing")?;
    let mut jobs = vec![first];
    if let Some(second) = load(tx, id, 2)? { jobs.push(second); }
    // Root is already terminal inside this uncommitted write transaction.
    // A later claimant cannot authorize a new dispatch after this commit.
    let _idle_transports = jobs.iter().map(|job| transport::require_idle(tx, job))
        .collect::<Result<Vec<_>, _>>()?;
    let mut unsent = vec![];
    let mut started = vec![];
    for job in &jobs {
        match job.state.as_str() {
            "prepared" => { require_unsent(tx, job, false)?; unsent.push((job, None)); }
            "executing" => {
                let dispatch: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2)",
                    params![job.child.assignment_id,job.child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
                if dispatch {
                    let proof = financial::metadata(tx, job)?;
                    if let Some(receipt) = specialist::not_sent_for_owned_proposal(tx, &job.scope, &job.child, &proof)? {
                        require_unsent(tx, job, false)?; unsent.push((job, Some(receipt)));
                    } else { started.push(job); }
                }
                else { financial::metadata(tx, job)?; require_unsent(tx, job, false)?; unsent.push((job, None)); }
            }
            "received" => started.push(job),
            "completed" | "failed" => { receipts::verified(tx, job)?.ok_or("ordered_closure_paid_receipt_missing")?; }
            _ => return Err("ordered_closure_state_conflict".into()),
        }
    }
    for job in started.iter() { started::pause(tx, job)?; }
    for (job, receipt) in unsent {
        if let Some(receipt) = receipt { scheduler::cancel_before_transport_child_in_transaction(tx, scope, &job.child, &receipt)?; }
        else if job.state == "prepared" { scheduler::cancel_unstarted_child_in_transaction(tx, scope, &job.child)?; }
        else { scheduler::cancel_started_before_model_dispatch_in_transaction(tx, scope, &job.child)?; }
        verify_cancelled(tx, job)?;
    }
    if !started.is_empty() { return Ok(Some(ClosureKind::ReconciliationRequired)); }
    Ok(Some(ClosureKind::NotApplied))
}

fn require_unsent(db: &Connection, job: &ActionJob, cancelled: bool) -> Result<attempts::AssignmentAttempt, String> {
    let worker = attempts::current(db, &job.scope, &job.child.assignment_id)?;
    let executing = job.state == "executing";
    let before_transport = if executing {
        specialist::not_sent_for_owned_proposal(db, &job.scope, &job.child, &financial::metadata(db, job)?)?.is_some()
    } else { false };
    let expected = if cancelled { "cancelled" } else if executing { "running" } else { "leased" };
    let bound: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND r.assignment_id=a.id AND a.state=?3 AND a.budget_settled_at=''
        AND ((?5=0 AND a.started_at='' AND r.started_at='') OR (?5=1 AND a.started_at<>'' AND r.started_at<>''))
        AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0
        AND ((?4=0 AND ((?5=0 AND r.status='prepared') OR (?5=1 AND r.status='running')) AND a.finished_at='' AND r.finished_at='' AND a.reserved_tokens=4000 AND a.reserved_requests=1)
          OR (?4=1 AND a.failure_class='proposal_task_ended_before_dispatch' AND a.finished_at<>'' AND r.status='terminal'
            AND r.terminal_state='cancelled' AND r.terminal_code='proposal_task_ended_before_dispatch' AND r.finished_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0)))
        AND (SELECT COUNT(*) FROM agent_assignment_attempts WHERE assignment_id=?1)=1", params![job.child.assignment_id, job.child.run_id, expected, cancelled, executing], |r|r.get(0)).map_err(|e|e.to_string())?;
    let sent: bool = db.query_row("SELECT (?4=0 AND EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2))
        OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE assignment_id=?1 OR child_run_id=?2)
        OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE assignment_id=?1 OR child_run_id=?2)
        OR EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE assignment_id=?1 OR child_run_id=?2)
        OR (?4=0 AND EXISTS(SELECT 1 FROM native_sdk_log_owners WHERE assignment_id=?1 OR run_id=?2))
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?2)
        OR EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?2)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)
        OR EXISTS(SELECT 1 FROM agent_directive_ordered_receipts WHERE action_id=?3 OR child_run_id=?2)
        OR EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?1 AND kind='human_ordered_assessment_result')",
        params![job.child.assignment_id, job.child.run_id, job.action.action_id, before_transport], |r|r.get(0)).map_err(|e|e.to_string())?;
    if executing { binding::verify_request(db, job, true)?; }
    if !bound || sent || worker.state != expected || worker.child_run_id != job.child.run_id || worker.lease_epoch != 1
        || (cancelled && (worker.finished_at.is_empty() || worker.failure_class != "proposal_task_ended_before_dispatch"))
        || (!cancelled && !worker.finished_at.is_empty()) { return Err("ordered_closure_unsent_proof_conflict".into()); }
    Ok(worker)
}

pub(super) fn verify_cancelled(db: &Connection, job: &ActionJob) -> Result<(), String> {
    let worker = require_unsent(db, job, true)?;
    let owner = budget::root::RootOwner::load_original(db, &job.scope.root_run_id)?;
    owner.require_original_coordinator(db, &job.scope)?;
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)
        OR EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')",
        params![job.child.assignment_id, job.child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if active { return Err("ordered_closure_leases_not_released".into()); }
    for dimension in &budget::DIMENSIONS[..4] {
        let amount = if *dimension == "model_requests" { 1 } else { 4000 };
        let exact: bool = db.query_row("SELECT COUNT(*)=2 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
            AND lease_attempt_id=?3 AND dimension=?4 AND amount=?5 AND ((kind='reserve' AND idempotency_key=?6 AND source_id=?7)
                OR (kind='release' AND idempotency_key=?8 AND source_id=?9))",
            params![job.scope.root_run_id, job.child.assignment_id, worker.id, dimension, amount,
                format!("reserve:{}:{dimension}", job.child.assignment_id), format!("assignment:{}", job.child.assignment_id),
                format!("finish:{}:{dimension}", job.child.assignment_id), format!("unsent:{}", job.child.assignment_id)], |r|r.get(0)).map_err(|e|e.to_string())?;
        let balance = budget::balance(db, &job.scope.root_run_id, Some(&job.child.assignment_id), dimension)?;
        if !exact || balance.reserved != 0 || balance.consumed != 0 || balance.indeterminate != 0 {
            return Err("ordered_closure_release_proof_conflict".into());
        }
    }
    Ok(())
}

pub(super) fn is_parent_closed(db: &Connection, job: &ActionJob) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives d JOIN agent_runs r ON r.id=d.root_run_id
        WHERE d.id=?1 AND r.status='terminal' AND d.status='deferred'
        AND json_extract(d.payload_json,'$.taskClosure.disposition')='not_applied'
        AND json_extract(d.payload_json,'$.taskClosure.requiresReconciliation')=0)", [&job.directive_id], |r|r.get(0)).map_err(|e|e.to_string())
}
