// Bootstrap request bytes remain exactly g2. Only a new captured output adds a
// new phase and permitted suggestion to its own independently paid round.
fn native_coordinator_basis_for_frame_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<JsonValue, String> {
    let mut basis = native_coordinator_basis_on(db, context, actor)?;
    if let Some(frame) = frame {
        frame.verify(db, context, actor)?;
        basis["phase"] = frame.kind.into();
        basis["changedFact"] = frame.fact();
        basis["permittedSuggestion"] = frame.step().into();
    }
    if basis.to_string().len() > 256 * 1024 {
        return Err("root_tick_basis_too_large".into());
    }
    Ok(basis)
}
fn native_coordinator_verify_frame_basis(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    captured: &JsonValue,
) -> Result<(), String> {
    if frame.is_none() {
        return native_coordinator_verify_original_basis(db, context, actor, captured);
    }
    if native_coordinator_basis_for_frame_on(db, context, actor, frame)? != *captured {
        return Err("root_tick_original_basis_changed".into());
    }
    Ok(())
}

fn native_coordinator_require_original_model_on(
    db: &rusqlite::Connection,
    root: &str,
    request: &JsonValue,
) -> Result<(), String> {
    let text: String = db
        .query_row(
            "SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1
        AND round=1 AND phase='request'",
            [root],
            |r| r.get(0),
        )
        .map_err(|_| "root_tick_original_model_missing")?;
    let fact: JsonValue =
        serde_json::from_str(&text).map_err(|_| "root_tick_original_model_invalid")?;
    let canonical_fact = fact.to_string();
    if canonical_fact != text {
        return Err("root_tick_original_model_invalid".into());
    }
    let original = &fact["request"]["request"];
    for key in [
        "model",
        "endpointBinding",
        "deployment",
        "credentialBinding",
        "proxyBinding",
        "maxContextTokens",
        "maxOutputTokens",
        "temperature",
        "timeoutPolicy",
        "schemaHash",
    ] {
        if original.get(key).is_none() || original[key] != request[key] {
            return Err("root_tick_original_model_changed".into());
        }
    }
    Ok(())
}
fn native_coordinator_request_for_frame(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<NativeCoordinatorRequest, String> {
    if let Some(frame) = frame {
        NativeCoordinatorTrigger::classify(frame)?;
    }
    let mut prepared = native_coordinator_request(context, actor)?;
    let Some(frame) = frame else {
        return Ok(prepared);
    };
    let db = db::open(&context.db_path)?;
    prepared.basis = native_coordinator_basis_for_frame_on(&db, context, actor, Some(frame))?;
    prepared.request.messages[1] = json!({"role":"user","content":json!({"basis":prepared.basis,
        "requiredOutput":crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary::schema(),
        "permittedSuggestion":frame.step(),"instruction":"Use the permitted suggestion exactly only if useful. Empty suggestions means defer. This suggestion grants no new scope, capability, budget or target request; Rust validates the actual dispatch."}).to_string()});
    prepared.fact["purpose"] = "coordinator_changed_fact_v1".into();
    prepared.fact["basisHash"] =
        crate::agent_runtime::store::stable_hash(&prepared.basis.to_string()).into();
    prepared.fact["messages"] = json!(prepared.request.messages);
    let bytes = serde_json::to_vec(&json!({"messages":prepared.request.messages,"tools":prepared.fact["tools"]}))
        .map_err(|e| e.to_string())?
        .len();
    let output = prepared
        .client
        .profile()
        .max_output_tokens
        .ok_or("root_tick_output_invalid")?;
    prepared.estimate = i64::try_from(bytes)
        .ok()
        .and_then(|n| n.checked_add(i64::try_from(output).ok()?))
        .filter(|n| *n > 0)
        .ok_or("root_tick_estimate_invalid")?;
    Ok(prepared)
}

fn native_coordinator_tick_for_frame(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
) -> Result<NativeCoordinatorTickReceipt, String> {
    native_coordinator_tick_with_frame(context, actor, Some(frame))
}
