// Pure proof for closing a recovered Root whose original tool child failed.
// Discover the actual failed child: a resumed Mapper can finish and a later
// Analyst can exhaust. The saved pending child is not evidence of failure.
fn audit_recovered_source_exhausted_root(
    db: &rusqlite::Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler::ScheduledChild, source_phases, source_rounds},
    };
    if db.is_autocommit() {
        return Err("source_recovered_exhausted_transaction_required".into());
    }
    let rows = db.prepare("SELECT a.id,r.id,a.role FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.coordinator_run_id=?1 AND a.state='failed' AND a.trigger_code='source_tools_ready'
        AND a.failure_class='child_execution_failed' AND a.budget_settled_at<>'' AND a.finished_at<>''
        AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.lane='read_only_analysis'
        AND a.target_key=?2 AND a.lease_epoch=?3 AND a.fencing_token=?4
        AND r.assignment_id=a.id AND r.root_run_id=?1 AND r.parent_run_id=?1
        AND r.role=a.role AND r.lane=a.lane AND r.scan_id=?5 AND r.attempt_number=?6 AND r.target_url=?2
        AND r.backend='native' AND r.status='terminal' AND r.terminal_state='failed'
        AND r.terminal_code='child_failed' AND r.terminal_reason='source_tool_phase_round_budget_exhausted_without_finish'
        AND r.cancel_requested_at='' AND r.finished_at<>''")
        .map_err(|e|e.to_string())?.query_map(params![c.root_run_id,c.target_key,c.lease_epoch,c.fencing_token,c.scan_id,c.attempt_number],
            |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))
        .map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let [(assignment_id, run_id, role)] = rows.as_slice() else {
        return Err("source_recovered_exhausted_original_child_required".into());
    };
    let (role, completed) = match role.as_str() {
        "repo_mapper" => (AgentRole::RepoMapper, 2_i64),
        "source_analyst" => (AgentRole::SourceAnalyst, 3_i64),
        _ => return Err("source_recovered_exhausted_role_invalid".into()),
    };
    let child = ScheduledChild {
        assignment_id: assignment_id.clone(),
        run_id: run_id.clone(),
        role,
    };
    let worker = crate::agent_runtime::multi_agent::attempts::current(db, c, &child.assignment_id)?;
    if worker.child_run_id != child.run_id
        || worker.state != "failed"
        || worker.finished_at.is_empty()
        || worker.failure_class != "child_execution_failed"
    {
        return Err("source_recovered_exhausted_original_worker_required".into());
    }
    let usage = source_rounds::audit_exhausted(db, c, &child)?;
    let prefix = source_phases::audit_completed_tool_prefix(db, c, completed as usize)?;
    let tokens = prefix
        .tokens
        .checked_add(usage.total_tokens)
        .ok_or("source_recovered_exhausted_usage_overflow")?;
    let requests = prefix
        .requests
        .checked_add(usage.model_requests)
        .ok_or("source_recovered_exhausted_usage_overflow")?;
    let clean:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs root ON root.id=b.root_run_id
        WHERE b.root_run_id=?1 AND b.lease_epoch=?3 AND b.fencing_token=?4
        AND b.total_tokens=root.hard_token_budget AND b.total_requests=root.hard_request_budget
        AND b.spent_tokens=?5 AND b.spent_requests=?6 AND b.reserved_tokens=0 AND b.reserved_requests=0
        AND root.reserved_tokens=0 AND root.reserved_requests=0)
        AND EXISTS(SELECT 1 FROM agent_runs WHERE id=?7 AND used_tokens=?8 AND used_requests=?9 AND used_cached_tokens=?10)
        AND (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1)=?11
        AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=?11
        AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=2
        AND (SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1)=?12
        AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1)=?13
        AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE coordinator_run_id=?1))
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
        AND NOT EXISTS(SELECT 1 FROM agent_source_review_decisions WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_source_coverage_decisions WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_root_tick_receipts WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id IN (SELECT id FROM agent_runs WHERE root_run_id=?1))",
        params![c.root_run_id,child.assignment_id,c.lease_epoch,c.fencing_token,tokens,requests,child.run_id,
            usage.total_tokens,usage.model_requests,usage.cached_input_tokens,completed+1,prefix.requests-2+3,completed],
        |r|r.get(0)).map_err(|e|format!("source_recovered_exhausted_accounting:{e}"))?;
    if !clean {
        return Err("source_recovered_exhausted_accounting_invalid".into());
    }
    budget::admission::require_settled_for_completion(db, &c.root_run_id)?;
    Ok(child)
}

// The label selects a stricter original audit, not execution or exit authority.
// Keep this ahead of tool continuation: a settled failed child cannot resume.
fn finish_pending_source_exhausted_root(
    db: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<bool, String> {
    let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.coordinator_run_id=?1 AND a.state='failed' AND a.trigger_code='source_tools_ready'
        AND r.status='terminal' AND r.terminal_state='failed' AND r.terminal_code='child_failed'
        AND r.terminal_reason='source_tool_phase_round_budget_exhausted_without_finish')",
        [&c.root_run_id],|r|r.get(0)).map_err(|e|format!("source_exhausted_pending_lookup:{e}"))?;
    if !pending {
        return Ok(false);
    }
    #[cfg(test)]
    source_failure_cutover_for_test();
    finish_owned_source_failure(
        db,
        context,
        c,
        &AgentTargetOutcome::incomplete("源码多智能体执行未完成，子任务回执和未决预算保留待核对"),
        true,
    ).map_err(|close|format!("source_tool_phase_round_budget_exhausted_without_finish;source_coordinator_close:{close}"))?;
    Ok(true)
}
