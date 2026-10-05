// Retry only local delivery of a saved response. A failed/unknown transport
// never enters this path; permissions are not reissued to reconcile costs.
fn deliver_source_coverage_review(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<JsonValue, String> {
    match complete_source_coverage_review(connection, context, lease, child) {
        Ok(payload) => Ok(payload),
        Err(error) => {
            stop_failed_child_preserving_usage(connection, lease, child, &error)
                .map_err(|cleanup| format!("{error};source_coverage_review_cleanup:{cleanup}"))?;
            complete_source_coverage_review(connection, context, lease, child).map_err(|recovery| {
                format!("{error};source_coverage_review_local_recovery:{recovery}")
            })
        }
    }
}

fn complete_source_coverage_review(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::{mailbox, scheduler, source_coverage_reviewer};
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    authorize_source_specialist(&tx, context, lease)?;
    let expired =
        crate::agent_runtime::multi_agent::attempts::ExpiredSavedProof::capture(&tx, lease, child)?;
    let (payload, usage) = source_coverage_reviewer::receipt_payload(&tx, lease, child)?;
    let state: String = tx
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if state == "completed" {
        let audit = source_coverage_reviewer::audit_delivery(&tx, lease)?;
        if audit.child != *child || audit.payload != payload || audit.usage != usage {
            return Err("source_coverage_review_replay_mismatch".into());
        }
        return Ok(payload);
    }
    if state != "running" && state != "paused" {
        return Err("source_coverage_review_delivery_requires_running_receipt".into());
    }
    if state == "paused" {
        let recoverable:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
            JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number
            AND l.target_key=r.target_url AND l.lane=a.lane
            WHERE a.id=?1 AND a.state='paused' AND r.status='paused' AND a.budget_settled_at=''
            AND (a.failure_class='child_usage_reconciliation_required' OR (?3 AND a.failure_class='worker_lease_expired')))
            AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')",
            params![child.assignment_id,child.run_id,expired.is_some()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !recoverable {
            return Err("source_coverage_review_recovery_state_conflict".into());
        }
    }
    let message = mailbox::send_saved_specialist(
        &tx,
        lease,
        &child.run_id,
        &lease.root_run_id,
        "evidence_reviewer",
        "coordinator",
        "source_coverage_review_result",
        &format!("source-coverage-review:{}", child.assignment_id),
        &child.assignment_id,
        1,
        &payload,
    )?;
    consume_proposal_mailbox_in_transaction(
        &tx,
        lease,
        &lease.root_run_id,
        &message,
        "source_coverage_review_result",
        &payload,
    )?;
    // Persisted response, exact source material, and authorization are checked
    // again after mailbox triggers and again after child termination.
    if source_coverage_reviewer::receipt_payload(&tx, lease, child)? != (payload.clone(), usage) {
        return Err("source_coverage_review_receipt_changed".into());
    }
    authorize_source_specialist(&tx, context, lease)?;
    if state == "paused" {
        settle_child_usage_for_status(&tx, lease, child, &usage, "paused")?;
        let changed=tx.execute("UPDATE agent_assignments SET state='completed',failure_class='',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime')
            WHERE id=?1 AND state='paused' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0",[&child.assignment_id]).map_err(|e|e.to_string())?;
        let finished=tx.execute("UPDATE agent_runs SET status='terminal',terminal_state='completed',terminal_code='source_coverage_review_receipt_reconciled',
            terminal_reason='Saved source review delivered locally without model replay or renewed capabilities',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime')
            WHERE id=?1 AND status='paused'",[&child.run_id]).map_err(|e|e.to_string())?;
        if changed != 1 || finished != 1 {
            return Err("source_coverage_review_recovery_write_conflict".into());
        }
        crate::agent_runtime::multi_agent::budget::limits::release_slot(
            &tx,
            lease,
            &child.assignment_id,
        )?;
        tx.execute(
            "DELETE FROM agent_lane_leases WHERE assignment_id=?1",
            [&child.assignment_id],
        )
        .map_err(|e| e.to_string())?;
        crate::agent_runtime::multi_agent::attempts::finish_saved(&tx, lease, child)?;
    } else {
        settle_child_usage_in_transaction(&tx, lease, child, &usage)?;
        scheduler::finish_child_in_transaction(
            &tx,
            lease,
            child,
            true,
            payload["summary"]
                .as_str()
                .ok_or("source_coverage_review_summary_missing")?,
        )?;
    }
    authorize_source_specialist(&tx, context, lease)?;
    crate::agent_runtime::multi_agent::source_coverage_decisions::publish(&tx, lease)?;
    authorize_source_specialist(&tx, context, lease)?;
    let audit = source_coverage_reviewer::audit_delivery(&tx, lease)?;
    if audit.child != *child || audit.payload != payload || audit.usage != usage {
        return Err("source_coverage_review_delivery_changed".into());
    }
    if let Some(proof) = expired {
        proof.verify(&tx, lease, child)?;
    }
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&tx, lease)?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(&tx, lease)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(payload)
}

fn run_source_coverage_review(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<JsonValue>, String> {
    use crate::agent_runtime::multi_agent::{source_coverage_reviewer, source_phases};
    if !source_coverage_reviewer::enabled(connection, lease)? {
        return Ok(None);
    }
    let (slice, received) = {
        let tx = rusqlite::Transaction::new_unchecked(
            connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| e.to_string())?;
        authorize_source_specialist(&tx, context, lease)?;
        let phases = source_phases::audit(&tx, lease)?;
        let received = match source_coverage_reviewer::progress(&tx, lease)? {
            source_coverage_reviewer::ReviewProgress::NotStarted => {
                source_assessment_completion(&tx, lease, &phases.bases)?;
                crate::agent_runtime::multi_agent::directive::source_guidance::freeze_review_in_transaction(&tx,lease,true)?;
                None
            }
            source_coverage_reviewer::ReviewProgress::Undispatched(child) => {
                source_coverage_reviewer::verify_assignment(&tx, lease, &child)?;
                None
            }
            source_coverage_reviewer::ReviewProgress::Received(child) => Some(child),
            source_coverage_reviewer::ReviewProgress::Delivered(audit) => {
                return Ok(Some(audit.payload))
            }
        };
        let slice = source_coverage_reviewer::task_slice(&tx, lease)?;
        tx.commit().map_err(|e| e.to_string())?;
        (slice, received)
    };
    if let Some(child) = received {
        return deliver_source_coverage_review(connection, context, lease, &child).map(Some);
    }
    let Some(slice) = slice else {
        return Ok(None);
    };
    let profile = agent_model_profile(context.environment, context.proxy)?;
    let (tokens, _) = source_assessment_budget(
        &source_assessment_messages(source_coverage_reviewer::SYSTEM, &slice),
        &profile,
    )?;
    let child = source_coverage_reviewer::prepare(connection, lease, &slice, tokens)?;
    specialist_round_transport(
        context,
        lease,
        &child,
        source_coverage_reviewer::SYSTEM,
        slice,
    )
    .map_err(|error| failed_specialist_error(connection, lease, &child, &error))?;
    // Delivery failures retain the received response and reserved accounting;
    // they do not send the model request again or manufacture a refund.
    deliver_source_coverage_review(connection, context, lease, &child).map(Some)
}
