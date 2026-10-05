// Paid model semantics only propose a bounded local action. Rust issues it.
#[allow(clippy::too_many_arguments)]
fn native_coordinator_schedule_decision(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
    role: crate::agent_runtime::contract::AgentRole,
    lane: crate::agent_runtime::contract::AgentLane,
    trigger: &str,
    task: &JsonValue,
    revision: i64,
    capabilities: &[String],
    tokens: i64,
    requests: i64,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler, store};
    let expected = match role {
        AgentRole::WebExecutor => "dispatch:web_executor",
        AgentRole::DeepInvestigator => "dispatch:deep_investigator",
        _ => return Err("root_decision_role_not_bounded".into()),
    };
    if expected != frame.step()
        || !decision.summary.as_json()["suggestions"]
            .as_array()
            .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(expected)))
    {
        return Err("root_decision_did_not_select_step".into());
    }
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let verify = || -> Result<(), String> {
        native_coordinator_tick_authority(&tx, context, actor)?;
        frame.verify(&tx, context, actor)?;
        decision.tick.require_executable(&tx)?;
        if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_decision_publication_changed".into());
        }
        Ok(())
    };
    verify()?;
    let allocation = if role == AgentRole::WebExecutor {
        native_coordinator_executor_allocation(&tx, actor, trigger, revision)?
    } else { None };
    let (tokens,requests)=allocation.map_or((tokens,requests),|(t,r,_)|(t,r));
    let rust_policy = native_coordinator_dispatch_policy_on(&tx, context, actor, frame, decision, role, tokens, requests, allocation.is_some())?;
    let mut task = task.clone();
    let object = task.as_object_mut().ok_or("root_decision_task_invalid")?;
    if object.contains_key("rootDecision") {
        return Err("root_decision_task_shadow".into());
    }
    object.insert(
        "rootDecision".into(),
        json!({"eventSequence":decision.event_sequence,
        "summaryHash":store::stable_hash(&decision.summary.as_json().to_string()),
        "frameHash":store::stable_hash(&frame.fact().to_string()),"step":expected,"rustPolicy":rust_policy}),
    );
    let child = native_coordinator_dispatch_protect(&tx, || {
        verify()?;
        let child = scheduler::schedule_child_in_transaction(
            &tx,
            actor,
            role,
            lane,
            trigger,
            &task,
            revision,
            capabilities,
            tokens,
            requests,
        )?;
        if let Some((_,_,fresh))=allocation {
            if crate::agent_runtime::multi_agent::budget::model::original_web_grant(&tx,actor,&child.assignment_id)? != (tokens,requests) {return Err("executor_allocation_original_grant_conflict".into());}
            if fresh {
                let (retained_tokens,retained_requests)=native_coordinator_executor_retained_floor(&tx,actor)?;
                crate::agent_runtime::multi_agent::budget::root::require_child_capacity(&tx,&actor.root_run_id,retained_tokens,retained_requests)?;
            }
        }
        verify()?; // Same captured proof, after every grant/reservation/trigger write.
        Ok(child)
    })?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(child)
}

fn native_coordinator_root_context(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> AgentRunContext {
    let mut root = context.clone();
    root.run = Some(AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: actor.root_run_id.clone(),
    });
    root.run_budget = None;
    root
}

include!("coordinator_executor_allocation.rs");

// Exact policy used both for issuance and original paid-issuance inspection.
#[allow(clippy::too_many_arguments)]
fn native_coordinator_dispatch_policy_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
    role: crate::agent_runtime::contract::AgentRole,
    tokens: i64,
    requests: i64,
    dynamic: bool,
) -> Result<JsonValue, String> {
    let mut limits = [None; 3];
    for (limit, dimension) in
        limits
            .iter_mut()
            .zip(["model_input_tokens", "model_requests", "target_requests"])
    {
        *limit = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![actor.root_run_id, dimension],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
    }
    let mode=crate::agent_runtime::web_mode::root::read(db,&actor.root_run_id)?.ok_or("web_mode_root_missing")?;
    let rule=NativeCoordinatorModelCostRule::from_original(&mode.bootstrap_dispatch().unwrap_or(JsonValue::Null))?;
    let mut rust_policy = match rule {
        NativeCoordinatorModelCostRule::OriginalConservative => native_coordinator_rust_dispatch_policy(
            context, frame, decision, role, tokens, requests, limits)?,
        NativeCoordinatorModelCostRule::NativeUnlimitedNoScarcity => native_coordinator_rust_dispatch_policy_with_rule(
            context, frame, decision, role, tokens, requests, limits, rule)?,
    };
    if rule == NativeCoordinatorModelCostRule::NativeUnlimitedNoScarcity {
        rust_policy["modelCostRule"]=json!(NATIVE_UNLIMITED_MODEL_COST_RULE);
        rust_policy["costBasis"]["modelNormalization"]=json!("share_of_finite_original_model_ceiling");
        rust_policy["costBasis"]["modelFeesTrackedIndependently"]=json!(true);
        rust_policy["costBasis"]["unboundedModelDimensions"]=json!(["model_input_tokens","model_requests"].into_iter().zip(limits).filter_map(|(name,limit)|limit.is_none().then_some(name)).collect::<Vec<_>>());
    }
    if dynamic {
        rust_policy["allocationRule"]=json!("current_balance_after_reviewer_floor_and_ordered_proposals");
        rust_policy["reviewerFloor"]=json!({"modelTokens":15_000,"modelRequests":1});
        let (retained_tokens,retained_requests)=native_coordinator_executor_retained_floor(db,actor)?;
        if retained_tokens>15_000 || retained_requests>1 {
            rust_policy["rootSupervisionFloor"]=json!({"modelTokens":retained_tokens-15_000,"modelRequests":retained_requests-1});
        }
    }
    Ok(rust_policy)
}
