pub fn mark_child_running(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    validate_coordinator_lease(connection, lease)?;
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 child 启动事务：{error}"))?;
    mark_child_running_in_transaction(&transaction, lease, child)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 child 启动事务：{error}"))
}

pub(super) fn mark_child_running_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    validate_coordinator_lease(transaction, lease)?;
    require_executable_coordinator(transaction, lease)?;
    super::source::validate_surface_role(transaction, lease, child.role)?;
    if child.role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(transaction, lease, child.role)?
    {
        super::source_review_subject::verify_assignment(transaction, lease, child)?;
    }
    let expected = super::assignment::load_assignment(transaction, &child.assignment_id)?
        .ok_or("scheduled_authority_missing")?;
    if verify_scheduled_authority(transaction, lease, &expected)? != *child {
        return Err("child_start_binding_conflict".into());
    }
    super::attempts::start(transaction,lease,child)?;
    let changed = transaction
        .execute(
            "UPDATE agent_assignments SET state='running',started_at=CASE WHEN started_at='' THEN datetime('now','localtime') ELSE started_at END,\
             updated_at=datetime('now','localtime') WHERE id=?1 AND child_run_id=?2 AND state='leased' AND lease_epoch=?3 AND fencing_token=?4",
            params![
                child.assignment_id,
                child.run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
        )
        .map_err(|error| format!("无法启动 assignment：{error}"))?;
    if changed != 1 {
        return Err("assignment_fencing_or_state_conflict".into());
    }
    let run_started = transaction
        .execute(
            "UPDATE agent_runs SET status='running',started_at=CASE WHEN started_at='' THEN datetime('now','localtime') ELSE started_at END,\
             updated_at=datetime('now','localtime') WHERE id=?1 AND root_run_id=?2 AND assignment_id=?3 AND status='prepared'",
            params![child.run_id, lease.root_run_id, child.assignment_id],
        )
        .map_err(|error| format!("无法启动 child run：{error}"))?;
    if run_started != 1 {
        return Err("child_run_start_fencing_or_state_conflict".into());
    }
    super::source::validate_surface_role(transaction, lease, child.role)?;
    if child.role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(transaction, lease, child.role)?
    {
        super::source_review_subject::verify_assignment(transaction, lease, child)?;
    }
    if verify_scheduled_authority(transaction, lease, &expected)? != *child {
        return Err("child_start_binding_conflict".into());
    }
    let started: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE a.id=?1 AND r.id=?2 AND a.state='running' AND r.status='running')",
        params![child.assignment_id,child.run_id], |r|r.get(0),
    ).map_err(|e|format!("child_start_postcondition:{e}"))?;
    if !started {
        return Err("child_start_state_conflict".into());
    }
    Ok(())
}

/// An unstarted assignment must not keep its lane and budget if the child
/// cannot enter the running state. Fencing prevents a stale coordinator from
/// releasing a replacement worker's resources.
pub fn start_child_or_release(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    if let Err(error) = mark_child_running(connection, lease, child) {
        if let Err(cleanup) = finish_child(connection, lease, child, false, &error) {
            return Err(format!("{error};child_start_cleanup:{cleanup}"));
        }
        return Err(error);
    }
    Ok(())
}

pub fn finish_child(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    success: bool,
    summary: &str,
) -> Result<(), String> {
    validate_coordinator_lease(connection, lease)?;
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 child 结束事务：{error}"))?;
    finish_child_in_transaction(&transaction, lease, child, success, summary)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 child 结束事务：{error}"))
}

pub(crate) fn finish_child_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    success: bool,
    summary: &str,
) -> Result<(), String> {
    finish_child_disposition(transaction,lease,child,if success {ChildFinish::Completed} else {ChildFinish::Failed},summary)
}

#[derive(Clone,Copy,PartialEq,Eq)]
enum ChildFinish {Completed,Failed,CancelledBeforeDispatch}

pub(crate) fn cancel_unstarted_child_in_transaction(
    tx:&rusqlite::Transaction<'_>,lease:&CoordinatorLease,child:&ScheduledChild,
)->Result<(),String> {
    let unstarted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state='leased' AND a.started_at='' AND r.status='prepared')",
        params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !unstarted {return Err("child_cancel_requires_unstarted_assignment".into());}
    finish_child_disposition(tx,lease,child,ChildFinish::CancelledBeforeDispatch,"Task ended before proposal dispatch")
}

// Local running state is not permission to refund a sent or unknown call.
// This narrow terminal consumer rejects every durable child execution record.
pub(crate) fn cancel_started_before_model_dispatch_in_transaction(
    tx: &rusqlite::Transaction<'_>, lease: &CoordinatorLease, child: &ScheduledChild,
) -> Result<(), String> {
    let no_dispatch: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state='running' AND r.status='running' AND a.started_at<>'' AND r.started_at<>''
        AND a.finished_at='' AND r.finished_at='' AND a.budget_settled_at='' AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0)
        AND NOT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM native_sdk_log_owners WHERE assignment_id=?1 OR run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)",
        params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !no_dispatch { return Err("child_cancel_requires_original_no_dispatch_proof".into()); }
    finish_child_disposition(tx, lease, child, ChildFinish::CancelledBeforeDispatch, "Task ended before proposal dispatch")
}

// Only the specialist receipt verifier can construct this original proof.
pub(crate) fn cancel_before_transport_child_in_transaction(
    tx: &rusqlite::Transaction<'_>, lease: &CoordinatorLease, child: &ScheduledChild,
    receipt: &super::specialist::UnsentCall,
) -> Result<(), String> {
    receipt.require(tx, lease, child)?;
    let untouched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state='running' AND r.status='running' AND a.budget_settled_at=''
        AND a.finished_at='' AND r.finished_at='' AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0)
        AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE assignment_id=?1 OR child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)",
        params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !untouched { return Err("child_cancel_unsent_execution_history_conflict".into()); }
    finish_child_disposition(tx, lease, child, ChildFinish::CancelledBeforeDispatch, "Task ended before proposal dispatch")?;
    receipt.require(tx, lease, child)
}

fn finish_child_disposition(
    transaction:&rusqlite::Transaction<'_>,lease:&CoordinatorLease,child:&ScheduledChild,
    disposition:ChildFinish,summary:&str,
)->Result<(),String> {
    let success=disposition==ChildFinish::Completed;
    let (state,code,failure)=match disposition {
        ChildFinish::Completed=>("completed","child_completed",""),
        ChildFinish::Failed=>("failed","child_failed","child_execution_failed"),
        ChildFinish::CancelledBeforeDispatch=>("cancelled","proposal_task_ended_before_dispatch","proposal_task_ended_before_dispatch"),
    };
    validate_coordinator_lease(transaction, lease)?;
    let event_floor: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("child_finish_event_cursor:{error}"))?;
    let (reserved_tokens, reserved_requests): (i64, i64) = transaction
        .query_row(
            "SELECT a.reserved_tokens,a.reserved_requests FROM agent_assignments a \
             JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id \
             JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id \
               AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane \
             WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 \
               AND r.parent_run_id=?3 AND a.role=?4 AND r.role=?4 AND r.lane=a.lane \
               AND a.target_key=?5 AND r.target_url=?5 AND r.scan_id=?6 AND r.attempt_number=?7 \
               AND a.lease_epoch=?8 AND a.fencing_token=?9 \
               AND ((a.state IN ('running','waiting_review') AND r.status='running') \
                 OR (a.state='leased' AND r.status='prepared' AND ?10=0))",
            params![child.assignment_id, child.run_id, lease.root_run_id, child.role.as_str(),
                lease.target_key, lease.scan_id, lease.attempt_number, lease.lease_epoch, lease.fencing_token, success],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("child_finish_binding_conflict:{error}"))?;
    // Capture the whole ledger, not just this child's reservation. Completion
    // must not release a sibling's budget, change the total, or invent spend.
    let budget = || -> Result<[i64; 6], String> {
        transaction.query_row(
            "SELECT total_tokens,total_requests,reserved_tokens,reserved_requests,spent_tokens,spent_requests \
             FROM agent_budget_ledger WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3",
            params![lease.root_run_id,lease.lease_epoch,lease.fencing_token],
            |row| Ok([row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?]),
        ).map_err(|error|format!("child_finish_budget_unavailable:{error}"))
    };
    let mut expected_budget = budget()?;
    expected_budget[2] = expected_budget[2]
        .checked_sub(reserved_tokens)
        .filter(|value| *value >= 0)
        .ok_or("child_finish_budget_invalid")?;
    expected_budget[3] = expected_budget[3]
        .checked_sub(reserved_requests)
        .filter(|value| *value >= 0)
        .ok_or("child_finish_budget_invalid")?;
    super::budget::model::release_unsent(transaction, lease, &child.assignment_id)?;
    super::budget::limits::release_slot(transaction, lease, &child.assignment_id)?;
    if reserved_tokens > 0 || reserved_requests > 0 {
        let released = transaction
            .execute(
                "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens-?1,reserved_requests=reserved_requests-?2,\
                 updated_at=datetime('now','localtime') WHERE root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 \
                 AND reserved_tokens>=?1 AND reserved_requests>=?2",
                params![
                    reserved_tokens,
                    reserved_requests,
                    lease.root_run_id,
                    lease.lease_epoch,
                    lease.fencing_token
                ],
            )
            .map_err(|error| format!("无法释放 child 剩余预算：{error}"))?;
        if released != 1 {
            return Err("child_budget_release_fencing_conflict".into());
        }
    }
    let reason = crate::agent_runtime::secrets::redact_text_with(summary, None);
    let changed = transaction
        .execute(
            "UPDATE agent_assignments SET state=?1,failure_class=?7,\
             reserved_tokens=0,reserved_requests=0,finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE id=?3 AND child_run_id=?4 AND (state IN ('running','waiting_review') OR (state='leased' AND ?2=0)) \
             AND lease_epoch=?5 AND fencing_token=?6",
            params![
                state,
                success,
                child.assignment_id,
                child.run_id,
                lease.lease_epoch,
                lease.fencing_token,
                failure
            ],
        )
        .map_err(|error| format!("无法结束 assignment：{error}"))?;
    if changed != 1 {
        return Err("assignment_finish_fencing_or_state_conflict".into());
    }
    let run_changed = transaction
        .execute(
            "UPDATE agent_runs SET status='terminal',terminal_state=?1,terminal_code=?2,terminal_reason=?3,\
             finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?4 AND root_run_id=?5",
            params![
                state,
                code,
                reason,
                child.run_id,
                lease.root_run_id
            ],
        )
        .map_err(|error| format!("无法结束 child run：{error}"))?;
    if run_changed != 1 {
        return Err("child_finish_run_write_unconfirmed".into());
    }
    transaction
        .execute(
            "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1 AND revoked_at=''",
            [&child.run_id],
        )
        .map_err(|error| format!("无法回收 capability lease：{error}"))?;
    transaction
        .execute(
            "DELETE FROM agent_lane_leases WHERE assignment_id=?1 AND EXISTS(\
             SELECT 1 FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND state IN ('completed','failed','cancelled') \
             AND lease_epoch=?3 AND fencing_token=?4)",
            params![child.assignment_id, child.run_id, lease.lease_epoch, lease.fencing_token],
        )
        .map_err(|error| format!("无法释放 lane：{error}"))?;
    // RAISE(IGNORE), AFTER triggers and zero-row updates can all report SQL
    // success without establishing the completion contract. Check only after
    // the final write so a later trigger cannot silently undo an earlier one.
    super::attempts::finish(transaction,lease,child,state)?;
    let closed: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.id=?1 AND r.id=?2 AND r.assignment_id=a.id AND a.coordinator_run_id=?3 \
           AND r.root_run_id=?3 AND r.parent_run_id=?3 AND a.role=?4 AND r.role=?4 AND a.lane=r.lane \
           AND a.target_key=?5 AND r.target_url=?5 AND r.scan_id=?6 AND r.attempt_number=?7 \
           AND a.lease_epoch=?8 AND a.fencing_token=?9 AND a.state=?10 AND r.status='terminal' \
           AND r.terminal_state=?10 AND r.terminal_code=?11 AND r.terminal_reason=?12 \
           AND a.failure_class=?13 AND a.finished_at<>'' AND r.finished_at<>'' \
           AND a.reserved_tokens=0 AND a.reserved_requests=0) \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (child_run_id=?2 OR assignment_id=?1) AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![child.assignment_id,child.run_id,lease.root_run_id,child.role.as_str(),lease.target_key,
            lease.scan_id,lease.attempt_number,lease.lease_epoch,lease.fencing_token,state,code,reason,
            failure], |row| row.get(0),
    ).map_err(|error|format!("child_finish_postcondition:{error}"))?;
    validate_coordinator_lease(transaction, lease)?;
    if !closed || budget()? != expected_budget {
        return Err("child_finish_postcondition".into());
    }
    let visible: i64 = transaction.query_row(
        "SELECT COUNT(DISTINCT event_type) FROM agent_collaboration_events \
         WHERE sequence>?1 AND scan_id=?2 AND attempt_number=?3 AND entity_type=event_type AND ( \
           (event_type='assignment' AND entity_id=?4 AND json_extract(payload_json,'$.state')=?6) \
           OR (event_type='agent_run' AND entity_id=?5 AND json_extract(payload_json,'$.status')='terminal' \
             AND json_extract(payload_json,'$.terminalState')=?6))",
        params![event_floor,lease.scan_id,lease.attempt_number,child.assignment_id,child.run_id,state],
        |row|row.get(0),
    ).map_err(|error|format!("child_finish_event_postcondition:{error}"))?;
    if visible != 2 {
        return Err("child_finish_event_postcondition_failed".into());
    }
    Ok(())
}
