use super::*;
fn canonical(db: &Connection, job: &ActionJob, receipt_id: &str) -> Result<Value, String> {
    if !matches!(job.state.as_str(), "completed" | "failed") {
        return Err("ordered_terminal_receipt_required".into());
    }
    financial::verify_projection(db, job)?;
    let proof = metadata(db, job)?;
    let (request_hash,response_hash,sequence):(String,String,i64)=db.query_row(
      "SELECT request_hash,response_hash,event_sequence FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",
      params![job.child.assignment_id,job.child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"ordered_original_fee_receipt_missing")?;
    let expected = if job.response["valid"] == true {
        "completed"
    } else {
        "failed"
    };
    let settled:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
      WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND a.state=?4 AND r.status='terminal'
      AND r.terminal_state=?4 AND a.budget_settled_at<>'' AND r.used_tokens=?5 AND r.used_requests=1
      AND a.reserved_tokens=0 AND a.reserved_requests=0)",params![job.child.assignment_id,job.child.run_id,job.scope.root_run_id,expected,job.usage["totalTokens"].as_i64()],|r|r.get(0)).map_err(|e|e.to_string())?;
    let (raw,delivered,ack):(String,String,String)=db.query_row("SELECT payload_json,delivered_at,acknowledged_at FROM agent_messages
      WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?3 AND to_run_id=?2 AND from_agent=?4 AND to_agent='coordinator'
      AND kind='human_ordered_assessment_result' AND correlation_id=?5 AND assignment_id=?6 AND evidence_revision=?7 AND dedup_key=?8",
      params![job.result_message_id,job.scope.root_run_id,job.child.run_id,job.child.role.as_str(),job.action.action_id,job.child.assignment_id,job.draft.revision,format!("{}:human_ordered_assessment_result:{}:{}",job.child.assignment_id,job.action.action_id,job.draft.revision)],
      |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"ordered_result_ack_missing")?;
    let payload: Value = serde_json::from_str(&raw).map_err(|_| "ordered_result_ack_unverified")?;
    if !settled
        || job.state != expected
        || delivered.is_empty()
        || ack.is_empty()
        || payload != financial::result_payload(job)
        || uuid::Uuid::parse_str(receipt_id).is_err()
    {
        return Err("ordered_terminal_receipt_unverified".into());
    }
    let previous = if job.action.order == 2 {
        let first = load(db, &job.directive_id, 1)?.ok_or("ordered_predecessor_missing")?;
        let previous = verified(db, &first)?.ok_or("ordered_predecessor_receipt_missing")?;
        if previous["outcome"] != "valid_advisory" {
            return Err("ordered_predecessor_not_valid".into());
        }
        json!({"receiptId":previous["receiptId"],"receiptHash":stable_hash(&previous.to_string()),"actionId":first.action.action_id})
    } else {
        Value::Null
    };
    Ok(
        json!({"schemaVersion":1,"kind":"ordered_readonly_assessment_receipt","receiptId":receipt_id,
      "directiveId":job.directive_id,"sourceDraftId":job.draft.id,"draftRevision":job.draft.revision,"draftHash":job.draft.draft_hash,
      "scanId":job.scope.scan_id,"attemptNumber":job.scope.attempt_number,"rootRunId":job.scope.root_run_id,"targetKey":job.scope.target_key,
      "bindingHash":job.plan.binding_hash,"planHash":job.plan.plan_hash,"actionId":job.action.action_id,"order":job.action.order,"role":job.action.role,
      "assignmentId":job.child.assignment_id,"childRunId":job.child.run_id,"leaseAttemptId":proof["leaseAttemptId"],"workerId":proof["workerId"],
      "rootFinanceId":proof["rootFinanceId"],"rootNativeContractHash":proof["rootNativeContractHash"],
      "requestHash":request_hash,"responseHash":response_hash,"modelEventSequence":sequence,"usage":job.usage,"usageReported":true,
      "resultMessageId":job.result_message_id,"resultPayloadHash":stable_hash(&raw),"predecessor":previous,
      "outcome":if expected=="completed" {"valid_advisory"}else{"invalid_assessment"},"assessment":job.response,
      "advisoryOnly":true,"coverageVerified":false,"targetRequests":0,"independentReviewApproved":false,"reviewerReceipt":Value::Null}),
    )
}
pub(super) fn record(db: &Connection, job: &ActionJob) -> Result<Value, String> {
    if let Some(saved) = verified(db, job)? {
        return Ok(saved);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let receipt = canonical(db, job, &id)?;
    let changed=db.execute("INSERT INTO agent_directive_ordered_receipts(receipt_id,action_id,directive_id,action_order,child_run_id,receipt_json)
      VALUES(?1,?2,?3,?4,?5,?6)",params![id,job.action.action_id,job.directive_id,job.action.order,job.child.run_id,receipt.to_string()]).map_err(|e|e.to_string())?;
    if changed != 1 || verified(db, job)?.as_ref() != Some(&receipt) {
        return Err("ordered_receipt_not_persisted".into());
    }
    Ok(receipt)
}
pub(super) fn verified(db: &Connection, job: &ActionJob) -> Result<Option<Value>, String> {
    let row: Option<(String, String, String, i64, String)> = db
        .query_row(
            "SELECT receipt_id,receipt_json,directive_id,action_order,child_run_id
      FROM agent_directive_ordered_receipts WHERE action_id=?1",
            [&job.action.action_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, raw, directive, order, child)) = row else {
        return Ok(None);
    };
    let receipt: Value = serde_json::from_str(&raw).map_err(|_| "ordered_receipt_unverified")?;
    if directive != job.directive_id
        || order != job.action.order
        || child != job.child.run_id
        || canonical(db, job, &id)? != receipt
    {
        return Err("ordered_receipt_unverified".into());
    }
    event_exact(db, job, "orderedReceiptId", &json!(id))?;
    event_exact(db, job, "orderedState", &json!(job.state))?;
    Ok(Some(receipt))
}
pub(crate) fn project(db: &Connection, id: &str) -> Result<Option<Value>, String> {
    let eligible: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE id=?1
      AND json_extract(payload_json,'$.readonlyAssessmentPlan.schemaVersion')=3)",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !eligible {
        return Ok(None);
    }
    if db.is_autocommit() {
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let value = project(&tx, id)?;
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(value);
    }
    let (draft, scope, state) = source(db, id)?;
    let plan = draft
        .readonly_assessment_plan
        .as_ref()
        .ok_or("ordered_plan_missing")?;
    let mut items = vec![];
    for order in 1..=2 {
        if let Some(job) = load(db, id, order)? {
            let receipt = verified(db, &job)?;
            if matches!(job.state.as_str(), "completed" | "failed") && receipt.is_none() {
                return Err("ordered_receipt_missing".into());
            }
            if order == 2 {
                let first = load(db, id, 1)?.ok_or("ordered_predecessor_missing")?;
                if verified(db, &first)?.is_none_or(|r| r["outcome"] != "valid_advisory") {
                    return Err("ordered_predecessor_not_valid".into());
                }
            }
            let state = if matches!(job.state.as_str(), "prepared" | "executing") && closure::is_parent_closed(db, &job)? {
                closure::verify_cancelled(db, &job)?;
                "cancelled_before_dispatch"
            } else if job.state == "executing" {
                let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls c JOIN agent_assignment_attempts w ON w.assignment_id=c.assignment_id AND w.child_run_id=c.child_run_id
             JOIN agent_coordinator_leases l ON l.root_run_id=c.root_run_id JOIN agent_runs r ON r.id=c.root_run_id
             WHERE c.child_run_id=?1 AND c.assignment_id=?2 AND c.state='executing' AND w.state='running'
             AND datetime(w.expires_at)>datetime('now','localtime') AND l.lease_epoch=?3 AND l.fencing_token=?4
             AND l.lease_expires_at>datetime('now','localtime') AND r.status IN ('prepared','running'))",
             params![job.child.run_id,job.child.assignment_id,job.scope.lease_epoch,job.scope.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
                if active {
                    metadata(db, &job)?;
                    "executing"
                } else {
                    "outcome_unknown"
                }
            } else {
                job.state.as_str()
            };
            items.push(json!({"actionId":job.action.action_id,"order":order,"role":job.action.role,"state":state,
          "assignmentId":job.child.assignment_id,"childRunId":job.child.run_id,"receipt":receipt}));
        } else {
            items.push(json!({"actionId":plan.actions[(order-1)as usize].action_id,"order":order,"role":plan.actions[(order-1)as usize].role,
         "state":"not_started","assignmentId":Value::Null,"childRunId":Value::Null,"receipt":Value::Null}));
        }
    }
    let valid = items
        .iter()
        .filter(|item| item["receipt"]["outcome"] == "valid_advisory")
        .count();
    if (state == "completed" && valid != 2)
        || (state == "failed"
            && !items
                .iter()
                .any(|item| item["receipt"]["outcome"] == "invalid_assessment"))
    {
        return Err("ordered_terminal_parent_unverified".into());
    }
    Ok(Some(
        json!({"schemaVersion":1,"directiveId":id,"sourceDraftId":draft.id,"draftHash":draft.draft_hash,"planHash":plan.plan_hash,
      "scanId":scope.scan_id,"attemptNumber":scope.attempt_number,"rootRunId":scope.root_run_id,"targetKey":scope.target_key,"threadKey":draft.thread_key,
      "state":state,"completedAssessments":valid,"plannedAssessments":2,"advisoryOnly":true,"coverageVerified":false,
      "independentReviewApproved":false,"actions":items}),
    ))
}
pub(crate) fn verified_context(
    db: &Connection,
    scope: &CoordinatorLease,
) -> Result<Vec<Value>, String> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Deferred)
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = {
        let mut q=tx.prepare("SELECT id FROM agent_user_directives WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3
       AND json_extract(payload_json,'$.readonlyAssessmentPlan.schemaVersion')=3 ORDER BY rowid").map_err(|e|e.to_string())?;
        let ids = q
            .query_map(
                params![scope.root_run_id, scope.scan_id, scope.attempt_number],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        ids
    };
    let mut receipts = vec![];
    for id in ids {
        let _ = project(&tx, &id)?;
        for order in 1..=2 {
            if let Some(job) = load(&tx, &id, order)? {
                if let Some(receipt) = verified(&tx, &job)? {
                    receipts.push(receipt)
                }
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(receipts)
}
