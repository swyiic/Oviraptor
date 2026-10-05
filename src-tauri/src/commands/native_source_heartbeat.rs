// A live Source worker extends the original grant, never claims a replacement.
// Callers verify the exact Source contract and in-flight call before/after this
// shared write boundary. Every deadline is sampled once under their write lock.
fn renew_source_worker_leases(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    expected_tools: &[String],
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{attempts, lease as leases};
    if db.is_autocommit() {
        return Err("source_heartbeat_transaction_required".into());
    }
    leases::validate_coordinator_lease(db, lease)?;
    let original = attempts::require_live_for_run(db, &child.run_id)?;
    let capabilities = || -> Result<Vec<(String, String, String)>, String> {
        let mut q=db.prepare("SELECT id,capability,lease_expires_at FROM agent_capability_leases
            WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4
              AND fencing_token=?5 AND revoked_at='' AND lease_expires_at>datetime('now','localtime') ORDER BY id")
            .map_err(|e|e.to_string())?;
        let rows = q
            .query_map(
                params![
                    child.assignment_id,
                    child.run_id,
                    lease.root_run_id,
                    lease.lease_epoch,
                    lease.fencing_token
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    };
    let before = capabilities()?;
    let mut actual_tools = before
        .iter()
        .map(|(_, name, _)| name.clone())
        .collect::<Vec<_>>();
    actual_tools.sort();
    let mut expected_tools = expected_tools.to_vec();
    expected_tools.sort();
    if actual_tools != expected_tools {
        return Err("source_heartbeat_capability_set_changed".into());
    }
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        db,
        &lease.root_run_id,
    )?;
    owner.require_original_coordinator(db, lease)?;
    let expiry =
        crate::agent_runtime::multi_agent::budget::clock::renewal_expiry(db, &lease.root_run_id)?;
    // A short Root already at its immutable deadline gains no time from a
    // heartbeat. Keep the same live grant without repeatedly updating rows.
    let at_deadline:bool=db.query_row("SELECT
        (SELECT count(*) FROM agent_coordinator_leases WHERE root_run_id=?1 AND lease_expires_at=?4)=1
        AND (SELECT count(*) FROM agent_assignments WHERE id=?2 AND state='running' AND budget_settled_at='' AND lease_expires_at=?4)=1
        AND (SELECT count(*) FROM agent_runs WHERE id IN (?1,?3) AND status IN ('prepared','running') AND cancel_requested_at='' AND lease_expires_at=?4)=2",
        params![lease.root_run_id,child.assignment_id,child.run_id,expiry],|r|r.get(0)).map_err(|e|e.to_string())?;
    if at_deadline
        && original.expires_at == expiry
        && before.iter().all(|(_, _, end)| end == &expiry)
    {
        owner.require_original_coordinator(db, lease)?;
        return Ok(());
    }
    // Every legitimate write below is counted exactly; these lease/heartbeat
    // updates emit no collaboration events. Extra trigger/foreign/business
    // writes cannot be hidden in a successful renewal. Keep an outer writer's
    // authorizer intact rather than replacing its single SQLite callback.
    let prior_writes = db.total_changes();
    let root=db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=?4,heartbeat_at=datetime('now','localtime')
        WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3 AND lease_expires_at>datetime('now','localtime')",
        params![lease.root_run_id,lease.lease_epoch,lease.fencing_token,expiry]).map_err(|e|e.to_string())?;
    let caps=db.execute("UPDATE agent_capability_leases SET lease_expires_at=?6
        WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5
          AND revoked_at='' AND lease_expires_at>datetime('now','localtime')",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token,expiry]).map_err(|e|e.to_string())?;
    let assignment=db.execute("UPDATE agent_assignments SET lease_expires_at=?6
        WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5
          AND state='running' AND budget_settled_at='' AND lease_expires_at>datetime('now','localtime')",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token,expiry]).map_err(|e|e.to_string())?;
    let runs = db
        .execute(
            "UPDATE agent_runs SET lease_expires_at=?3,heartbeat_at=datetime('now','localtime')
        WHERE id IN (?1,?2) AND status IN ('prepared','running') AND cancel_requested_at=''",
            params![lease.root_run_id, child.run_id, expiry],
        )
        .map_err(|e| e.to_string())?;
    if root != 1 || caps != before.len() || assignment != 1 || runs != 2 {
        return Err("source_heartbeat_write_unconfirmed".into());
    }
    attempts::renew(db, lease, child, &expiry)?;
    let mut expected_worker = original;
    expected_worker.expires_at = expiry.clone();
    let expected_caps = before
        .into_iter()
        .map(|(id, name, _)| (id, name, expiry.clone()))
        .collect::<Vec<_>>();
    let uniform:bool=db.query_row("SELECT
        (SELECT count(*) FROM agent_coordinator_leases WHERE root_run_id=?1 AND lease_expires_at=?4)=1
        AND (SELECT count(*) FROM agent_assignments WHERE id=?2 AND lease_expires_at=?4)=1
        AND (SELECT count(*) FROM agent_runs WHERE id IN (?1,?3) AND lease_expires_at=?4)=2
        AND (SELECT count(*) FROM agent_capability_leases WHERE child_run_id=?3 AND revoked_at='')=?5",
        params![lease.root_run_id,child.assignment_id,child.run_id,expiry,expected_caps.len() as i64],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !uniform
        || capabilities()? != expected_caps
        || attempts::require_live_for_run(db, &child.run_id)? != expected_worker
    {
        return Err("source_heartbeat_postcondition".into());
    }
    owner.require_original_coordinator(db, lease)?;
    // C(1) + original capabilities(N) + assignment(1) + runs(2) + worker(1).
    if db.total_changes().checked_sub(prior_writes) != Some(5 + caps as u64) {
        return Err("source_heartbeat_collateral_write".into());
    }
    Ok(())
}
