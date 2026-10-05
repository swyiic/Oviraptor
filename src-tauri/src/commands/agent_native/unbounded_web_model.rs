// Per-call reservations on original Native5 unbounded model dimensions only.
fn native_model_unbounded_dimensions_on(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<Option<[bool; 4]>, String> {
    let Some(mode) = crate::agent_runtime::web_mode::root::read(db, root)? else {
        return Ok(None);
    };
    let rule = NativeCoordinatorModelCostRule::from_original(
        &mode.bootstrap_dispatch().unwrap_or(JsonValue::Null),
    )?;
    if rule != NativeCoordinatorModelCostRule::NativeUnlimitedNoScarcity {
        return Ok(None);
    }
    crate::agent_runtime::multi_agent::budget::limits::verify_root_contract(db, root)?;
    let mut flags = [false; 4];
    for (flag, dimension) in flags
        .iter_mut()
        .zip(crate::agent_runtime::multi_agent::budget::DIMENSIONS[..4].iter())
    {
        let limit: Option<i64> = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![root, dimension],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        *flag = limit.is_none();
    }
    Ok(Some(flags))
}
fn native_model_unbounded_window(context: &AgentRunContext) -> Result<bool, String> {
    let Some(run) = context.run.as_ref() else {
        return Ok(false);
    };
    let db = db::open(&context.db_path)?;
    let row =
        crate::agent_runtime::store::load_run(&db, &run.run_id)?.ok_or("tool_run_not_found")?;
    if row.orchestration_policy != crate::agent_runtime::contract::MultiAgentPolicy::Multi
        || row.role != crate::agent_runtime::contract::AgentRole::WebExecutor
    {
        return Ok(false);
    }
    Ok(native_model_unbounded_dimensions_on(&db, &row.root_run_id)?
        .is_some_and(|flags| flags.into_iter().any(|v| v)))
}
fn native_model_reserve_unbounded_on(
    tx: &rusqlite::Transaction<'_>,
    context: &AgentRunContext,
    call: &NativeModelBudgetAdmission,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let flags = native_model_unbounded_dimensions_on(tx, &call.lease.root_run_id)?;
    let Some(flags) = flags.filter(|flags| flags.iter().any(|v| *v)) else {
        return Ok(());
    };
    let estimate = call
        .unbounded_estimate
        .filter(|n| *n > 0)
        .ok_or("budget_unbounded_model_estimate_missing")?;
    let child = &context.run.as_ref().ok_or("tool_run_not_found")?.run_id;
    let original = budget::receipts::original_owner(tx, child, &call.lease, &call.assignment)?;
    original.verify(tx)?;
    let grant = budget::model::original_web_grant(tx, &call.lease, &call.assignment)?;
    if flags[0] && grant.0 != 0 || flags[3] && grant.1 != 0 {
        return Err("budget_unbounded_original_grant_conflict".into());
    }
    let source = format!(
        "web-model:{}:{}:{}",
        call.assignment, call.round, call.request_hash
    );
    native_model_unbounded_reservation_protect(tx, || {
        for (index, dimension) in budget::DIMENSIONS[..4].iter().enumerate() {
            if !flags[index] {
                continue;
            }
            let required = if index == 3 { 1 } else { estimate };
            let held = budget::balance(
                tx,
                &call.lease.root_run_id,
                Some(&call.assignment),
                dimension,
            )?
            .reserved;
            if required > held {
                budget::append(
                    tx,
                    &call.lease,
                    &call.assignment,
                    dimension,
                    budget::Kind::Reserve,
                    required - held,
                    &format!("reserve:{source}:{dimension}"),
                    &source,
                )?;
            }
        }
        original.verify(tx)
    })
}

// This new financial write cannot give a trigger unrelated write authority.
fn native_model_unbounded_reservation_protect<T>(
    tx: &rusqlite::Transaction<'_>,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    tx.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert {
            table_name: "agent_budget_entries",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))
    .map_err(|e| e.to_string())?;
    let result = work();
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}
