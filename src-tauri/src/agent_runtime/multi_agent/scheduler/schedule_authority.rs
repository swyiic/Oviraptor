// Shared postcondition for fresh grants and read-only idempotent lookups.
// No historical authority is created here. A broken or expired grant needs
// explicit reconciliation/another attempt, never an implicit scheduler repair.
fn verify_scheduled_authority(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    expected: &AgentAssignment,
) -> Result<ScheduledChild, String> {
    verify_scheduled_authority_on(tx, lease, expected, &|_| {
        super::budget::limits::initialize(tx, lease)?;
        super::budget::admission::require_determinate(tx, &lease.root_run_id)
    })
}

// Pure authority inspection, including during the caller's own live Root SDK.
// Fresh-work financial admission remains with the caller and the paid SDK gate.
pub(crate) fn verify_original_scheduled_child(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    super::budget::root::RootOwner::load_original(db, &lease.root_run_id)?
        .require_original_coordinator(db, lease)?;
    let assignment = super::assignment::load_assignment(db, &child.assignment_id)?
        .ok_or("scheduled_authority_missing")?;
    if assignment.child_run_id != child.run_id || assignment.role != child.role {
        return Err("scheduled_authority_original_child_conflict".into());
    }
    let checked = verify_scheduled_authority_on(db, lease, &assignment, &|db| {
        super::budget::limits::verify_root_contract(db, &lease.root_run_id)
    })?;
    if checked != *child {
        return Err("scheduled_authority_original_child_conflict".into());
    }
    Ok(())
}

fn verify_scheduled_authority_on(
    tx: &Connection,
    lease: &CoordinatorLease,
    expected: &AgentAssignment,
    check_limits: &dyn Fn(&Connection) -> Result<(), String>,
) -> Result<ScheduledChild, String> {
    validate_coordinator_lease(tx, lease)?;
    require_executable_coordinator(tx, lease)?;
    let actual = super::assignment::load_assignment(tx, &expected.id)?
        .ok_or("scheduled_authority_missing")?;
    if actual.id != expected.id || actual.content_key() != expected.content_key() {
        return Err("assignment_dedup_conflict".into());
    }
    if actual.child_run_id != expected.child_run_id {
        lineage::verify(tx, lease, &actual)?;
        let mut replacement = expected.clone();
        replacement.child_run_id = actual.child_run_id.clone();
        return verify_scheduled_authority_on(tx, lease, &replacement, check_limits);
    }
    let worker = super::attempts::current(tx, lease, &actual.id)?;
    if worker.lease_epoch > 1 {
        lineage::verify(tx, lease, &actual)?;
    }
    check_limits(tx)?;
    let key = super::contract_owner::schedule_contract_key(
        lease.attempt_number,
        &lease.target_key,
        expected.role.as_str(),
        &expected.trigger_code,
        expected.evidence_revision,
    );
    super::contract_owner::verify_held_owner(
        tx,
        &lease.root_run_id,
        &key,
        &expected.id,
        lease.lease_epoch,
        &lease.fencing_token,
    )?;
    let intact: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a
         JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id
         JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id
           AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane
         JOIN agent_contract_owners o ON o.assignment_id=a.id AND o.root_run_id=a.coordinator_run_id
           AND o.contract_key=?10 AND o.state='held' AND o.lease_epoch=a.lease_epoch
           AND o.fencing_token=a.fencing_token
         JOIN agent_budget_ledger b ON b.root_run_id=a.coordinator_run_id
           AND b.lease_epoch=a.lease_epoch AND b.fencing_token=a.fencing_token
         WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3
           AND r.parent_run_id=?3 AND a.role=?4 AND r.role=?4 AND a.lane=?5 AND r.lane=?5
           AND a.target_key=?6 AND r.target_url=?6 AND r.scan_id=?7 AND r.attempt_number=?8
           AND a.lease_epoch=?9 AND a.fencing_token=?11 AND r.backend='native'
           AND r.orchestration_policy='multi' AND r.cancel_requested_at=''
           AND a.lease_expires_at>datetime('now','localtime')
           AND r.lease_expires_at>datetime('now','localtime')
           AND ((a.state='leased' AND r.status='prepared')
             OR (a.state IN ('running','waiting_review') AND r.status='running')))",
            params![
                expected.id,
                expected.child_run_id,
                lease.root_run_id,
                expected.role.as_str(),
                expected.lane.as_str(),
                lease.target_key,
                lease.scan_id,
                lease.attempt_number,
                lease.lease_epoch,
                key,
                lease.fencing_token
            ],
            |r| r.get(0),
        )
        .map_err(|e| format!("scheduled_authority_read:{e}"))?;
    let run = store::load_run(tx, &expected.child_run_id)?.ok_or("scheduled_authority_missing")?;
    super::attempts::require_live_for_run(tx, &expected.child_run_id)?;
    let mut capabilities = tx
        .prepare(
            "SELECT capability,root_run_id,child_run_id,lease_epoch,fencing_token,
         revoked_at='',lease_expires_at>datetime('now','localtime')
         FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 ORDER BY capability",
        )
        .map_err(|e| e.to_string())?;
    let rows = capabilities
        .query_map(params![expected.id, expected.child_run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut names = expected.capability_lease.clone();
    names.sort();
    let exact_caps = rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>() == names
        && rows.iter().all(|r| {
            r.1 == lease.root_run_id
                && r.2 == expected.child_run_id
                && r.3 == lease.lease_epoch
                && r.4 == lease.fencing_token
                && r.5
                && r.6
        });
    if !intact || !exact_caps || run.capability_lease != expected.capability_lease {
        return Err("scheduled_authority_incomplete_or_expired".into());
    }
    Ok(ScheduledChild {
        assignment_id: expected.id.clone(),
        run_id: expected.child_run_id.clone(),
        role: expected.role,
    })
}

fn verify_fresh_grant_deadline(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    let uniform: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id
           AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=a.target_key
         WHERE a.id=?1 AND r.id=?2 AND c.root_run_id=?3
           AND a.lease_expires_at=?4 AND r.lease_expires_at=?4 AND c.lease_expires_at=?4)
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND lease_expires_at<>?4)
         AND EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE assignment_id=?1 AND child_run_id=?2 AND expires_at=?4)",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_expires_at], |r|r.get(0),
    ).map_err(|e|format!("scheduled_deadline_read:{e}"))?;
    if !uniform {
        return Err("scheduled_grant_deadline_conflict".into());
    }
    Ok(())
}
