#[allow(clippy::too_many_arguments)]
pub fn schedule_child(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    lane: AgentLane,
    trigger_code: &str,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<ScheduledChild, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定多智能体调度：{error}"))?;
    let child = schedule_child_in_transaction(
        &transaction,
        lease,
        role,
        lane,
        trigger_code,
        task_slice,
        evidence_revision,
        capabilities,
        reserved_tokens,
        reserved_requests,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交多智能体调度：{error}"))?;
    Ok(child)
}

/// The caller commits the assignment with its source instruction and outbox.
/// Never accept a plain Connection here: a partial handoff must roll back the
/// child, lane, capabilities and budget together with the directive state.
#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_child_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    role: AgentRole,
    lane: AgentLane,
    trigger_code: &str,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<ScheduledChild, String> {
    let connection: &Connection = transaction;
    validate_coordinator_lease(connection, lease)?;
    super::source::validate_surface_role(connection, lease, role)?;
    if role == AgentRole::Coordinator {
        return Err("child_role_must_not_be_coordinator".into());
    }
    // A caller cannot grant target tools to a review/read-only child simply
    // by choosing another lane or passing an arbitrary capability string.
    if (role == AgentRole::EvidenceReviewer) != (lane == AgentLane::Review) {
        return Err("reviewer_requires_exclusive_review_lane".into());
    }
    if matches!(
        role,
        AgentRole::SpaApiMapper
            | AgentRole::IdentitySession
            | AgentRole::RepoMapper
            | AgentRole::SourceAnalyst
    ) && lane != AgentLane::ReadOnlyAnalysis
    {
        return Err("analysis_specialist_requires_read_only_lane".into());
    }
    if role == AgentRole::DeepInvestigator && lane != AgentLane::ReadOnlyAnalysis {
        return Err("deep_investigator_requires_read_only_lane".into());
    }
    if role == AgentRole::WebExecutor && lane != AgentLane::TargetTouching {
        return Err("web_executor_requires_target_lane".into());
    }
    if role == AgentRole::Authorization && lane != AgentLane::TargetTouching {
        return Err("authorization_requires_target_lane".into());
    }
    if role == AgentRole::ExternalSurface && lane != AgentLane::TargetTouching {
        return Err("external_surface_requires_target_lane".into());
    }
    let allowed: Option<&[&str]> = match role {
        AgentRole::EvidenceReviewer => Some(&["evidence.read", "review.write"][..]),
        AgentRole::SpaApiMapper => Some(&["evidence.read", "mailbox.read", "mailbox.write"][..]),
        AgentRole::IdentitySession => Some(&["evidence.read", "mailbox.read", "mailbox.write"][..]),
        // Source validates the complete frozen phase/capability contract below.
        AgentRole::RepoMapper | AgentRole::SourceAnalyst => None,
        AgentRole::DeepInvestigator => {
            Some(&["evidence.read", "mailbox.read", "mailbox.write"][..])
        }
        AgentRole::WebExecutor => {
            // The executor's tool set is additionally checked per invocation by
            // the Broker against the exact lease and current fencing token.
            None
        }
        AgentRole::Authorization => Some(&["authorization_probe"][..]),
        AgentRole::ExternalSurface => {
            Some(&["public_surface_get", "evidence.read", "mailbox.write"][..])
        }
        _ => return Err("specialist_role_not_implemented".into()),
    };
    if let Some(allowed) = allowed {
        if capabilities
            .iter()
            .any(|capability| !allowed.contains(&capability.as_str()))
        {
            return Err("role_capability_not_allowed".into());
        }
    }
    if super::source::is_source_role(role) {
        super::source::validate_scheduled_slice(
            connection,
            lease,
            role,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
    }
    if role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(connection, lease, role)?
    {
        super::source_review_subject::validate_slice(
            connection,
            lease,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
    }
    if role == AgentRole::IdentitySession {
        let (mode, _) = crate::auth_session::validated_scan_identities(
            connection,
            &lease.scan_id,
            &lease.target_key,
        )?;
        if mode == crate::auth_session::ScanIdentityMode::AnonymousOnly {
            return Err("identity_session_requires_bound_identity".into());
        }
    }
    if role == AgentRole::Authorization {
        if capabilities != ["authorization_probe"] {
            return Err("authorization_capability_invalid".into());
        }
        let (mode, identities) = crate::auth_session::validated_scan_identities(
            connection,
            &lease.scan_id,
            &lease.target_key,
        )?;
        let (count, invalid): (i64, i64) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(CASE WHEN method!='GET' OR owner_identity=tester_identity THEN 1 ELSE 0 END),0) \
             FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3",
            params![lease.scan_id, lease.attempt_number, lease.target_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| "authorization_control_lookup_failed".to_string())?;
        if mode != crate::auth_session::ScanIdentityMode::IdentitySet
            || count == 0
            || count > 4
            || invalid != 0
            || reserved_tokens != 0
            // No model invocation: target requests are claimed by their own
            // broker and never reserved from the model-request ledger.
            || reserved_requests != 0
        {
            return Err("authorization_control_or_budget_invalid".into());
        }
        let unbound: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 \
             AND (owner_identity NOT IN (SELECT value FROM json_each(?4)) OR tester_identity NOT IN (SELECT value FROM json_each(?4))))",
            params![lease.scan_id, lease.attempt_number, lease.target_key, serde_json::json!(identities).to_string()],
            |row| row.get(0),
        ).map_err(|_| "authorization_control_identity_lookup_failed".to_string())?;
        if unbound {
            return Err("authorization_control_identity_unbound".into());
        }
    }
    let dedup_key = format!(
        "{}:{}:{}:{}",
        role.as_str(),
        trigger_code,
        lease.target_key,
        evidence_revision
    );
    let assignment_id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup_key}", lease.root_run_id))[..24]
    );
    let run_id = format!("run-{assignment_id}");
    let mut assignment = AgentAssignment::new(
        assignment_id.clone(),
        lease.root_run_id.clone(),
        role,
        lane,
        lease.target_key.clone(),
        dedup_key,
    );
    assignment.child_run_id = run_id.clone();
    assignment.trigger_code = trigger_code.to_string();
    assignment.task_slice = task_slice.clone();
    assignment.evidence_revision = evidence_revision;
    assignment.reserved_tokens = reserved_tokens.max(0);
    assignment.reserved_requests = reserved_requests.max(0);
    assignment.capability_lease = capabilities.to_vec();
    validate_coordinator_lease(transaction, lease)?;
    require_executable_coordinator(transaction, lease)?;
    if role == AgentRole::WebExecutor {
        ensure_fresh_web_executor_attempt(transaction, lease)?;
    }
    // Upgraded databases can contain active assignments written before the
    // lane table existed. Treat those as occupying their lane too; never
    // silently reclaim a slot on expiry, particularly a target-touching one.
    let legacy_occupant: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id \
             WHERE r.scan_id=?1 AND r.attempt_number=?2 AND a.target_key=?3 AND a.lane=?4 \
             AND a.id<>?5 AND a.state IN ('prepared','leased','running','waiting_review'))",
            params![
                lease.scan_id,
                lease.attempt_number,
                lease.target_key,
                lane.as_str(),
                assignment_id
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法检查旧版 lane 占用：{error}"))?;
    if legacy_occupant {
        return Err("agent_lane_occupied".into());
    }
    transaction
        .execute(
            "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) \
             SELECT id,hard_token_budget,hard_request_budget,?1,?2 FROM agent_runs WHERE id=?3 \
             ON CONFLICT(root_run_id) DO UPDATE SET lease_epoch=excluded.lease_epoch,fencing_token=excluded.fencing_token,updated_at=datetime('now','localtime')",
            params![lease.lease_epoch, lease.fencing_token, lease.root_run_id],
        )
        .map_err(|error| format!("无法初始化 child 预算账本：{error}"))?;
    let inserted = matches!(
        insert_assignment(transaction, &assignment)?,
        AssignmentInsert::Inserted(_)
    );
    let active: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND state IN ('prepared','leased','running','waiting_review'))",
            [&assignment_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法检查 assignment 生命周期：{error}"))?;
    if !active {
        return Err("assignment_replay_not_active".into());
    }
    // §5.3 Loop6: every scheduled child holds exactly one contract row in the
    // same transaction that takes the lane and reserves budget, so a failure
    // here rolls everything back together. The v1 key binds attempt + target +
    // role + trigger + revision; cross-trigger logical dedup (dropping the
    // trigger) waits until task_slice carries a stable action field.
    let contract = super::contract_owner::schedule_contract_key(
        lease.attempt_number,
        &lease.target_key,
        role.as_str(),
        trigger_code,
        evidence_revision,
    );
    super::contract_owner::acquire_contract_owner(
        transaction,
        &lease.root_run_id,
        &contract,
        &assignment_id,
        lease.lease_epoch,
        &lease.fencing_token,
    )?;
    let claimed = transaction
        .execute(
            "INSERT INTO agent_lane_leases(scan_id,attempt_number,target_key,lane,assignment_id) \
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
            params![
                lease.scan_id,
                lease.attempt_number,
                lease.target_key,
                lane.as_str(),
                assignment_id
            ],
        )
        .map_err(|error| format!("无法占用 lane：{error}"))?;
    if claimed == 0 {
        let owner: String = transaction
            .query_row(
                "SELECT assignment_id FROM agent_lane_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND lane=?4",
                params![lease.scan_id, lease.attempt_number, lease.target_key, lane.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法读取 lane 持有者：{error}"))?;
        if owner != assignment_id {
            return Err("agent_lane_occupied".into());
        }
    }
    if inserted {
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
    transaction
        .execute(
            "UPDATE agent_assignments SET lease_epoch=?1,fencing_token=?2,lease_expires_at=?3,\
             state=CASE WHEN state='prepared' THEN 'leased' ELSE state END,leased_at=CASE WHEN leased_at='' THEN datetime('now','localtime') ELSE leased_at END,\
             updated_at=datetime('now','localtime') WHERE id=?4 AND coordinator_run_id=?5",
            params![
                lease.lease_epoch,
                lease.fencing_token,
                lease.lease_expires_at,
                assignment_id,
                lease.root_run_id
            ],
        )
        .map_err(|error| format!("无法租用 assignment：{error}"))?;
    let mut row = AgentRunRow::new(
        run_id.clone(),
        lease.scan_id.clone(),
        lease.attempt_number,
        lease.target_key.clone(),
        AgentBackendKind::Native,
        role,
        format!("child:{trigger_code}:{evidence_revision}"),
        "",
    );
    row.parent_run_id = Some(lease.root_run_id.clone());
    row.root_run_id = lease.root_run_id.clone();
    row.assignment_id = assignment_id.clone();
    row.lane = Some(lane);
    row.orchestration_policy = MultiAgentPolicy::Multi;
    row.capability_lease = capabilities.to_vec();
    row.reserved_tokens = reserved_tokens.max(0);
    row.reserved_requests = reserved_requests.max(0);
    row.lease_expires_at = lease.lease_expires_at.clone();
    row.status = AgentRunStatus::Prepared;
    store::create_run(transaction, &row)?;
    for capability in capabilities {
        transaction
            .execute(
                "INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(assignment_id,capability) DO NOTHING",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    lease.root_run_id,
                    assignment_id,
                    run_id,
                    capability,
                    lease.lease_epoch,
                    lease.fencing_token,
                    lease.lease_expires_at
                ],
            )
            .map_err(|error| format!("无法签发 capability lease：{error}"))?;
    }
    // INSERT triggers must not change the frozen surface while authority is
    // being issued. Failure rolls back child, lane, capabilities and budget.
    super::source::validate_surface_role(transaction, lease, role)?;
    if role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(transaction, lease, role)?
    {
        super::source_review_subject::validate_slice(
            transaction,
            lease,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
        super::source_review_subject::verify_assignment(
            transaction,
            lease,
            &ScheduledChild {
                assignment_id: assignment_id.clone(),
                run_id: run_id.clone(),
                role,
            },
        )?;
    }
    Ok(ScheduledChild {
        assignment_id,
        run_id,
        role,
    })
}
