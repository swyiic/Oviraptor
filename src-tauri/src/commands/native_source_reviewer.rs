// Reviewer-boundary reentry must happen before acquisition can rotate an expired
// fence, and before generic error handling can terminalize an unknown call.
// Earlier partially completed source-tool phases need their own recovery contract.
enum SourceResumePhase {
    Initial(usize),
    Tools(usize),
    ToolPending(SourceToolPending),
    Review,
}

fn source_review_boundary_reentry(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
) -> Result<
    Option<(
        crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        Vec<JsonValue>,
        SourceResumePhase,
    )>,
    String,
> {
    use crate::agent_runtime::multi_agent::{
        lease, source_coverage_reviewer, source_phases, source_reviewer,
    };
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    let (reviewers,completed,nonreview):(i64,i64,i64)=tx.query_row("SELECT
        (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='evidence_reviewer'),
        (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role<>'evidence_reviewer' AND state='completed'),
        (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role<>'evidence_reviewer')",
        [context.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    // Source tools belong to fenced child assignments. A root invocation at
    // any checkpoint may represent an unknown external side effect, even if
    // all visible child receipts are complete. Never schedule past it.
    let root_tool: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)",
            [context.run_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("source_root_tool_lookup:{e}"))?;
    if root_tool {
        return Err("source_root_tool_outcome_requires_reconciliation".into());
    }
    // A partial assignment may own an executing request whose response was
    // never durably received. Admission must not rotate its fence or let the
    // generic error handler terminalize the root on a dedup conflict.
    if reviewers == 0 && completed < 3 {
        if nonreview != completed
            && !(completed < 2 && nonreview == completed + 1)
            && !(completed == 2 && nonreview == 3)
        {
            return Err("source_partial_outcome_unknown_requires_reconciliation".into());
        }
        if completed == 0 && nonreview == 0 {
            // Registration is not a dispatch receipt. A root with no children
            // can start only when there is no orphaned accounting or phase
            // output; otherwise a retry could rotate a fence and silently
            // execute new model calls against an inconsistent old budget.
            let empty:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r WHERE r.id=?1
                AND r.root_run_id=r.id AND r.status IN ('prepared','running')
                AND r.cancel_requested_at='' AND r.used_tokens=0 AND r.used_cached_tokens=0
                AND r.used_requests=0 AND r.reserved_tokens=0 AND r.reserved_requests=0
                AND r.terminal_state='' AND r.finished_at='')
                AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE root_run_id=?1 AND id<>?1)
                AND NOT EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1)
                AND NOT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?1)
                AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1)
                AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE root_run_id=?1
                    AND kind IN ('evidence_summary','source_tool_result','source_review_result','source_coverage_review_result'))
                AND NOT EXISTS(SELECT 1 FROM agent_source_review_decisions WHERE root_run_id=?1)
                AND NOT EXISTS(SELECT 1 FROM agent_source_coverage_decisions WHERE root_run_id=?1)",
                [context.run_id],|r|r.get(0)).map_err(|e|format!("source_empty_prefix_lookup:{e}"))?;
            if !empty {
                return Err("source_empty_prefix_orphaned_work_or_accounting".into());
            }
            return Ok(None);
        }
    }
    let coordinator = tx
        .query_row(
            "SELECT lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases
        WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_key=?4",
            params![
                context.run_id,
                context.scan_id,
                context.attempt_number,
                context.target_key
            ],
            |r| {
                Ok(lease::CoordinatorLease {
                    scan_id: context.scan_id.into(),
                    attempt_number: context.attempt_number,
                    target_key: context.target_key.into(),
                    root_run_id: context.run_id.into(),
                    lease_epoch: r.get(0)?,
                    fencing_token: r.get(1)?,
                    lease_expires_at: r.get(2)?,
                })
            },
        )
        .map_err(|_| "source_review_reentry_lease_missing")?;
    lease::require_executable_coordinator(&tx, &coordinator)?;
    lease::validate_coordinator_lease(&tx, &coordinator)?;
    authorize_source_specialist(&tx, context, &coordinator)?;
    if reviewers == 0 && completed < 2 && nonreview == completed + 1 {
        let (child, usage, payload) =
            source_initial_received_reentry(&tx, &coordinator, completed)?;
        // The admission transaction is deliberately read-only. Delivery has
        // its own atomic transaction and must never invoke transport/prepare.
        drop(tx);
        complete_readonly_assessment(connection, &coordinator, &child, &usage, &payload)?;
        return source_review_boundary_reentry(connection, context);
    }
    // A received Reviewer call also has a known outcome. Audit its complete
    // phase prefix and reservation before local delivery; recursive reentry
    // still refuses any next model phase after the deadline.
    if reviewers > 0
        && context
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    {
        if let source_reviewer::ReviewProgress::Received(child) =
            source_reviewer::progress(&tx, &coordinator)?
        {
            let phases = source_phases::audit(&tx, &coordinator)?;
            source_review_pending_accounting(&tx, &coordinator, &child, &phases, true)?;
            drop(tx);
            deliver_source_candidate_review(connection, context, &coordinator, &child)?;
            return source_review_boundary_reentry(connection, context);
        }
        if source_coverage_reviewer::enabled(&tx, &coordinator)? {
            if let source_coverage_reviewer::ReviewProgress::Received(child) =
                source_coverage_reviewer::progress(&tx, &coordinator)?
            {
                let phases = source_phases::audit(&tx, &coordinator)?;
                source_review_pending_accounting(&tx, &coordinator, &child, &phases, true)?;
                drop(tx);
                deliver_source_coverage_review(connection, context, &coordinator, &child)?;
                return source_review_boundary_reentry(connection, context);
            }
        }
    }
    // Saved receipts need no new model work, but a subsequent phase does.
    if context
        .deadline
        .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    {
        if reviewers == 0 && matches!(completed, 2 | 3) && nonreview == completed + 1 {
            let (_, pending) =
                source_tool_received_reentry(&tx, context, &coordinator, completed, true)?;
            drop(tx);
            settle_expired_source_finish(connection, context, &coordinator, &pending)?;
            return source_review_boundary_reentry(connection, context);
        }
        if completed == 4 && nonreview == 4 {
            if let Some(assessments) = source_expired_completed_reviews(&tx, &coordinator)? {
                return Ok(Some((coordinator, assessments, SourceResumePhase::Review)));
            }
        }
        return Err("source_model_deadline_exceeded".into());
    }
    if reviewers == 0 && matches!(completed, 2 | 3) && nonreview == completed + 1 {
        let (assessments, pending) =
            source_tool_received_reentry(&tx, context, &coordinator, completed, false)?;
        if pending.settled_finish {
            drop(tx);
            settle_expired_source_finish(connection, context, &coordinator, &pending)?;
            return source_review_boundary_reentry(connection, context);
        }
        return Ok(Some((
            coordinator,
            assessments,
            SourceResumePhase::ToolPending(pending),
        )));
    }
    if reviewers == 0 && completed < 3 {
        let prefix = source_phases::audit_initial_prefix(&tx, &coordinator, completed as usize)?;
        let settled:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs r ON r.id=b.root_run_id
            WHERE b.root_run_id=?1 AND b.lease_epoch=?2 AND b.fencing_token=?3
            AND b.reserved_tokens=0 AND b.reserved_requests=0 AND b.spent_tokens=?4 AND b.spent_requests=?5
            AND b.total_tokens=r.hard_token_budget AND b.total_requests=r.hard_request_budget
            AND r.reserved_tokens=0 AND r.reserved_requests=0)
            AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
            AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=?6
            AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('source_tool_result','evidence_summary'))=?6
            AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=?6
            AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1)",
            params![coordinator.root_run_id,coordinator.lease_epoch,coordinator.fencing_token,
                prefix.tokens,prefix.requests,completed],|r|r.get(0)).map_err(|e|format!("source_initial_prefix_ledger_lookup:{e}"))?;
        if !settled {
            return Err("source_initial_prefix_ledger_or_extra_work_mismatch".into());
        }
        return Ok(Some((
            coordinator,
            prefix.assessments,
            SourceResumePhase::Initial(completed as usize),
        )));
    }
    if reviewers == 0 && completed == 3 {
        let prefix = source_phases::audit_first_tool_prefix(&tx, &coordinator)?;
        let settled:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs r ON r.id=b.root_run_id
            WHERE b.root_run_id=?1 AND b.lease_epoch=?2 AND b.fencing_token=?3
            AND b.reserved_tokens=0 AND b.reserved_requests=0 AND b.spent_tokens=?4 AND b.spent_requests=?5
            AND b.total_tokens=r.hard_token_budget AND b.total_requests=r.hard_request_budget
            AND r.reserved_tokens=0 AND r.reserved_requests=0)
            AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
            AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=3
            AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('source_tool_result','evidence_summary'))=3
            AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=2
            AND (SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1)=?5-2",
            params![coordinator.root_run_id,coordinator.lease_epoch,coordinator.fencing_token,
                prefix.tokens,prefix.requests],|r|r.get(0)).map_err(|e|format!("source_partial_ledger_lookup:{e}"))?;
        if !settled {
            return Err("source_partial_ledger_or_extra_work_mismatch".into());
        }
        return Ok(Some((
            coordinator,
            prefix.assessments,
            SourceResumePhase::Tools(1),
        )));
    }
    let phases = source_phases::audit(&tx, &coordinator)?;
    match source_reviewer::progress(&tx, &coordinator)? {
        source_reviewer::ReviewProgress::NotStarted
        | source_reviewer::ReviewProgress::Delivered(_) => {
            if source_coverage_reviewer::enabled(&tx, &coordinator)? {
                match source_coverage_reviewer::progress(&tx, &coordinator)? {
                    source_coverage_reviewer::ReviewProgress::NotStarted
                    | source_coverage_reviewer::ReviewProgress::Delivered(_) => {
                        source_assessment_completion(&tx, &coordinator, &phases.bases)?;
                    }
                    source_coverage_reviewer::ReviewProgress::Undispatched(child) => {
                        source_review_pending_accounting(
                            &tx,
                            &coordinator,
                            &child,
                            &phases,
                            false,
                        )?;
                    }
                    source_coverage_reviewer::ReviewProgress::Received(child) => {
                        source_review_pending_accounting(&tx, &coordinator, &child, &phases, true)?;
                    }
                }
            } else {
                source_assessment_completion(&tx, &coordinator, &phases.bases)?;
            }
        }
        source_reviewer::ReviewProgress::Undispatched(child) => {
            source_review_pending_accounting(&tx, &coordinator, &child, &phases, false)?;
        }
        source_reviewer::ReviewProgress::Received(child) => {
            source_review_pending_accounting(&tx, &coordinator, &child, &phases, true)?;
        }
    }
    // Pure audit: leave all original timestamps, fences, reservations and
    // capabilities untouched. Each following write rechecks this authority.
    Ok(Some((
        coordinator,
        phases.assessments,
        SourceResumePhase::Review,
    )))
}

// A durably received initial assessment has a known outcome. Verify the
// complete prefix and the sole pending reservation before any local write;
// executing/unknown calls do not pass this gate and cannot be redispatched.
fn source_initial_received_reentry(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    completed: i64,
) -> Result<
    (
        crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        AgentTokenUsage,
        JsonValue,
    ),
    String,
> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler::ScheduledChild, source, source_phases, specialist},
    };
    let (tokens, requests) = if completed == 1 {
        let prefix = source_phases::audit_initial_received_prefix(connection, lease)?;
        (prefix.tokens, prefix.requests)
    } else if completed == 0 {
        (0, 0)
    } else {
        return Err("source_initial_pending_prefix_invalid".into());
    };
    let role = if completed == 0 {
        AgentRole::RepoMapper
    } else {
        AgentRole::SourceAnalyst
    };
    let (assignment_id,run_id,state,child_status,reserved_tokens,reserved_requests,request):
        (String,String,String,String,i64,i64,String)=connection.query_row(
        "SELECT a.id,r.id,a.state,r.status,a.reserved_tokens,a.reserved_requests,c.request_json
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id
         JOIN agent_specialist_calls c ON c.assignment_id=a.id AND c.child_run_id=r.id
         WHERE a.coordinator_run_id=?1 AND a.role=?2 AND a.lane='read_only_analysis'
         AND a.lease_epoch=?3 AND a.fencing_token=?4 AND a.target_key=?5
         AND a.budget_settled_at='' AND r.root_run_id=?1 AND r.parent_run_id=?1
         AND r.role=a.role AND r.lane=a.lane AND r.scan_id=?6 AND r.attempt_number=?7
         AND r.target_url=?5 AND r.cancel_requested_at='' AND r.used_tokens=0 AND r.used_requests=0
         AND c.root_run_id=?1 AND c.role=a.role AND c.lease_epoch=?3 AND c.fencing_token=?4
         AND c.state='received'",
        params![lease.root_run_id,role.as_str(),lease.lease_epoch,lease.fencing_token,
            lease.target_key,lease.scan_id,lease.attempt_number],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))
        .map_err(|_|"source_initial_received_binding_invalid")?;
    if !matches!(
        (state.as_str(), child_status.as_str()),
        ("running", "running") | ("paused", "paused")
    ) || reserved_tokens <= 0
        || reserved_requests != 1
    {
        return Err("source_initial_received_state_invalid".into());
    }
    let child = ScheduledChild {
        assignment_id,
        run_id,
        role,
    };
    let slice = source::verify_assignment(connection, lease, &child)?;
    let request: JsonValue =
        serde_json::from_str(&request).map_err(|_| "source_initial_request_invalid")?;
    source::validate_request(connection, lease, &child, &request)?;
    let receipt = specialist::source_received_for_completion(connection, lease, &child)?;
    if !receipt.rejection.is_empty() {
        return Err("source_initial_response_rejected".into());
    }
    let clean:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs root ON root.id=b.root_run_id
        JOIN agent_assignments a ON a.id=?2 JOIN agent_runs r ON r.id=a.child_run_id
        WHERE b.root_run_id=?1 AND b.lease_epoch=?3 AND b.fencing_token=?4
        AND b.total_tokens=root.hard_token_budget AND b.total_requests=root.hard_request_budget
        AND b.spent_tokens=?5 AND b.spent_requests=?6 AND b.reserved_tokens=a.reserved_tokens
        AND b.reserved_requests=1 AND r.reserved_tokens=a.reserved_tokens AND r.reserved_requests=1
        AND root.reserved_tokens=0 AND root.reserved_requests=0)
        AND (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1)=?7
        AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=?7
        AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=?7
        AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='evidence_summary')=?8
        AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND assignment_id<>?2 AND revoked_at='')
        AND (SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?2 AND revoked_at='')=CASE WHEN ?9='paused' THEN 0 ELSE 2 END
        AND (?9='paused' OR ((SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?2 AND revoked_at='' AND capability='evidence.read')=1
            AND (SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?2 AND revoked_at='' AND capability='mailbox.write')=1))
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?2 AND revoked_at=''
            AND (root_run_id<>?1 OR child_run_id<>?10 OR lease_epoch<>?3 OR fencing_token<>?4
                 OR lease_expires_at<=datetime('now','localtime') OR capability NOT IN ('evidence.read','mailbox.write')))
        AND (SELECT count(*) FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id
            WHERE a.coordinator_run_id=?1)=1
        AND EXISTS(SELECT 1 FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id
            WHERE l.assignment_id=?2 AND l.scan_id=?11 AND l.attempt_number=?12
            AND l.target_key=?13 AND l.lane=a.lane)",
        params![lease.root_run_id,child.assignment_id,lease.lease_epoch,lease.fencing_token,
            tokens,requests,completed+1,completed,state,child.run_id,lease.scan_id,
            lease.attempt_number,lease.target_key],|r|r.get(0))
        .map_err(|e|format!("source_initial_received_accounting:{e}"))?;
    if !clean {
        return Err("source_initial_received_accounting_invalid".into());
    }
    Ok((
        child,
        receipt.usage,
        json!({"sourceTask":slice,"summary":receipt.text}),
    ))
}

include!("native_source_candidate_review.rs");
