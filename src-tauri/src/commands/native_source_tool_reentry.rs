// Admit only a live source-tools child with a durable model receipt and local
// SourceBroker calls still pending. Never adopt an executing/uncertain model
// request, rotate an expired fence, or infer usage from a completed label.
struct SourceToolPending {
    child: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    slice: JsonValue,
    next_index: usize,
    settled_finish: bool,
}

fn source_tool_received_reentry(
    tx: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    completed: i64,
    expired_finish_only: bool,
) -> Result<(Vec<JsonValue>, SourceToolPending), String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::{source, source_phases, source_rounds}};
    if tx.is_autocommit() || !matches!(completed, 2 | 3) {
        return Err("source_tool_reentry_context_invalid".into());
    }
    let prefix = source_phases::audit_completed_tool_prefix(tx, lease, completed as usize)?;
    let role = if completed == 2 { AgentRole::RepoMapper } else { AgentRole::SourceAnalyst };
    let (assignment_id, run_id, reserved_tokens, reserved_requests, slice, revision): (String, String, i64, i64, String, i64) = tx.query_row(
        "SELECT a.id,r.id,a.reserved_tokens,a.reserved_requests,a.task_slice_json,a.evidence_revision
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id
         WHERE a.coordinator_run_id=?1 AND a.role=?2 AND a.trigger_code='source_tools_ready'
         AND a.lane='read_only_analysis' AND a.target_key=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5
         AND a.state='running' AND a.budget_settled_at='' AND a.reserved_tokens>0 AND a.reserved_requests=3
         AND r.root_run_id=?1 AND r.parent_run_id=?1 AND r.role=a.role AND r.lane=a.lane
         AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?3 AND r.status='running'
         AND r.cancel_requested_at='' AND r.used_tokens=0 AND r.used_requests=0",
        params![lease.root_run_id, role.as_str(), lease.target_key, lease.lease_epoch,
            lease.fencing_token, lease.scan_id, lease.attempt_number],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).map_err(|_| "source_tool_reentry_child_binding_invalid")?;
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id, run_id, role,
    };
    if expired_finish_only {
        authorize_source_tool_saved_finish(tx, context, lease, &child)?;
    } else {
        authorize_source_tool_phase(tx, context, lease, &child)?;
    }
    let slice: JsonValue = serde_json::from_str(&slice).map_err(|_| "source_tool_reentry_slice_invalid")?;
    source::validate_scheduled_slice(tx, lease, role, &slice, revision, &source::tool_capabilities(role)?)?;
    if slice["phase"] != "source_tools" { return Err("source_tool_reentry_phase_invalid".into()); }
    let round = source_rounds::audit_recoverable_boundary(tx, lease, &child)?;
    if round.rounds > reserved_requests || round.usage.model_requests != round.rounds
        || round.usage.total_tokens > reserved_tokens {
        return Err("source_tool_reentry_reserved_usage_invalid".into());
    }
    let clean: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs root ON root.id=b.root_run_id
            WHERE b.root_run_id=?1 AND b.lease_epoch=?3 AND b.fencing_token=?4
            AND b.total_tokens=root.hard_token_budget AND b.total_requests=root.hard_request_budget
            AND b.spent_tokens=?5 AND b.spent_requests=?6
            AND b.reserved_tokens=?7 AND b.reserved_requests=?8
            AND root.reserved_tokens=0 AND root.reserved_requests=0)
         AND (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1)=?9
         AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=?9
         AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=2
         AND (SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1)=?10
         AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1
              AND kind IN ('evidence_summary','source_tool_result'))=?11
         AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?2)
         AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1
              AND assignment_id<>?2 AND revoked_at='')
         AND (SELECT count(*) FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id
              WHERE a.coordinator_run_id=?1)=1
         AND EXISTS(SELECT 1 FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id
              WHERE l.assignment_id=?2 AND l.scan_id=?12 AND l.attempt_number=?13
              AND l.target_key=?14 AND l.lane=a.lane)
         AND NOT EXISTS(SELECT 1 FROM agent_source_review_decisions WHERE root_run_id=?1)
         AND NOT EXISTS(SELECT 1 FROM agent_source_coverage_decisions WHERE root_run_id=?1)",
        params![lease.root_run_id, child.assignment_id, lease.lease_epoch, lease.fencing_token,
            prefix.tokens, prefix.requests, reserved_tokens, reserved_requests, completed + 1,
            prefix.requests - 2 + round.rounds, completed, lease.scan_id, lease.attempt_number, lease.target_key],
        |row| row.get(0),
    ).map_err(|e| format!("source_tool_reentry_accounting:{e}"))?;
    if !clean { return Err("source_tool_reentry_accounting_invalid".into()); }
    if expired_finish_only && !round.settled_finish
        && source_rounds::audit_pending_finish(tx, lease, &child)?.is_none() {
        return Err("source_model_deadline_exceeded".into());
    }
    Ok((prefix.assessments, SourceToolPending {
        child, slice, next_index: completed as usize - 1,
        settled_finish: round.settled_finish,
    }))
}

fn settle_expired_source_finish(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    pending: &SourceToolPending,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::source_rounds;
    let tx = rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    authorize_source_tool_saved_finish(&tx, context, lease, &pending.child)?;
    let call = if pending.settled_finish {
        source_rounds::audit_settled_finish(&tx, lease, &pending.child)?
            .ok_or("source_round_reentry_finish_changed")?
    } else {
        source_rounds::audit_pending_finish(&tx, lease, &pending.child)?
            .ok_or("source_model_deadline_exceeded")?
    };
    drop(tx);
    if !pending.settled_finish {
        source_rounds::execute_tool(connection, &call, 0,
            |db| {
                authorize_source_tool_saved_finish(db, context, lease, &pending.child)?;
                source_rounds::audit_saved_finish_transition(db, lease, &pending.child)?;
                Ok(())
            },
            |db, name, args| {
                if name != "assignment.finish" { return Err("source_tool_expired_nonterminal_denied".into()); }
                source_tool_broker_call(db, lease, &pending.child, name, args)
            },
        )?;
    }
    complete_source_tool_assignment_with_deadline(connection, context, lease, &pending.child, &call, false)?;
    Ok(())
}
