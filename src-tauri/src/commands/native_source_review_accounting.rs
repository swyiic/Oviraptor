// A pending Reviewer is the only unsettled work at this boundary. Its saved
// receipt is not yet spent: delivery must settle it exactly once. Refuse a
// mismatched ledger BEFORE activation, local delivery or error-path closure.
fn source_review_pending_accounting(
    connection:&rusqlite::Connection,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    phases:&crate::agent_runtime::multi_agent::source_phases::PhaseProof,
    received:bool,
)->Result<(),String> {
    use crate::agent_runtime::multi_agent::{source_review_subject::{self,Subject},source_reviewer};
    let subject=source_review_subject::for_child(connection,lease,child)?;
    let (prior_count,prior_tokens,prior_requests)=if subject==Subject::Coverage {
        match source_reviewer::progress(connection,lease)? {
            source_reviewer::ReviewProgress::Delivered(audit)=>(1,audit.usage.total_tokens,audit.usage.model_requests),
            source_reviewer::ReviewProgress::NotStarted if source_reviewer::task_slice(connection,lease)?.is_none()=>(0,0,0),
            _=>return Err("source_coverage_candidate_review_not_ready".into()),
        }
    } else {(0,0,0)};
    let spent_tokens=phases.tokens.checked_add(prior_tokens).ok_or("source_completion_usage_overflow")?;
    let spent_requests=phases.requests.checked_add(prior_requests).ok_or("source_completion_usage_overflow")?;
    let valid:bool=connection.query_row("SELECT EXISTS(
        SELECT 1 FROM agent_budget_ledger b JOIN agent_runs root ON root.id=b.root_run_id
        JOIN agent_assignments a ON a.coordinator_run_id=b.root_run_id JOIN agent_runs r ON r.id=a.child_run_id
        WHERE b.root_run_id=?1 AND a.id=?2 AND r.id=?3 AND b.lease_epoch=?4 AND b.fencing_token=?5
        AND b.spent_tokens=?6 AND b.spent_requests=?7 AND b.reserved_tokens=a.reserved_tokens
        AND b.reserved_requests=1 AND a.reserved_requests=1 AND a.reserved_tokens>0
        AND r.reserved_tokens=a.reserved_tokens AND r.reserved_requests=a.reserved_requests
        AND b.total_tokens=root.hard_token_budget AND b.total_requests=root.hard_request_budget
        AND root.reserved_tokens=0 AND root.reserved_requests=0)
        AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=5+?9
        AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('source_tool_result','evidence_summary'))=4
        AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?2)
        AND (?9=1 OR NOT EXISTS(SELECT 1 FROM agent_source_review_decisions WHERE root_run_id=?1))
        AND NOT EXISTS(SELECT 1 FROM agent_source_coverage_decisions WHERE root_run_id=?1)
        AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=2+?8+?9
        AND (SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1)=?10-2",
        params![lease.root_run_id,child.assignment_id,child.run_id,lease.lease_epoch,lease.fencing_token,spent_tokens,spent_requests,i64::from(received),prior_count,phases.requests],
        |r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {return Err("source_review_pending_ledger_or_extra_work_mismatch".into());}
    Ok(())
}
