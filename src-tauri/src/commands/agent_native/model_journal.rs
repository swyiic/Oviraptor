// Web per-call billing facts. Receipts settle costs under the original binding;
// they cannot renew authority, restore tools, or authorize another model call.
fn native_model_journal_insert(
    tx: &rusqlite::Transaction<'_>,
    context: &AgentRunContext,
    call: &NativeModelBudgetAdmission,
    phase: &str,
    receipt: &JsonValue,
) -> Result<(), String> {
    let child = &context.run.as_ref().ok_or("tool_run_not_found")?.run_id;
    let changed=tx.execute("INSERT INTO agent_web_model_journal
        (call_id,root_run_id,assignment_id,child_run_id,lease_epoch,fencing_token,round,request_hash,phase,receipt_json)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![call.call_id,call.lease.root_run_id,call.assignment,child,call.lease.lease_epoch,
            call.lease.fencing_token,call.round,call.request_hash,phase,receipt.to_string()]).map_err(|e|format!("web_model_journal_write:{e}"))?;
    if changed != 1 {
        return Err("web_model_journal_write_missing".into());
    }
    native_model_journal_verify(tx, context, call, phase, receipt)
}

fn native_model_journal_verify(
    db: &rusqlite::Transaction<'_>,
    context: &AgentRunContext,
    call: &NativeModelBudgetAdmission,
    phase: &str,
    receipt: &JsonValue,
) -> Result<(), String> {
    let child = &context.run.as_ref().ok_or("tool_run_not_found")?.run_id;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE call_id=?1
        AND root_run_id=?2 AND assignment_id=?3 AND child_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6
        AND round=?7 AND request_hash=?8 AND phase=?9 AND receipt_json=?10)",
        params![call.call_id,call.lease.root_run_id,call.assignment,child,call.lease.lease_epoch,
            call.lease.fencing_token,call.round,call.request_hash,phase,receipt.to_string()],|r|r.get(0))
        .map_err(|e|e.to_string())?;
    if !exact {
        return Err("web_model_journal_binding_conflict".into());
    }
    crate::agent_runtime::multi_agent::budget::receipts::original_owner(
        db,
        child,
        &call.lease,
        &call.assignment,
    )?
    .verify(db)
}

fn native_model_claim_on(
    tx: &rusqlite::Transaction<'_>,
    context: &AgentRunContext,
    call: &NativeModelBudgetAdmission,
) -> Result<crate::agent_runtime::execution_owner::NativeInvocationOwner, String> {
    let child = &context.run.as_ref().ok_or("tool_run_not_found")?.run_id;
    let history: (i64, i64) = tx
        .query_row(
            "SELECT
        (SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'),
        (SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed')",
            [child],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if call.round < 1 || history != (call.round - 1, call.round - 1) {
        return Err("budget_history_requires_reconciliation".into());
    }
    let used: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_web_model_journal
        WHERE child_run_id=?1 AND round=?2)",
            params![child, call.round],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if used {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    // History and the original financial worker are checked before any inode
    // can be created. A rejected replay must not manufacture an exit proof.
    crate::agent_runtime::multi_agent::budget::receipts::original_owner(tx,child,&call.lease,&call.assignment)?.verify(tx)?;
    native_model_reserve_unbounded_on(tx, context, call)?;
    let owner=crate::agent_runtime::execution_owner::claim_native_invocation(&context.db_path,
        &context.scan_id,context.attempt_number,WEB_MODEL_INVOCATION_KIND,child)?;
    native_model_journal_insert(tx, context, call, "dispatch", &serde_json::json!({}))?;
    Ok(owner)
}

fn native_model_terminal_receipt(
    context: &AgentRunContext,
    call: &NativeModelBudgetAdmission,
    phase: &str,
    response: Option<&crate::agent_runtime::model::gateway::ModelResponse>,
    code: &str,
) -> Result<bool, String> {
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_model_journal_verify(&tx, context, call, "dispatch", &serde_json::json!({}))?;
    let owner = crate::agent_runtime::multi_agent::budget::receipts::original_owner(
        &tx,
        &context.run.as_ref().ok_or("tool_run_not_found")?.run_id,
        &call.lease,
        &call.assignment,
    )?;
    let source = format!(
        "web-model:{}:{}:{}",
        call.assignment, call.round, call.request_hash
    );
    let receipt = if let Some(response) = response {
        let response_hash=crate::agent_runtime::store::stable_hash(&serde_json::json!({"text":response.text,
            "calls":response.tool_calls.iter().map(|c|serde_json::json!({"id":c.id,"name":c.name,"arguments":c.arguments})).collect::<Vec<_>>(),
            "finishReason":response.finish_reason,"usage":response.usage.as_json(),"usageReported":response.usage_reported}).to_string());
        serde_json::json!({"responseHash":response_hash,"usage":response.usage.as_json(),"usageReported":response.usage_reported})
    } else {
        if phase == "uncertain" {
            crate::agent_runtime::multi_agent::budget::receipts::forfeit_original_call(
                &tx, &owner, &source,
            )?;
        }
        serde_json::json!({"code":code})
    };
    native_model_journal_insert(&tx, context, call, phase, &receipt)?;
    if let Some(response) = response {
        let hash = receipt["responseHash"]
            .as_str()
            .ok_or("web_model_receipt_hash_missing")?;
        crate::agent_runtime::multi_agent::budget::receipts::record_original(
            &tx,
            &owner,
            &source,
            hash,
            &response.usage,
            response.usage_reported,
        )?;
        native_model_journal_verify(&tx, context, call, phase, &receipt)?;
    }
    let unresolved = owner.is_unresolved(&tx)?;
    owner.verify(&tx)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(unresolved)
}
