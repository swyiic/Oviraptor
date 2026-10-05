fn transition_proposal_on_closure(
    transaction: &rusqlite::Transaction<'_>,
    directive_id: &str,
    from_state: &str,
    to_state: &str,
    error_code: &str,
) -> Result<(), String> {
    let changed = transaction.execute(
        "UPDATE agent_directive_proposals SET state=?3,error_code=?4,\
         updated_at=datetime('now','localtime') WHERE directive_id=?1 AND state=?2",
        params![directive_id, from_state, to_state, error_code],
    ).map_err(|e| e.to_string())?;
    if changed != 1 { return Err("directive_closure_proposal_state_conflict".into()); }
    let confirmed: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_directive_proposals \
         WHERE directive_id=?1 AND state=?2 AND error_code=?3)",
        params![directive_id, to_state, error_code], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !confirmed { return Err("directive_closure_proposal_postcondition_conflict".into()); }
    Ok(())
}

/// A new target invocation owns the OS lock only after the prior invocation
/// has released it (including process death). A proposal left in `executing`
/// may already have reached the provider. Retain its reservation and make the
/// unknown outcome visible without stopping unrelated authorized Web work.
fn reconcile_abandoned_human_proposals(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    _invocation: &NativeInvocationOwner,
) -> Result<usize, String> {
    use crate::agent_runtime::multi_agent::lease::{
        require_executable_coordinator, validate_coordinator_lease,
    };
    let transaction = rusqlite::Transaction::new_unchecked(
        connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(|error| error.to_string())?;
    validate_coordinator_lease(&transaction, lease)?;
    require_executable_coordinator(&transaction, lease)?;
    let rows = {
        let mut statement = transaction.prepare(
            "SELECT p.directive_id,p.assignment_id,p.child_run_id FROM agent_directive_proposals p \
             JOIN agent_user_directives d ON d.id=p.directive_id \
             WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.target_key=?3 \
             AND d.root_run_id=?4 AND p.state='executing' ORDER BY d.rowid",
        ).map_err(|error| error.to_string())?;
        let selected = statement.query_map(params![lease.scan_id, lease.attempt_number, lease.target_key, lease.root_run_id],
            |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
        selected
    };
    for (directive_id, assignment_id, child_run_id) in &rows {
        // This is cleanup, not a new execution grant. Recheck the immutable
        // bindings and the old claim even when the current lease has a new
        // fencing epoch; never adopt or settle that older worker's budget.
        let bound: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_user_directives d \
             JOIN agent_directive_proposals p ON p.directive_id=d.id \
             JOIN agent_assignments a ON a.id=p.assignment_id \
             JOIN agent_runs r ON r.id=p.child_run_id \
             WHERE d.id=?1 AND p.assignment_id=?2 AND p.child_run_id=?3 \
             AND d.status='assigned' AND d.claim_run_id=?4 \
             AND d.claim_lease_epoch=a.lease_epoch AND d.claim_fencing_token=a.fencing_token \
             AND a.coordinator_run_id=?4 AND a.child_run_id=r.id \
             AND a.target_key=?5 AND a.trigger_code=('human_directive:' || d.id) \
             AND a.lane='read_only_analysis' AND r.lane=a.lane \
             AND a.state='running' AND a.budget_settled_at='' \
             AND a.reserved_requests=1 AND r.parent_run_id=?4 AND r.status='running' \
             AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?5)",
            params![directive_id, assignment_id, child_run_id, lease.root_run_id,
                lease.target_key, lease.scan_id, lease.attempt_number],
            |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !bound { return Err("directive_reentry_proposal_binding_conflict".into()); }
        transition_proposal_on_closure(&transaction, directive_id, "executing", "uncertain",
            "proposal_reentry_outcome_unknown")?;
        let changed = transaction.execute(
            "UPDATE agent_assignments SET state='paused',updated_at=datetime('now','localtime') \
             WHERE id=?1 AND child_run_id=?2 AND state='running' AND budget_settled_at=''",
            params![assignment_id, child_run_id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("directive_reentry_assignment_conflict".into()); }
        let changed = transaction.execute(
            "UPDATE agent_runs SET status='paused',updated_at=datetime('now','localtime') \
             WHERE id=?1 AND parent_run_id=?2 AND status='running'",
            params![child_run_id, lease.root_run_id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("directive_reentry_child_conflict".into()); }
        let active: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
            [child_run_id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let revoked = transaction.execute(
            "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') \
             WHERE child_run_id=?1 AND revoked_at=''", [child_run_id],
        ).map_err(|error| error.to_string())?;
        if i64::try_from(revoked).map_err(|error| error.to_string())? != active {
            return Err("directive_reentry_capability_conflict".into());
        }
        crate::agent_runtime::multi_agent::attempts::pause_original(&transaction,lease,assignment_id,child_run_id,
            "proposal_reentry_outcome_unknown")?;
        let changed = transaction.execute(
            "UPDATE agent_user_directives SET status='deferred', \
             rejection_code='proposal_reentry_outcome_unknown',finished_at=datetime('now','localtime'), \
             updated_at=datetime('now','localtime') WHERE id=?1 AND status='assigned' \
             AND claim_run_id=?2 AND scan_id=?3 AND attempt_number=?4 AND target_key=?5",
            params![directive_id, lease.root_run_id, lease.scan_id, lease.attempt_number, lease.target_key],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("directive_reentry_directive_conflict".into()); }
        // Only the OS-lock successor can know the old local worker is gone.
        // An uncertain provider response still owns its budget reservation,
        // but a paused read-only child has no target action to serialize.
        // Never apply this release rule to target_touching or review lanes.
        transaction.execute(
            "DELETE FROM agent_lane_leases WHERE assignment_id=?1 AND scan_id=?2 \
             AND attempt_number=?3 AND target_key=?4 AND lane='read_only_analysis' \
             AND EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
               WHERE a.id=?1 AND a.child_run_id=?5 AND a.state='paused' AND r.status='paused' \
               AND a.lane='read_only_analysis' AND r.lane=a.lane)",
            params![assignment_id, lease.scan_id, lease.attempt_number, lease.target_key, child_run_id],
        ).map_err(|error| error.to_string())?;
        let confirmed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_directive_proposals p \
             JOIN agent_user_directives d ON d.id=p.directive_id \
             JOIN agent_assignments a ON a.id=p.assignment_id \
             JOIN agent_runs r ON r.id=p.child_run_id \
             WHERE d.id=?1 AND p.state='uncertain' AND p.error_code='proposal_reentry_outcome_unknown' \
             AND d.status='deferred' AND d.rejection_code='proposal_reentry_outcome_unknown' \
             AND a.state='paused' AND a.budget_settled_at='' AND a.reserved_requests=1 \
             AND r.status='paused' AND NOT EXISTS(SELECT 1 FROM agent_capability_leases c \
               WHERE c.child_run_id=r.id AND c.revoked_at='') \
             AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))",
            [directive_id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !confirmed { return Err("directive_reentry_postcondition_conflict".into()); }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(rows.len())
}

/// Close only this target's unresolved inbox, atomically with its root run.
/// A terminal task is not evidence that an instruction was executed. In-flight
/// calls retain their reservation until an explicit reconciliation can prove cost.
fn close_human_directives_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    terminal_code: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{directive::proposals, lease::validate_coordinator_lease, scheduler};
    validate_coordinator_lease(transaction, lease)?;
    let items = {
        let mut query = transaction.prepare(
            "SELECT id,status,claim_run_id,claim_lease_epoch,claim_fencing_token FROM agent_user_directives \
             WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 \
             AND status IN ('pending','claimed','accepted','assigned','applied') \
             AND json_extract(payload_json,'$.taskClosure') IS NULL ORDER BY rowid",
        ).map_err(|e| e.to_string())?;
        let rows = query.query_map(params![lease.scan_id,lease.attempt_number,lease.target_key,lease.root_run_id],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?
    };
    for (id, status, owner, epoch, token) in items {
        let has_proposal: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_directive_proposals WHERE directive_id=?1)", [&id], |r|r.get(0),
        ).map_err(|e|e.to_string())?;
        let current_claim = owner == lease.root_run_id && epoch == lease.lease_epoch && token == lease.fencing_token;
        let unclaimed = status == "pending" && owner.is_empty() && epoch == 0 && token.is_empty();
        let (disposition, reason, reconciliation) = if !current_claim && !unclaimed {
            // Never adopt a replaced worker's assignments or refund its budget.
            ("reconciliation_required", "directive_task_ended_old_fence", true)
        } else if status=="accepted" && crate::agent_runtime::multi_agent::directive::source_guidance::project_delivery(transaction,&id)?.is_some() {
            // The completed action is delivery of an advisory preference. It
            // does not assert adoption, successful tools or a confirmed finding.
            ("analysis_guidance_delivered", "", false)
        } else if matches!(status.as_str(), "pending" | "claimed" | "accepted") {
            ("not_applied", "directive_task_ended_without_action", false)
        } else if let Some(kind) = crate::agent_runtime::multi_agent::directive::ordered_execution::close_for_terminal(transaction, lease, &id)? {
            match kind {
                crate::agent_runtime::multi_agent::directive::ordered_execution::ClosureKind::NotApplied =>
                    ("not_applied", "directive_task_ended_without_action", false),
                crate::agent_runtime::multi_agent::directive::ordered_execution::ClosureKind::ReconciliationRequired =>
                    ("reconciliation_required", "directive_task_ended_execution_receipt_missing", true),
            }
        } else if let Some(job) = if has_proposal { proposals::load_job(transaction, lease, &id)? } else { None } {
            match job.state.as_str() {
                "prepared" => {
                    let unstarted: bool = transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
                         WHERE a.id=?1 AND r.id=?2 AND a.state='leased' AND a.started_at='' AND r.status='prepared')",
                        params![job.child.assignment_id,job.child.run_id], |r|r.get(0),
                    ).map_err(|e|e.to_string())?;
                    if !unstarted { return Err("directive_closure_dispatch_state_conflict".into()); }
                    scheduler::cancel_unstarted_child_in_transaction(transaction,lease,&job.child)?;
                    transition_proposal_on_closure(transaction, &id, "prepared", "failed", "proposal_task_ended_before_dispatch")?;
                    ("not_started", "directive_task_ended_without_action", false)
                }
                "executing" | "uncertain" | "received" => {
                    if job.state == "executing" {
                        transition_proposal_on_closure(transaction, &id, "executing", "uncertain", "proposal_task_ended_outcome_unknown")?;
                    }
                    // No finish_child here: that would release potentially billed usage.
                    let changed = transaction.execute(
                        "UPDATE agent_assignments SET state='paused',updated_at=datetime('now','localtime') \
                         WHERE id=?1 AND state IN ('running','paused') AND lease_epoch=?2 AND fencing_token=?3",
                        params![job.child.assignment_id,lease.lease_epoch,lease.fencing_token],
                    ).map_err(|e|e.to_string())?;
                    if changed != 1 { return Err("directive_closure_assignment_state_conflict".into()); }
                    let changed = transaction.execute(
                        "UPDATE agent_runs SET status='paused',updated_at=datetime('now','localtime') \
                         WHERE id=?1 AND root_run_id=?2 AND status IN ('running','paused')",
                        params![job.child.run_id,lease.root_run_id],
                    ).map_err(|e|e.to_string())?;
                    if changed != 1 { return Err("directive_closure_child_state_conflict".into()); }
                    let active_capabilities: i64 = transaction.query_row(
                        "SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
                        [&job.child.run_id], |r| r.get(0),
                    ).map_err(|e| e.to_string())?;
                    let changed = transaction.execute(
                        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1 AND revoked_at=''",
                        [&job.child.run_id],
                    ).map_err(|e|e.to_string())?;
                    if i64::try_from(changed).map_err(|e|e.to_string())? != active_capabilities {
                        return Err("directive_closure_capability_revocation_conflict".into());
                    }
                    let remaining: bool = transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='')",
                        [&job.child.run_id], |r| r.get(0),
                    ).map_err(|e| e.to_string())?;
                    if remaining { return Err("directive_closure_capability_postcondition_conflict".into()); }
                    crate::agent_runtime::multi_agent::attempts::pause(transaction,lease,&job.child,
                        "directive_task_ended_outcome_unknown")?;
                    if job.state == "received" {
                        ("receipt_pending", "directive_task_ended_receipt_pending", true)
                    } else {
                        ("outcome_unknown", "directive_task_ended_outcome_unknown", true)
                    }
                }
                _ => return Err("directive_closure_proposal_receipt_conflict".into()),
            }
        } else {
            ("reconciliation_required", "directive_task_ended_execution_receipt_missing", true)
        };
        let receipt = serde_json::json!({
            "fromStatus":status,"disposition":disposition,"rootTerminalCode":terminal_code,
            "requiresReconciliation":reconciliation,"automaticRetry":false,
        });
        let final_status=if disposition=="analysis_guidance_delivered" {"completed"} else {"deferred"};
        let changed = transaction.execute(
            "UPDATE agent_user_directives SET status=?5,rejection_code=?1,\
             payload_json=json_set(payload_json,'$.taskClosure',json_set(json(?2),'$.closedAt',datetime('now','localtime'))),\
             finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE id=?3 AND status=?4 AND json_extract(payload_json,'$.taskClosure') IS NULL",
            params![reason,receipt.to_string(),id,status,final_status],
        ).map_err(|e|format!("无法收口用户指令：{e}"))?;
        if changed != 1 { return Err("directive_closure_state_conflict".into()); }
        let confirmed:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE id=?1 AND status=?2 AND rejection_code=?3 AND json(json_remove(json_extract(payload_json,'$.taskClosure'),'$.closedAt'))=json(?4))",
            params![id,final_status,reason,receipt.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !confirmed {return Err("directive_closure_postcondition_conflict".into());}
        if disposition=="analysis_guidance_delivered" && crate::agent_runtime::multi_agent::directive::source_guidance::project_delivery(transaction,&id)?.is_none() {
            return Err("source_guidance_delivery_missing".into());
        }
    }
    Ok(())
}
