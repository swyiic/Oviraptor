fn child_grant_row(lease: &CoordinatorLease, assignment: &AgentAssignment) -> AgentRunRow {
    let mut row = AgentRunRow::new(
        assignment.child_run_id.clone(),
        lease.scan_id.clone(),
        lease.attempt_number,
        lease.target_key.clone(),
        AgentBackendKind::Native,
        assignment.role,
        format!(
            "child:{}:{}",
            assignment.trigger_code, assignment.evidence_revision
        ),
        "",
    );
    row.parent_run_id = Some(lease.root_run_id.clone());
    row.root_run_id = lease.root_run_id.clone();
    row.assignment_id = assignment.id.clone();
    row.lane = Some(assignment.lane);
    row.orchestration_policy = MultiAgentPolicy::Multi;
    row.capability_lease = assignment.capability_lease.clone();
    row.reserved_tokens = assignment.reserved_tokens;
    row.reserved_requests = assignment.reserved_requests;
    row.lease_expires_at = lease.lease_expires_at.clone();
    row.status = AgentRunStatus::Prepared;
    row
}

fn create_child_grant(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &AgentAssignment,
) -> Result<(), String> {
    let row = child_grant_row(lease, assignment);
    store::create_run(transaction, &row)?;
    for capability in &assignment.capability_lease {
        transaction
            .execute(
                "INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(assignment_id,child_run_id,capability) DO NOTHING",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    lease.root_run_id,
                    assignment.id,
                    assignment.child_run_id,
                    capability,
                    lease.lease_epoch,
                    lease.fencing_token,
                    lease.lease_expires_at
                ],
            )
            .map_err(|error| format!("无法签发 capability lease：{error}"))?;
    }
    Ok(())
}
