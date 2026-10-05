// Frozen bounded local reasoning; actual dispatch remains with Rust policy.
fn native_coordinator_tick(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<NativeCoordinatorTickReceipt, String> {
    native_coordinator_tick_with_frame(context, actor, None)
}
fn native_coordinator_tick_with_frame(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let original = native_coordinator_request_for_frame(context, actor, frame)?;
    let original_basis = original.basis.clone();
    if original_basis.get("localDeliberation").is_none() {
        return native_coordinator_tick_once(context, actor, frame, original, &original_basis, &[]);
    }
    let mut history: Vec<NativeCoordinatorTickReceipt> = Vec::new();
    let mut prepared = original;
    let mut calls_used = 0usize;
    for step in 0..3 {
        let db = db::open(&context.db_path)?;
        let parents = history
            .iter()
            .map(|r| r.tick.local_parent_proof(&db, &r.saved, r.event_sequence))
            .collect::<Result<Vec<_>, _>>()?;
        prepared.basis["localParents"] = json!(parents);
        prepared.basis["localRound"] = json!(step);
        prepared.basis["localCallsUsed"] = json!(calls_used);
        prepared.basis["localHistory"] = json!(history
            .iter()
            .map(|r| r
                .saved
                .local_step
                .as_ref()
                .expect("only local steps enter history")
                .as_json())
            .collect::<Vec<_>>());
        prepared.fact["basisHash"] =
            crate::agent_runtime::store::stable_hash(&prepared.basis.to_string()).into();
        prepared.fact["messages"] = json!(prepared.request.messages);
        let bytes = serde_json::to_vec(
            &json!({"messages":prepared.request.messages,"tools":prepared.fact["tools"]}),
        )
        .map_err(|e| e.to_string())?
        .len();
        if bytes > 96 * 1024 {
            return Err("root_local_context_limit".into());
        }
        let output = prepared
            .client
            .profile()
            .max_output_tokens
            .ok_or("root_tick_output_invalid")?;
        prepared.estimate = i64::try_from(bytes)
            .ok()
            .and_then(|b| b.checked_add(i64::try_from(output).ok()?))
            .ok_or("root_tick_estimate_invalid")?;
        let receipt = native_coordinator_tick_once(
            context,
            actor,
            frame,
            prepared,
            &original_basis,
            &history,
        )?;
        let Some(local) = receipt.saved.local_step.as_ref() else {
            return Ok(receipt);
        };
        calls_used += local.count();
        if calls_used > 4 {
            return Err("root_local_call_limit".into());
        }
        history.push(receipt);
        if step == 2 {
            return Err("root_local_react_round_limit".into());
        }
        // Rebuild configured provider/profile and immutable original facts, then
        // append only original paid, validated local tool results.
        prepared = native_coordinator_request_for_frame(context, actor, frame)?;
        if prepared.basis != original_basis {
            return Err("root_tick_original_basis_changed".into());
        }
        for receipt in &history {
            prepared.request.messages.extend(
                receipt
                    .saved
                    .local_step
                    .as_ref()
                    .expect("validated local step")
                    .messages(),
            );
        }
    }
    Err("root_local_react_round_limit".into())
}
fn native_coordinator_verify_local_history(
    db: &rusqlite::Connection,
    history: &[NativeCoordinatorTickReceipt],
) -> Result<(), String> {
    for receipt in history {
        receipt.tick.require_executable(db)?;
        if receipt.saved.local_step.is_none()
            || receipt.tick.published(db, &receipt.saved)? != Some(receipt.event_sequence)
        {
            return Err("root_local_original_history_changed".into());
        }
    }
    Ok(())
}
