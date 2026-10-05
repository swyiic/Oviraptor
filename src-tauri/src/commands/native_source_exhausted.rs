// Exhausted known tool rounds are failed work, not unknown provider usage.
fn close_exhausted_source_tool_assignment(
    db: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{scheduler, source_rounds};
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // Paid local closure still requires original current authority, not new IO.
    authorize_source_tool_phase_with_deadline(&tx, context, actor, child, false)?;
    let _idle = source_rounds::require_idle_for_root(&tx, actor)?;
    let usage = source_rounds::audit_exhausted(&tx, actor, child)?;
    let foreign: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2)
        OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE assignment_id=?1 OR child_run_id=?2)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)",
        params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("source_exhausted_foreign_execution_history".into());
    }
    let ledger = || -> Result<[i64; 6], String> {
        tx.query_row("SELECT total_tokens,total_requests,reserved_tokens,reserved_requests,spent_tokens,spent_requests
            FROM agent_budget_ledger WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3",
            params![actor.root_run_id,actor.lease_epoch,actor.fencing_token],
            |r| Ok([r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?])).map_err(|e|e.to_string())
    };
    let (tokens, requests): (i64, i64) = tx
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_assignments
        WHERE id=?1 AND child_run_id=?2 AND budget_settled_at='' AND state='running'",
            params![child.assignment_id, child.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let mut expected = ledger()?;
    for (index, amount) in [(2, tokens), (3, requests)] {
        expected[index] = expected[index]
            .checked_sub(amount)
            .filter(|n| *n >= 0)
            .ok_or("source_exhausted_budget_invalid")?;
    }
    for (index, amount) in [(4, usage.total_tokens), (5, usage.model_requests)] {
        expected[index] = expected[index]
            .checked_add(amount)
            .ok_or("source_exhausted_budget_invalid")?;
    }
    settle_child_usage_in_transaction(&tx, actor, child, &usage)?;
    scheduler::finish_child_in_transaction(
        &tx,
        actor,
        child,
        false,
        "source_tool_phase_round_budget_exhausted_without_finish",
    )?;
    authorize_source_specialist(&tx, context, actor)?;
    if source_rounds::audit_exhausted(&tx, actor, child)? != usage || ledger()? != expected {
        return Err("source_exhausted_closure_changed".into());
    }
    let exact: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state='failed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0
        AND r.status='terminal' AND r.terminal_state='failed' AND r.used_tokens=?3 AND r.used_requests=?4 AND r.used_cached_tokens=?5)
        AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at='')",
        params![child.assignment_id,child.run_id,usage.total_tokens,usage.model_requests,usage.cached_input_tokens],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        return Err("source_exhausted_closure_unconfirmed".into());
    }
    tx.commit()
        .map_err(|e| format!("source_exhausted_commit_unconfirmed:{e}"))
}
