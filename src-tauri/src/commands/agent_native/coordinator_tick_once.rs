// One physical SDK round; local deliberation never retries an original bill.
fn native_coordinator_tick_once(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    mut prepared: NativeCoordinatorRequest,
    original_basis: &JsonValue,
    history: &[NativeCoordinatorTickReceipt],
) -> Result<NativeCoordinatorTickReceipt, String> {
    use crate::agent_runtime::{
        model::OneShotFailure,
        multi_agent::budget::root::model::tick::{Begin, Tick},
    };

    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    // Read original C/financial identity before any first invocation ownership.
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        &tx, &actor.root_run_id,
    )?.require_original_coordinator(&tx, actor)?;
    // Historical calls only probe their existing original inode. Keep that
    // guard through admission, SDK, cost/log and local publication/replay.
    let original_invocation = crate::agent_runtime::multi_agent::budget::root::model::tick::require_idle_original(&tx, actor)?;
    prepared.basis=Tick::with_budget_snapshot(&tx,&actor.root_run_id,&prepared.basis)?;
    if prepared.basis.get("budgetSnapshot").is_some() {
        let mut input:JsonValue=serde_json::from_str(prepared.request.messages[1]["content"].as_str().ok_or("root_tick_messages_invalid")?).map_err(|_|"root_tick_messages_invalid")?;
        input["basis"]["budgetSnapshot"]=prepared.basis["budgetSnapshot"].clone();
        prepared.request.messages[1]["content"]=input.to_string().into();
        prepared.fact["basisHash"]=Tick::basis_hash(&prepared.basis).into();
        prepared.fact["messages"]=json!(prepared.request.messages);
        let bytes=serde_json::to_vec(&json!({"messages":prepared.request.messages,"tools":prepared.fact["tools"]})).map_err(|e|e.to_string())?.len();
        if bytes>96*1024 {return Err("root_local_context_limit".into());}
        let output=prepared.client.profile().max_output_tokens.ok_or("root_tick_output_invalid")?;
        prepared.estimate=i64::try_from(bytes).ok().and_then(|n|n.checked_add(i64::try_from(output).ok()?)).filter(|n|*n>0).ok_or("root_tick_estimate_invalid")?;
    }
    let round = Tick::next_round(&tx, &actor.root_run_id, &prepared.basis)?;
    let begin = Tick::begin(
        &tx,
        &actor.root_run_id,
        round,
        &prepared.basis,
        &prepared.fact,
        prepared.estimate,
    )?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    // Only validated first admission may create invocation ownership. No
    // rejected input or historical replay can manufacture original exit proof.
    let _root_invocation = match original_invocation {
        Some(guard) => guard,
        None => crate::agent_runtime::execution_owner::claim_native_invocation(
            &context.db_path, &actor.scan_id, actor.attempt_number,
            crate::agent_runtime::multi_agent::budget::root::model::tick::INVOCATION_KIND,
            &actor.root_run_id,
        )?,
    };
    tx.commit().map_err(|e| e.to_string())?;
    let mut model_log = None;
    let (tick, decision, replayed) = match begin {
        Begin::Saved(tick, decision) => (tick, *decision, true),
        Begin::Dispatch(tick) => {
            prepared.request.request_timeout = Some(tick.remaining());
            let deadline = Instant::now() + tick.remaining();
            let owned = context.clone();
            let original = actor.clone();
            let inflight = tick.clone();
            let human_frame = frame.filter(|f| f.kind == "human-directive").cloned();
            let scan =
                agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
            let cancel = CancelToken::from_checker(move || {
                scan.is_cancelled()
                    || Instant::now() >= deadline
                    || !db::open(&owned.db_path).is_ok_and(|db| {
                        inflight.require_executable(&db).is_ok()
                            && native_coordinator_tick_authority(&db, &owned, &original).is_ok()
                            && human_frame.as_ref().is_none_or(|f| f.verify(&db, &owned, &original).is_ok())
                    })
            });
            let log = crate::agent_runtime::model::diagnostics::ModelLog::coordinator(
                &context.db_path,
                &actor.root_run_id,
                round,
            );
            let observer = log.observer();
            model_log = Some(log);
            match prepared.client.complete_once_observed_with_lifecycle(
                &prepared.request,
                &cancel,
                observer,
            ) {
                Ok(response) => {
                    let db = db::open(&context.db_path)?;
                    tick.persist_received_invoice(&db, &response)?;
                    let tx = rusqlite::Transaction::new_unchecked(
                        &db,
                        rusqlite::TransactionBehavior::Immediate,
                    )
                    .map_err(|e| e.to_string())?;
                    // Financial and validated semantic receipt bind the captured
                    // original call. Cancellation/takeover cannot erase a bill.
                    let received = tick.received(&tx, &response)?;
                    tx.commit().map_err(|e| e.to_string())?;
                    if let Some(log) = &model_log {
                        log.cost_saved();
                    }
                    tick.persist_terminal_capture(&db, received.terminal)?;
                    #[cfg(test)]
                    native_coordinator_tick_paid_checkpoint();
                    if received.unknown {
                        return Err("budget_indeterminate_requires_reconciliation".into());
                    }
                    (tick, received.decision?, false)
                }
                Err(error) => {
                    let (phase, code) = match &error {
                        OneShotFailure::BeforeTransport(error) => ("unsent", error.code()),
                        OneShotFailure::TransportOutcomeUnknown(error) => {
                            ("uncertain", error.code())
                        }
                    };
                    let db = db::open(&context.db_path)?;
                    let tx = rusqlite::Transaction::new_unchecked(
                        &db,
                        rusqlite::TransactionBehavior::Immediate,
                    )
                    .map_err(|e| e.to_string())?;
                    let terminal = tick.stopped(&tx, phase, code)?;
                    tx.commit().map_err(|e| e.to_string())?;
                    if let Some(log) = &model_log {
                        log.cost_saved();
                    }
                    tick.persist_terminal_capture(&db, terminal)?;
                    return Err(format!("root_tick_{phase}:{code}"));
                }
            }
        }
    };
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    let sequence = tick.publish(&tx, &decision)?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    tick.require_executable(&tx)?;
    if tick.published(&tx, &decision)? != Some(sequence) {
        return Err("root_tick_publication_unconfirmed".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    if let Some(log) = &model_log {
        log.finish(true);
    }
    Ok(NativeCoordinatorTickReceipt {
        summary: decision.summary.clone(),
        event_sequence: sequence,
        replayed,
        tick,
        saved: decision,
    })
}
