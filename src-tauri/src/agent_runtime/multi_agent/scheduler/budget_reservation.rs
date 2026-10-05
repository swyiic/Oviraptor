// Projection and append-only source admission share the scheduling transaction.
fn initialize_child_budget(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    super::budget::limits::initialize(transaction, lease)?;
    transaction
        .execute(
            "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) \
             SELECT id,hard_token_budget,hard_request_budget,?1,?2 FROM agent_runs WHERE id=?3 \
             ON CONFLICT(root_run_id) DO UPDATE SET lease_epoch=excluded.lease_epoch,fencing_token=excluded.fencing_token,updated_at=datetime('now','localtime')",
            params![lease.lease_epoch, lease.fencing_token, lease.root_run_id],
        )
        .map_err(|error| format!("无法初始化 child 预算账本：{error}"))?;
    Ok(())
}

fn reserve_child_budget(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &AgentAssignment,
) -> Result<(), String> {
    super::budget::root::require_child_capacity(
        transaction,
        &lease.root_run_id,
        assignment.reserved_tokens,
        assignment.reserved_requests,
    )?;
    {
        let reserved = transaction
            .execute(
                "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+?1,reserved_requests=reserved_requests+?2,\
                 updated_at=datetime('now','localtime') WHERE root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 \
                 AND (total_tokens=0 OR reserved_tokens+spent_tokens+?1<=total_tokens) \
                 AND (total_requests=0 OR reserved_requests+spent_requests+?2<=total_requests)",
                params![
                    assignment.reserved_tokens,
                    assignment.reserved_requests,
                    lease.root_run_id,
                    lease.lease_epoch,
                    lease.fencing_token
                ],
            )
            .map_err(|error| format!("无法预留 child 预算：{error}"))?;
        if reserved != 1 {
            return Err("child_budget_reservation_exceeded_or_stale".into());
        }
    }
    super::budget::model::reserve(
        transaction,
        lease,
        &assignment.id,
        assignment.reserved_tokens,
        assignment.reserved_requests,
    )?;
    super::budget::limits::reserve_slot(transaction, lease, &assignment.id)?;
    super::budget::root::require_child_capacity(transaction, &lease.root_run_id, 0, 0)?;
    Ok(())
}
