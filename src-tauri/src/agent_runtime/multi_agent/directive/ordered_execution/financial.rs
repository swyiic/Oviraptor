use super::*;
pub(crate) fn metadata(db: &Connection, job: &ActionJob) -> Result<Value, String> {
    if db.is_autocommit() {
        return Err("ordered_transaction_required".into());
    }
    let owner = budget::root::RootOwner::load_original(db, &job.scope.root_run_id)?;
    owner.require_original_coordinator(db, &job.scope)?;
    let (finance, native): (String, String) = db
        .query_row(
            "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&job.scope.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let worker = attempts::current(db, &job.scope, &job.child.assignment_id)?;
    let original = budget::receipts::original_owner(
        db,
        &job.child.run_id,
        &job.scope,
        &job.child.assignment_id,
    )?;
    original.verify(db)?;
    if original.attempt_id() != worker.id || worker.child_run_id != job.child.run_id {
        return Err("ordered_original_worker_changed".into());
    }
    binding::verify_request(db, job, true)?;
    let previous = if job.action.order == 2 {
        let first = load(db, &job.directive_id, 1)?.ok_or("ordered_predecessor_missing")?;
        let receipt =
            receipts::verified(db, &first)?.ok_or("ordered_predecessor_receipt_missing")?;
        if receipt["outcome"] != "valid_advisory" {
            return Err("ordered_predecessor_not_valid".into());
        }
        let receipt_hash = stable_hash(&receipt.to_string());
        if job.input["previousAssessment"]["receiptId"] != receipt["receiptId"]
            || job.input["previousAssessment"]["receiptHash"] != receipt_hash
            || job.input["previousAssessment"]["actionId"] != first.action.action_id
            || job.input["previousAssessment"]["advisoryOnly"] != true
            || job.input["previousAssessment"]["reviewApproved"] != false
            || job.input["frozenEvidence"] != first.input["frozenEvidence"]
        {
            return Err("ordered_predecessor_input_changed".into());
        }
        json!({"receiptId":receipt["receiptId"],"receiptHash":receipt_hash,"actionId":first.action.action_id})
    } else {
        if !job.input["previousAssessment"].is_null() {
            return Err("ordered_first_input_changed".into());
        }
        Value::Null
    };
    let proof = json!({"schemaVersion":3,"kind":"human_ordered_readonly_assessment_dispatch",
      "directiveId":job.directive_id,"sourceDraftId":job.draft.id,"confirmedRevision":job.draft.revision,"confirmedHash":job.draft.draft_hash,
      "planHash":job.plan.plan_hash,"actionId":job.action.action_id,"order":job.action.order,"predecessor":previous,
      "scanId":job.scope.scan_id,"attempt":job.scope.attempt_number,"targetKey":job.scope.target_key,"rootRunId":job.scope.root_run_id,
      "rootFinanceId":finance,"rootNativeContractHash":stable_hash(&native),"coordinatorEpoch":job.scope.lease_epoch,"coordinatorFence":job.scope.fencing_token,
      "assignmentId":job.child.assignment_id,"childRunId":job.child.run_id,"role":job.child.role.as_str(),
      "leaseAttemptId":worker.id,"workerId":worker.worker_id,"workerEpoch":worker.lease_epoch,"workerFence":worker.fencing_token,
      "requestMessageId":job.request_message_id,"inputHash":stable_hash(&job.input.to_string()),"evidenceRevision":job.draft.revision,
      "modelRequests":1,"modelTokenCeiling":4000,"maxOutputTokens":512,"targetRequests":0,"advisoryOnly":true,"reviewApproved":false});
    if let Some((raw, hash)) = db
        .query_row(
            "SELECT request_json,request_hash FROM agent_specialist_calls WHERE child_run_id=?1",
            [&job.child.run_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        let request: Value =
            serde_json::from_str(&raw).map_err(|_| "ordered_original_dispatch_invalid")?;
        if stable_hash(&raw) != hash || request["humanDirectiveDispatch"] != proof {
            return Err("ordered_original_dispatch_invalid".into());
        }
    }
    Ok(proof)
}
pub(crate) fn authorize(db: &Connection, job: &ActionJob, proof: &Value) -> Result<(), String> {
    current(db, &job.scope)?;
    let current_job =
        load(db, &job.directive_id, job.action.order)?.ok_or("ordered_job_missing")?;
    if current_job.state != "executing"
        || current_job.child != job.child
        || current_job.input != job.input
        || metadata(db, &current_job)? != *proof
    {
        return Err("ordered_dispatch_binding_changed".into());
    }
    attempts::require_live_for_run(db, &job.child.run_id)?;
    Ok(())
}
pub(super) fn saved(
    db: &Connection,
    job: &ActionJob,
) -> Result<specialist::StoredResponse, String> {
    let proof = metadata(db, job)?;
    let saved = specialist::received_for_owned_proposal(db, &job.scope, &job.child, &proof)?;
    if !matches!(
        saved.rejection.as_str(),
        "" | "empty_response" | "unexpected_tool_calls" | "response_too_large"
    ) {
        return Err("ordered_original_response_withheld".into());
    }
    Ok(saved)
}
pub(crate) fn received(
    db: &Connection,
    scope: &CoordinatorLease,
    id: &str,
    order: i64,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<ActionJob, String> {
    let private = writer::private_connection(db, Mode::Receive)?;
    let db = &private;
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    current(&tx, scope)?;
    check(&tx)?;
    let job = load(&tx, id, order)?.ok_or("ordered_job_missing")?;
    if job.scope.root_run_id != scope.root_run_id
        || job.scope.lease_epoch != scope.lease_epoch
        || job.scope.fencing_token != scope.fencing_token
        || !matches!(job.state.as_str(), "executing" | "received")
    {
        return Err("ordered_response_state_conflict".into());
    }
    let saved = saved(&tx, &job)?;
    let response = super::super::proposals::owned::assessment(&saved.text);
    let response = if saved.rejection.is_empty() {
        response
    } else {
        super::super::proposals::owned::assessment("")
    };
    let usage = saved.usage.as_json();
    let guard = if job.state == "executing" {
        Some(Writer::install(&private, scope, id, &job.child, Mode::Receive)?)
    } else {
        None
    };
    if job.state == "executing" {
        let changed=tx.execute("UPDATE agent_directive_ordered_actions SET state='received',response_json=?1,usage_json=?2
         WHERE action_id=?3 AND state='executing'",params![response.to_string(),usage.to_string(),job.action.action_id]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("ordered_response_not_persisted".into());
        }
    } else if response != job.response || usage != job.usage {
        return Err("ordered_response_replay_conflict".into());
    }
    let stored = load(&tx, id, order)?.ok_or("ordered_response_not_persisted")?;
    if stored.state != "received" || stored.response != response || stored.usage != usage {
        return Err("ordered_response_not_persisted".into());
    }
    event_exact(&tx, &stored, "orderedState", &json!("received"))?;
    if let Some(guard) = &guard {
        guard.verify()?
    }
    check(&tx)?;
    current(&tx, scope)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(stored)
}
pub(crate) fn recover_received(
    db: &Connection,
    scope: &CoordinatorLease,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    let ids: Vec<(String, i64)> = {
        let mut q=db.prepare("SELECT x.directive_id,x.action_order FROM agent_directive_ordered_actions x
       JOIN agent_user_directives d ON d.id=x.directive_id JOIN agent_specialist_calls c ON c.child_run_id=x.child_run_id AND c.assignment_id=x.assignment_id
       WHERE x.state='executing' AND c.state='received' AND d.status='assigned' AND d.root_run_id=?1 AND d.scan_id=?2 AND d.attempt_number=?3 ORDER BY x.rowid")
       .map_err(|e|e.to_string())?;
        let rows = q
            .query_map(
                params![scope.root_run_id, scope.scan_id, scope.attempt_number],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    for (id, order) in ids {
        received(db, scope, &id, order, &check)?;
    }
    Ok(())
}
pub(crate) fn result_for_mailbox(
    db: &Connection,
    scope: &CoordinatorLease,
    action: &str,
) -> Result<(ActionJob, Value), String> {
    current(db, scope)?;
    let (id,order):(String,i64)=db.query_row("SELECT directive_id,action_order FROM agent_directive_ordered_actions WHERE action_id=?1",
       [action],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"ordered_job_missing")?;
    let job = load(db, &id, order)?.ok_or("ordered_job_missing")?;
    if job.scope.root_run_id != scope.root_run_id
        || job.scope.lease_epoch != scope.lease_epoch
        || job.scope.fencing_token != scope.fencing_token
        || !matches!(job.state.as_str(), "received" | "completed" | "failed")
    {
        return Err("ordered_result_missing".into());
    }
    verify_projection(db, &job)?;
    let payload = result_payload(&job);
    Ok((job, payload))
}
pub(super) fn verify_projection(db: &Connection, job: &ActionJob) -> Result<(), String> {
    let saved = saved(db, job)?;
    let wanted = super::super::proposals::owned::assessment(if saved.rejection.is_empty() {
        &saved.text
    } else {
        ""
    });
    let events:i64=db.query_row("SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&job.child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if job.response != wanted || job.usage != saved.usage.as_json() || events != 1 {
        return Err("ordered_original_projection_changed".into());
    }
    Ok(())
}
pub(super) fn result_payload(job: &ActionJob) -> Value {
    json!({"directiveId":job.directive_id,"actionId":job.action.action_id,"order":job.action.order,"planHash":job.plan.plan_hash,
      "summary":job.response["summary"],"assessment":job.response,"advisoryOnly":true,"coverageVerified":false,"targetRequests":0,
      "independentReviewApproved":false})
}
