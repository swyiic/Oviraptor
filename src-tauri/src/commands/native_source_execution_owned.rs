// All post-capture Source exits return to the financial-only finally wrapper.
fn run_native_source_assessments_owned(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    coordinator: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    profile: &AgentModelProfile,
) -> Result<JsonValue, String> {
    let db_path = context.db_path;
    // This pure original read precedes reentry and every SDK/activation write.
    // Never mint/reacquire a C or financial owner for historical missing data.
    native_source_fresh_finance::original_for_execution(connection, context.run_id)?;
    let coordinator = coordinator.clone();
    let remaining = crate::agent_runtime::multi_agent::budget::clock::remaining(
        connection,
        &coordinator.root_run_id,
    )?;
    let deadline = std::time::Instant::now()
        .checked_add(remaining)
        .ok_or("source_model_timeout_invalid")?;
    let context = SpecialistTransportContext {
        deadline: Some(deadline),
        ..context.clone()
    };
    // This is paid local closure, never continuation of a failed worker.
    // All original proof and current authority are rechecked by the writer.
    if finish_pending_source_exhausted_root(connection, &context, &coordinator)? {
        return Err("source_tool_phase_round_budget_exhausted_without_finish".into());
    }
    let reentry = source_review_boundary_reentry(connection, &context)?;
    if let Some((saved, _, _)) = &reentry {
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
            connection,
            &coordinator.root_run_id,
        )?
        .require_original_coordinator(connection, saved)?;
    }

    let execute=|| -> Result<(Vec<JsonValue>,SourceCompletionProof,Option<crate::native_pipeline::source_ci::SourceGateReport>),String> {
        let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        authorize_source_specialist(&tx,&context,&coordinator)?;
        let changed=tx.execute("UPDATE agent_runs SET status='running',started_at=CASE WHEN started_at='' THEN datetime('now','localtime') ELSE started_at END,updated_at=datetime('now','localtime') WHERE id=?1 AND status IN ('prepared','running') AND cancel_requested_at=''",[&coordinator.root_run_id]).map_err(|e|e.to_string())?;
        if changed!=1 { return Err("source_coordinator_activation_conflict".into()); }
        authorize_source_specialist(&tx,&context,&coordinator)?;
        tx.commit().map_err(|e|e.to_string())?;
        let supervisor=crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor::start(db_path,&coordinator)?;
        let context=SpecialistTransportContext { supervision:Some(supervisor.ticket()), ..context.clone() };
        let (mut assessments, initial_start, tool_start) = match &reentry {
            None => (Vec::new(), 0, Some(0)),
            Some((_, saved, SourceResumePhase::Initial(next))) => (saved.clone(), *next, Some(0)),
            Some((_, saved, SourceResumePhase::Tools(next))) => (saved.clone(), 2, Some(*next)),
            Some((_, saved, SourceResumePhase::ToolPending(pending))) =>
                (saved.clone(), 2, (pending.next_index < 2).then_some(pending.next_index)),
            Some((_, saved, SourceResumePhase::Review)) => (saved.clone(), 2, None),
        };
        assessments.extend(run_source_initial_phases_from(connection,&context,&coordinator,profile,initial_start)?);
        supervisor.check()?;
        if let Some((_,_,SourceResumePhase::ToolPending(pending)))=&reentry {
            // The exact saved child is resumed; scheduling it again would
            // change its reservation and could repeat an already received call.
            let result=execute_source_tool_assignment(connection,&context,&coordinator,&pending.child,&pending.slice)?;
            assessments.push(json!({"role":pending.child.role.as_str(),"assignmentId":pending.child.assignment_id,
                "runId":pending.child.run_id,"result":result}));
        }
        supervisor.check()?;
        if let Some(next) = tool_start {
            assessments.extend(run_source_tool_phases_from(connection,&context,&coordinator,next)?);
        }
        supervisor.check()?;
        run_source_candidate_review(connection,&context,&coordinator)?;
        supervisor.check()?;
        run_source_coverage_review(connection,&context,&coordinator)?;
        supervisor.check()?;
        let (proof,gate)=finish_native_source_coordinator(connection,&context,&coordinator)?;
        Ok((assessments,proof,gate))
    };
    match execute() {
        Ok((assessments, proof, gate)) => {
            let candidate_review = proof.candidate_review;
            let completion = proof.completion;
            Ok(
                json!({"rootRunId":coordinator.root_run_id,"status":if proof.coverage_decision.is_some() {"source_coverage_reviewed"} else if candidate_review.is_some() {"source_candidates_reviewed_coverage_open"} else {"source_analysis_completed_unreviewed"},
                "independentReviewCompleted":proof.coverage_decision.is_some(),
                "sourceCoverageDecision":proof.coverage_decision.as_ref().map(|decision|decision.as_json()),
                "independentCandidateReviewCompleted":candidate_review.is_some(),"candidateReview":candidate_review,"assessments":assessments,
                "coverageReviewPreparation":proof.coverage_preparation.as_json(),
                "sourceDecisionProjection":proof.decision_projection.as_ref().map(|set|set.as_json()),"confirmedFindings":completion.confirmed_findings,
                "gate":gate.as_ref().map(|gate|gate.as_json()),
                "verifiedToolResults":completion.verified_tool_results,"modelRequests":completion.model_requests,"totalTokens":completion.total_tokens}),
            )
        }
        Err(error) => {
            // A recovered child may have committed local tools before a later
            // failure. Preserve its still-live root and reservation for a
            // fresh audit instead of terminalizing an unknown continuation.
            let recovered_exhausted =
                matches!(reentry, Some((_, _, SourceResumePhase::ToolPending(_))))
                    && error == "source_tool_phase_round_budget_exhausted_without_finish";
            if matches!(reentry, Some((_, _, SourceResumePhase::ToolPending(_))))
                && !recovered_exhausted
            {
                return Err(error);
            }
            // A stopped attempt, cancelled Root, expired clock or replaced C
            // cannot publish a new business terminal. Preserve its original
            // pending receipts/fees; the outer finally only observes elapsed.
            if native_source_fresh_finance::original_for_execution(
                connection,
                &coordinator.root_run_id,
            )
            .is_err()
            {
                return Err(error);
            }
            #[cfg(test)]
            source_failure_cutover_for_test();
            // Incomplete root may retain paused children/reservations. Never
            // manufacture refunds or completed mailbox messages for unknown IO.
            finish_owned_source_failure(
                connection,
                &context,
                &coordinator,
                &AgentTargetOutcome::incomplete(
                    "源码多智能体执行未完成，子任务回执和未决预算保留待核对",
                ),
                recovered_exhausted,
            )
            .map_err(|close| format!("{error};source_coordinator_close:{close}"))?;
            Err(error)
        }
    }
}

// A live failure may publish its existing paused outcome, but the admission
// and that publication must hold the same SQLite write transaction.
fn finish_owned_source_failure(
    db: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    outcome: &AgentTargetOutcome,
    recovered_exhausted: bool,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{budget, lease};
    budget::clock::closure_write(db, c, false, |tx| {
        native_source_fresh_finance::original_for_execution(tx, &c.root_run_id)?;
        lease::require_executable_coordinator(tx, c)?;
        authorize_source_specialist(tx, context, c)?;
        // The error selects a stricter audit; it never proves a failed child.
        let exhausted = recovered_exhausted
            .then(|| audit_recovered_source_exhausted_root(tx, c))
            .transpose()?;
        let clock = finish_coordinator_run_in_transaction(tx, c, outcome)?;
        lease::require_active_attempt(tx, &c.scan_id, c.attempt_number)?;
        budget::root::RootOwner::load_original(tx, &c.root_run_id)?
            .require_original_coordinator(tx, c)?;
        if let Some(original) = exhausted {
            if audit_recovered_source_exhausted_root(tx, c)? != original {
                return Err("source_recovered_exhausted_closure_changed".into());
            }
        }
        clock.verify_closed(
            tx,
            outcome.terminal_status(),
            outcome.terminal_code(),
            &crate::agent_runtime::secrets::redact_text_with(&outcome.detail(), None),
        )
    })
}

#[cfg(test)]
thread_local! {
    static SOURCE_FAILURE_CUTOVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn source_failure_cutover_once_for_test(work: impl FnOnce() + 'static) {
    SOURCE_FAILURE_CUTOVER.with(|cell| {
        assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(Box::new(work));
    });
}
#[cfg(test)]
fn source_failure_cutover_for_test() {
    let work = SOURCE_FAILURE_CUTOVER.with(|cell| cell.borrow_mut().take());
    if let Some(work) = work {
        work();
    }
}

include!("native_source_recovered_exhausted.rs");
