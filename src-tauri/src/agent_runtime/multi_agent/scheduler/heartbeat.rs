#[derive(Clone, PartialEq, Eq)]
struct ExecutorLeaseAuthority {
    scan_id: String,
    attempt: i64,
    target: String,
    root: String,
    assignment: String,
    epoch: i64,
    fencing: String,
}

/// A live executor can spend longer than the initial 600-second lease in a
/// local model generation. Refresh all matching leases as one fenced
/// transaction. An expired or taken-over coordinator cannot be resurrected by
/// its old worker; the lane remains occupied until an explicit terminal step.
pub fn refresh_running_executor_leases(
    connection: &Connection,
    child_run_id: &str,
) -> Result<(), String> {
    refresh_running_executor_leases_authorized(connection, child_run_id, |_| Ok(()))
}

/// Restart admission and lease writes share one lock. The existing live
/// heartbeat may retain its own authority while a model is in flight.
pub(crate) fn refresh_running_executor_leases_authorized(
    connection: &Connection,
    child_run_id: &str,
    authorize: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 executor 续租：{error}"))?;
    let read_owner = || -> Result<Option<ExecutorLeaseAuthority>, String> {
        transaction
        .query_row(
            "SELECT c.scan_id,c.attempt_number,c.target_key,c.root_run_id,a.id,c.lease_epoch,c.fencing_token \
             FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id \
             JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id \
               AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url \
             JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id \
               AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane \
             WHERE r.id=?1 AND r.status='running' AND r.backend='native' AND r.cancel_requested_at='' \
               AND r.lease_expires_at>datetime('now','localtime') AND r.role='web_executor' AND r.lane='target_touching' \
               AND r.orchestration_policy='multi' AND a.state='running' AND a.role=r.role AND a.lane=r.lane \
               AND a.target_key=r.target_url AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token \
               AND c.lease_expires_at>datetime('now','localtime') \
               AND a.lease_expires_at>datetime('now','localtime') \
               AND EXISTS(SELECT 1 FROM agent_capability_leases p WHERE p.assignment_id=a.id \
                 AND p.child_run_id=r.id AND p.root_run_id=c.root_run_id AND p.revoked_at='' \
                 AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token \
                 AND p.lease_expires_at>datetime('now','localtime'))",
            [child_run_id],
            |row| Ok(ExecutorLeaseAuthority{scan_id:row.get(0)?,attempt:row.get(1)?,target:row.get(2)?,
                root:row.get(3)?,assignment:row.get(4)?,epoch:row.get(5)?,fencing:row.get(6)?}),
        )
        .optional()
        .map_err(|error| format!("无法检查 executor 续租权限：{error}"))
    };
    let owner = read_owner()?;
    let expected_owner = owner.clone();
    let Some(ExecutorLeaseAuthority {
        scan_id,
        attempt,
        target,
        root,
        assignment,
        epoch,
        fencing,
    }) = owner
    else {
        return Err("executor_lease_expired_or_fenced".into());
    };
    require_active_attempt(&transaction, &scan_id, attempt)?;
    require_open_coordinator(&transaction, &scan_id, attempt, &target, &root)?;
    let worker=super::attempts::require_live_for_run(&transaction,child_run_id)?;
    authorize(&transaction)?;
    let expiry: String = transaction
        .query_row(
            "SELECT datetime('now','+600 seconds','localtime')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("executor_heartbeat_clock:{error}"))?;
    let capabilities = || -> Result<Vec<(String, String, String)>, String> {
        let mut query=transaction.prepare("SELECT id,capability,lease_expires_at FROM agent_capability_leases
            WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5
            AND revoked_at='' AND lease_expires_at>datetime('now','localtime') ORDER BY id").map_err(|error|error.to_string())?;
        let rows = query
            .query_map(
                params![assignment, child_run_id, root, epoch, fencing],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    };
    let expected_capabilities: Vec<_> = capabilities()?
        .into_iter()
        .map(|(id, capability, _)| (id, capability, expiry.clone()))
        .collect();
    let renewed = transaction
        .execute(
            "UPDATE agent_coordinator_leases SET lease_expires_at=?7,\
             heartbeat_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 \
             AND lease_epoch=?5 AND fencing_token=?6 AND lease_expires_at>datetime('now','localtime')",
            params![scan_id, attempt, target, root, epoch, fencing,expiry],
        )
        .map_err(|error| format!("无法续租 Coordinator：{error}"))?;
    if renewed != 1 {
        return Err("executor_lease_expired_or_fenced".into());
    }
    transaction.execute(
        "UPDATE agent_assignments SET lease_expires_at=?5,\
         updated_at=datetime('now','localtime') WHERE id=?1 AND child_run_id=?2 AND state='running' \
         AND lease_epoch=?3 AND fencing_token=?4",
        params![assignment, child_run_id, epoch, fencing,expiry],
    ).map_err(|error| format!("无法续租 assignment：{error}"))?;
    transaction.execute(
        "UPDATE agent_capability_leases SET lease_expires_at=?5 \
         WHERE assignment_id=?1 AND child_run_id=?2 AND revoked_at='' AND lease_epoch=?3 AND fencing_token=?4 \
         AND lease_expires_at>datetime('now','localtime')",
        params![assignment, child_run_id, epoch, fencing,expiry],
    ).map_err(|error| format!("无法续租 capability：{error}"))?;
    transaction.execute(
        "UPDATE agent_runs SET lease_expires_at=?2,\
         heartbeat_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status='running'",
        params![child_run_id,expiry],
    ).map_err(|error| format!("无法续租 child run：{error}"))?;
    let child=ScheduledChild {assignment_id:assignment.clone(),run_id:child_run_id.into(),role:AgentRole::WebExecutor};
    let current_lease=CoordinatorLease {scan_id:scan_id.clone(),attempt_number:attempt,target_key:target.clone(),
        root_run_id:root.clone(),lease_epoch:epoch,fencing_token:fencing.clone(),lease_expires_at:expiry.clone()};
    super::attempts::renew(&transaction,&current_lease,&child,&expiry)?;
    authorize(&transaction)?;
    require_active_attempt(&transaction, &scan_id, attempt)?;
    require_open_coordinator(&transaction, &scan_id, attempt, &target, &root)?;
    let complete:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases c
        JOIN agent_assignments a ON a.coordinator_run_id=c.root_run_id JOIN agent_runs r ON r.id=a.child_run_id
        WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.target_key=?3 AND c.root_run_id=?4
        AND a.id=?5 AND r.id=?6 AND c.lease_epoch=?7 AND c.fencing_token=?8
        AND c.lease_expires_at=?9 AND a.lease_expires_at=?9 AND r.lease_expires_at=?9)",
        params![scan_id,attempt,target,root,assignment,child_run_id,epoch,fencing,expiry],|row|row.get(0))
        .map_err(|error|format!("executor_heartbeat_postcondition:{error}"))?;
    let mut expected_worker=worker;expected_worker.expires_at=expiry.clone();
    if read_owner()? != expected_owner || capabilities()? != expected_capabilities || !complete
        || super::attempts::require_live_for_run(&transaction,child_run_id)?!=expected_worker {
        return Err("executor_heartbeat_postcondition".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交 executor 续租：{error}"))
}
