fn multi_agent_finish_execution(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
    outcome: &AgentTargetOutcome,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::scheduler;
    let connection = db::open(&context.db_path)?;
    session.lease = crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
        &session.lease.root_run_id,
        600,
    )?;
    let settlement = (|| -> Result<AgentTokenUsage, String> {
        let tx = rusqlite::Transaction::new_unchecked(
            &connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| e.to_string())?;
        let usage = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
            &tx,
            &session.lease,
            &session.executor.assignment_id,
            &session.executor.run_id,
        )?;
        settle_child_usage_in_transaction(&tx, &session.lease, &session.executor, &usage)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(usage)
    })();
    let usage = match settlement {
        Ok(usage) => usage,
        Err(error) => {
            if let Err(cleanup) = stop_failed_child_preserving_usage(
                &connection,
                &session.lease,
                &session.executor,
                &error,
            ) {
                return Err(format!("{error};executor_child_cleanup:{cleanup}"));
            }
            return Err(error);
        }
    };
    // Financial settlement above remains valid even if supervision failed.
    // A stopped supervisor cannot authorize subsequent business publication.
    session.supervisor.check()?;
    let success = matches!(
        outcome,
        AgentTargetOutcome::Completed(_)
            | AgentTargetOutcome::BoundedCompleted(_)
            | AgentTargetOutcome::Incomplete(_)
            | AgentTargetOutcome::Limited(_)
    );
    let correlation_id = format!("executor-result:{}", session.executor.assignment_id);
    let payload = serde_json::json!({
        "terminalCode": outcome.terminal_code(), "summary": outcome.detail(),
        "usage": { "totalTokens": usage.total_tokens, "modelRequests": usage.model_requests }
    });
    if success
        && original_executor_finished_replay(
            &connection,
            &session.lease,
            &session.executor,
            &payload,
            &outcome.detail(),
        )?
    {
        if !outcome_requires_manual_execution_resolution(outcome) {
            multi_agent_client_side_readonly(context, session)?;
        }
        return Ok(());
    }
    let finalize = complete_target_child_delivery(
        &connection,
        &session.lease,
        &session.executor,
        "execution_result",
        &correlation_id,
        &payload,
        success,
        &outcome.detail(),
    );
    if let Err(error) = finalize {
        if let Err(cleanup) = scheduler::finish_child(
            &connection,
            &session.lease,
            &session.executor,
            false,
            &error,
        ) {
            return Err(format!("{error};executor_child_cleanup:{cleanup}"));
        }
        return Err(error);
    }
    if success && !outcome_requires_manual_execution_resolution(outcome) {
        multi_agent_client_side_readonly(context, session)?;
    }
    Ok(())
}

/// A separately leased, target-touching Authorization child runs only when an
/// operator prepared an immutable control group for this exact attempt. Its
/// three GETs are claimed individually by the broker before network I/O.
fn multi_agent_authorization(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let connection = db::open(&context.db_path)?;
    session.supervisor.check()?;
    let controls = load_authorization_controls(&connection, context)?;
    if controls.is_empty() {
        return Ok(());
    }
    if controls.len() > 4 {
        return Err("authorization_control_group_limit_exceeded".into());
    }
    session.lease = crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
        &session.lease.root_run_id,
        600,
    )?;
    let child = scheduler::schedule_child(
        &connection,
        &session.lease,
        AgentRole::Authorization,
        AgentLane::TargetTouching,
        "operator_authorization_control_ready",
        &serde_json::json!({"contractKeys":controls.iter().map(|row| &row.contract_key).collect::<Vec<_>>(),
            "objective":"仅执行已登记的 A→X、B→X、B→Y GET 控制组"}),
        1,
        &["authorization_probe".into()],
        0,
        0,
    )?;
    scheduler::start_child_or_release(&connection, &session.lease, &child)?;
    let mut child_context = context.clone();
    child_context.run = Some(AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: child.run_id.clone(),
    });
    let result = (|| -> Result<usize, String> {
        let mut verified = 0;
        for control in &controls {
            session.supervisor.check()?;
            authorization_control_urls(control)?;
            let (owner, owner_artifact) = execute_authorization_side(
                &child_context,
                &child,
                control,
                "owner",
                &control.owner_identity,
                &control.owner_object_url,
            )?;
            let (cross, cross_artifact) = execute_authorization_side(
                &child_context,
                &child,
                control,
                "cross",
                &control.tester_identity,
                &control.owner_object_url,
            )?;
            let (tester, tester_artifact) = execute_authorization_side(
                &child_context,
                &child,
                control,
                "tester",
                &control.tester_identity,
                &control.tester_control_url,
            )?;
            if agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number)
                .is_cancelled()
            {
                return Err("authorization_attempt_cancelled".into());
            }
            match verify_authorization_control_group(control, [&owner, &cross, &tester]) {
                Ok(()) => {
                    let key = format!(
                        "authorization-control:{}",
                        &crate::agent_runtime::store::stable_hash(&format!(
                            "{}:{}:{}",
                            context.scan_id, context.attempt_number, control.contract_key
                        ))[..24]
                    );
                    stage_agent_finding_with_active_guard(
                        &child_context,
                        AGENT_VULNERABILITY_STAGE,
                        "vulnerability",
                        &key,
                        "跨账号访问已登记的他人对象",
                        "high",
                        &serde_json::json!({
                            "kind":"authorization_cross_object_read", "contractKey":control.contract_key,
                            "verification":"operator_bound_three_side_native_control",
                            "responseObjectPointer":control.response_object_pointer,
                            "method":"GET", "sides":[
                                {"name":"owner", "artifactId":owner_artifact},
                                {"name":"cross", "artifactId":cross_artifact},
                                {"name":"tester", "artifactId":tester_artifact},
                            ],
                            "attemptNumber":context.attempt_number,
                        }),
                        true,
                    )?;
                    verified += 1;
                }
                Err(code) => {
                    append_runner_log(
                        &context.log_path,
                        &format!(
                            "authorization control {} insufficient: {code}",
                            control.contract_key
                        ),
                    );
                }
            }
        }
        Ok(verified)
    })();
    let summary = match &result {
        Ok(count) => format!("authorization controls verified {count}/{}", controls.len()),
        Err(code) => format!("authorization control incomplete: {code}"),
    };
    // Once running, even a usage query or mailbox failure must attempt a
    // fenced terminal transition. Otherwise the target lane and reserved
    // request budget remain occupied after the coordinator has failed.
    let finalization = (|| -> Result<(), String> {
        let spent: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1",
                [&child.run_id],
                |row| row.get(0),
            )
            .map_err(|_| "authorization_usage_unavailable".to_string())?;
        let usage = AgentTokenUsage::default();
        settle_child_usage(&connection, &session.lease, &child, &usage)?;
        complete_target_child_delivery(
            &connection,
            &session.lease,
            &child,
            "authorization_result",
            &format!("authorization:{}", child.assignment_id),
            &serde_json::json!({"summary":summary,"verified":result.as_ref().ok(),"requestCount":spent}),
            result.is_ok(),
            &summary,
        )
    })();
    if let Err(error) = finalization {
        if let Err(cleanup) =
            stop_failed_child_preserving_usage(&connection, &session.lease, &child, &error)
        {
            return Err(format!("{error};authorization_child_cleanup:{cleanup}"));
        }
        return Err(error);
    }
    result.map(|_| ())
}
