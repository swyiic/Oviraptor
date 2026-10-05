//! First-worker Web allocation proof; no mutable task can grow an original invoice.
use crate::agent_runtime::multi_agent::{attempts, lease::CoordinatorLease};
use rusqlite::{params, Connection};
pub(crate) fn original_web_grant(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(i64, i64), String> {
    super::mapper_allocation::require_original_invoice_guards(db)?;
    super::super::root::RootOwner::load_original(db, &lease.root_run_id)?
        .require_original_coordinator(db, lease)?;
    let worker = attempts::current(db, lease, assignment)?;
    if worker.lease_epoch != 1 {
        return Err("executor_allocation_original_worker_conflict".into());
    }
    let hard: (i64, i64) = db
        .query_row(
            "SELECT hard_token_budget,hard_request_budget FROM agent_runs WHERE id=?1",
            [&lease.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let projection:(i64,i64,String,String)=db.query_row("SELECT reserved_tokens,reserved_requests,state,budget_settled_at FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='web_executor'",params![assignment,lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"executor_allocation_original_grant_conflict")?;
    let source = format!("assignment:{assignment}");
    let mut q=db.prepare("SELECT dimension,amount,idempotency_key FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND source_id=?4 AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') ORDER BY dimension").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(
            params![lease.root_run_id, assignment, worker.id, source],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let amount = |dimension: &str, limit: i64| -> Result<i64, String> {
        let found = rows.iter().find(|r| r.0 == dimension);
        match (limit, found) {
            (0, None) => Ok(0),
            (n, Some(row)) if n > 0 && row.1 > 0 && row.1 <= n => Ok(row.1),
            _ => Err("executor_allocation_original_grant_conflict".into()),
        }
    };
    let tokens = amount("model_input_tokens", hard.0)?;
    let requests = amount("model_requests", hard.1)?;
    let count = usize::from(tokens > 0) * 3 + usize::from(requests > 0);
    if rows.len() != count
        || rows.iter().any(|(dimension, n, key)| {
            *n != if dimension == "model_requests" {
                requests
            } else {
                tokens
            } || key != &format!("reserve:{assignment}:{dimension}")
        })
        || projection.0 != tokens
        || projection.1 != requests
        || !matches!(
            projection.2.as_str(),
            "leased" | "running" | "waiting_review"
        )
        || !projection.3.is_empty()
    {
        return Err("executor_allocation_original_grant_conflict".into());
    }
    Ok((tokens, requests))
}
