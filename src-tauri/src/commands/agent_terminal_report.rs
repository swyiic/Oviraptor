fn record_runtime_terminal_facts_original(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    original: &OriginalAgentTerminalIdentity,
    outcome: &AgentTargetOutcome,
) -> Result<Option<crate::agent_runtime::reducer::Reduction>, String> {
    original.require_scope(db_path, scan_id, route)?;
    let connection = db::open(db_path)?;
    {
        let tx = connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        if let Some(reduction) = original_multi_terminal_on(&tx, original, scan_id, route)? {
            require_original_multi_outcome(&reduction, outcome)?;
            return Ok(Some(reduction));
        }
    }
    let mut report = runtime_report_for_attempt(db_path, scan_id, route, original.attempt_number);
    let source = crate::agent_runtime::single_projection::source_hash(&connection, &report)?;
    let native_state = NativeAgentState::read(db_path, scan_id, &route.url);
    if native_state.as_ref().is_some_and(|state| {
        state.attempt_number != report.attempt_number || state.backend != report.backend.as_str()
    }) {
        return Err("runtime_terminal_native_source_scope_conflict".into());
    }
    report.pending_contracts = native_state
        .as_ref()
        .map(|value| value.pending_queue.len() as i64)
        .unwrap_or(0);
    report.completed_contracts = native_state
        .as_ref()
        .map(|value| value.completed_contract_keys.clone())
        .unwrap_or_default();
    report.target_requests = native_state
        .as_ref()
        .map(|value| value.target_requests)
        .unwrap_or(0);
    let accounting =
        agent_request_accounting_view(&connection, scan_id, report.attempt_number, &route.url);
    if let Some(recorded) = accounting["recordedRequests"].as_i64() {
        report.target_requests = recorded;
    }
    report.request_accounting = Some(accounting);
    // The completion only exists on the two *Completed states; a limited,
    // cancelled or interrupted run still owes its real spend, which the native
    // checkpoint already carries.
    report.usage = match outcome.completion() {
        Some(completion) => crate::agent_runtime::store::UsageDelta {
            total_tokens: completion.total_tokens,
            model_requests: completion.model_requests,
            ..Default::default()
        },
        None => native_state
            .as_ref()
            .map(|value| crate::agent_runtime::store::UsageDelta {
                input_tokens: value.token_usage.input_tokens,
                cached_input_tokens: value.token_usage.cached_input_tokens,
                output_tokens: value.token_usage.output_tokens,
                total_tokens: value.token_usage.total_tokens,
                model_requests: value.token_usage.model_requests,
            })
            .unwrap_or_default(),
    };
    if let Some(completion) = outcome.completion() {
        report.covered_families = completion.covered_families.clone();
        report.evidence_records = completion.verified_tool_results;
        report.confirmed_findings = completion.confirmed_findings;
        report.ledger_closed = completion.ledger_reported;
        // The ledger's own accounting is the requirement: families it proved plus
        // families it named as gaps. Anything the close-out declared not applicable
        // is therefore neither a gap nor a coverage claim (§9.5).
        report.required_families = completion
            .covered_families
            .iter()
            .chain(completion.uncovered_families.iter())
            .cloned()
            .collect();
        // A bounded native exit can happen before a finish_target ledger is
        // accepted (hard budget or the second no-progress window).  That is a
        // terminal boundary with usable evidence, not a resumable/open ledger.
        // Express the boundary as reducer input instead of letting
        // `pending_contracts` incorrectly collapse it to Incomplete.
        if matches!(outcome, AgentTargetOutcome::BoundedCompleted(_)) && !completion.ledger_reported
        {
            match completion.terminal_code {
                AGENT_STOP_HARD_TOKENS | AGENT_STOP_HARD_REQUESTS => {
                    report.hard_limit_reason = Some(completion.summary.clone());
                }
                AGENT_STOP_SOFT_TOKENS
                | AGENT_STOP_SOFT_REQUESTS
                | AGENT_STOP_NO_PROGRESS
                | AGENT_STOP_DERIVED => {
                    report.soft_budget_stall_reason = Some(completion.summary.clone());
                }
                _ => {}
            }
        }
    }
    let stop = outcome.stop();
    if let Some(stop) = stop {
        let reason = stop.reason.clone();
        match stop.code {
            // §5.2: the reduced state has to say "the local record failed", or the
            // projection below would blame the model for a disk or database error.
            AGENT_STOP_PERSISTENCE => report.persistence_failure = Some(reason),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED => {
                report.request_reconciliation_required = Some(reason)
            }
            terminal_code::EXECUTION_AUTHORIZATION_DENIED => {
                report.execution_authorization_denied = Some(reason)
            }
            AGENT_STOP_CONFIGURATION | AGENT_STOP_EVIDENCE_INTEGRITY => {
                report.configuration_error = Some(reason)
            }
            AGENT_STOP_WAF | AGENT_STOP_RATE_LIMIT | AGENT_STOP_SCOPE => {
                report.protection_signal = Some(reason)
            }
            AGENT_STOP_HARD_TOKENS | AGENT_STOP_HARD_REQUESTS => {
                report.hard_limit_reason = Some(reason)
            }
            AGENT_STOP_SOFT_TOKENS | AGENT_STOP_SOFT_REQUESTS | AGENT_STOP_NO_PROGRESS => {
                report.soft_budget_stall_reason = Some(reason)
            }
            AGENT_STOP_UNSUPPORTED => report.unsupported_capability = Some(reason),
            AGENT_STOP_RESUME_INCOMPATIBLE => report.resume_incompatible = Some(reason),
            _ => report.detail = reason,
        }
    }
    report.cancelled = matches!(outcome, AgentTargetOutcome::Cancelled);
    // The checkpoint is the authority on whether this attempt can continue, so
    // the run row agrees with it instead of claiming a terminal state early.
    report.resumable = report.request_reconciliation_required.is_none()
        && report.execution_authorization_denied.is_none()
        && native_state
            .as_ref()
            .map(|value| value.terminal_reason.is_empty())
            .unwrap_or(false)
        && matches!(
            outcome,
            AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::Cancelled
        );
    report.detail = outcome.detail();
    let mut q = connection.prepare("SELECT id,orchestration_policy FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator'").map_err(|e| e.to_string())?;
    let roots = q
        .query_map(params![scan_id, report.attempt_number, route.url], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let [(root, policy)] = roots.as_slice() else {
        return Err("runtime_terminal_original_root_missing_or_ambiguous".into());
    };
    if Some(root.as_str()) != original.root_run_id.as_deref() {
        return Err("runtime_terminal_captured_root_conflict".into());
    }
    if policy == "single" {
        use crate::agent_runtime::reducer::backend_exit::BackendExit;
        let exit = match outcome {
            AgentTargetOutcome::Completed(_) => BackendExit::Completed,
            AgentTargetOutcome::BoundedCompleted(_) => BackendExit::Bounded,
            AgentTargetOutcome::Incomplete(_) => BackendExit::Incomplete,
            AgentTargetOutcome::Limited(_) => BackendExit::Limited,
            AgentTargetOutcome::Failed(_) => BackendExit::Failed,
            AgentTargetOutcome::ResumeIncompatible(_) => BackendExit::ResumeIncompatible,
            AgentTargetOutcome::Cancelled => BackendExit::Cancelled,
        };
        return crate::agent_runtime::single_projection::close(
            db_path,
            root,
            &crate::agent_runtime::single_projection::SingleReport {
                report: &report,
                exit,
                code: outcome.terminal_code(),
                reason: &outcome.detail(),
                source: &source,
            },
        );
    }
    Err("runtime_terminal_original_policy_changed".into())
}

// A pure original financial reader. It grants neither live C nor resume rights.
fn original_multi_terminal_on(
    db: &rusqlite::Connection,
    original: &OriginalAgentTerminalIdentity,
    scan: &str,
    route: &FrontendRoute,
) -> Result<Option<crate::agent_runtime::reducer::Reduction>, String> {
    if db.is_autocommit() {
        return Err("runtime_terminal_original_snapshot_required".into());
    }
    original.require_on(db, scan, route)?;
    let root = original
        .root_run_id
        .as_deref()
        .ok_or("runtime_terminal_original_root_missing")?;
    let run = crate::agent_runtime::store::load_run(db, root)?
        .ok_or("runtime_terminal_original_root_missing")?;
    if run.orchestration_policy.as_str() == "single" {
        // Reject a Multi relabelled as Single using its immutable original contract.
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(db, root)?;
        return Ok(None);
    }
    crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(db, root)?;
    Ok(Some(crate::agent_runtime::reducer::Reduction {
        state: run.terminal_state.ok_or("runtime_terminal_missing")?,
        code: run.terminal_code,
        reason: run.terminal_reason,
    }))
}

fn require_original_multi_outcome(
    reduction: &crate::agent_runtime::reducer::Reduction,
    outcome: &AgentTargetOutcome,
) -> Result<(), String> {
    if reduction.state.to_sentinel_status() != outcome.terminal_status()
        || reduction.code != outcome.terminal_code()
        || reduction.reason
            != crate::agent_runtime::secrets::redact_text_with(&outcome.detail(), None)
    {
        return Err("runtime_terminal_original_outcome_conflict".into());
    }
    Ok(())
}
