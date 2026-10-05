//! Read-only delivery proof. Stored ownership proves history, not permission
//! to execute. All callers select and verify in a single database transaction.
use super::*;

const INVALID: &str = "directive_proposal_receipt_unverified";

pub(crate) fn project_receipt(connection: &Connection, id: &str) -> Result<Option<Value>, String> {
    project(connection, id).map_err(|_| INVALID.to_string())
}

fn project(connection: &Connection, id: &str) -> Result<Option<Value>, String> {
    let record = connection.query_row(
        "SELECT d.scan_id,d.attempt_number,d.target_key,d.root_run_id,d.claim_lease_epoch,d.claim_fencing_token,\
         d.source_draft_id,d.status,d.payload_json,d.recipient_role,\
         p.state,p.result_message_id,p.error_code,p.response_json,p.assignment_id,p.child_run_id \
         FROM agent_user_directives d LEFT JOIN agent_directive_proposals p ON p.directive_id=d.id WHERE d.id=?1",
        [id], |r| Ok((CoordinatorLease { scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,
            root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:String::new() },
            r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?,
            r.get::<_,Option<String>>(10)?,r.get::<_,Option<String>>(11)?,r.get::<_,Option<String>>(12)?,
            r.get::<_,Option<String>>(13)?,r.get::<_,Option<String>>(14)?,r.get::<_,Option<String>>(15)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((
        scope,
        source,
        status,
        raw_payload,
        role,
        state,
        result,
        error,
        response,
        assignment,
        child,
    )) = record
    else {
        return Ok(None);
    };
    let payload: Value = serde_json::from_str(&raw_payload).map_err(|e| e.to_string())?;
    let draft = load_draft(connection, &source)?;
    // Ordered records have a separate original physical receipt contract.
    // Validate it before declining singleton projection; never hide a damaged
    // singleton by accepting only a version field from its payload.
    if payload["readonlyAssessmentPlan"]["schemaVersion"] == 3
        || draft.as_ref().and_then(|d| d.readonly_assessment_plan.as_ref())
            .is_some_and(|plan| plan.schema_version == 3)
    {
        if state.is_some() || super::super::ordered_execution::project(connection, id)?.is_none() {
            return Err(INVALID.into());
        }
        return Ok(None);
    }
    let is_proposal = state.is_some()
        || payload["intent"] == "agent_proposal_request"
        || draft
            .as_ref()
            .is_some_and(|d| d.intent == "agent_proposal_request");
    if !is_proposal {
        return Ok(None);
    }
    let Some(state) = state else {
        return if matches!(status.as_str(), "completed" | "failed") {
            Err(INVALID.into())
        } else {
            Ok(None)
        };
    };
    let response: Value =
        serde_json::from_str(response.as_deref().unwrap_or("null")).map_err(|e| e.to_string())?;
    let projected = json!({"state":state,"assignmentId":assignment,"childRunId":child,
        "summary":response["summary"],"errorCode":error,"advisoryOnly":true,"coverageVerified":false});
    // A cancelled, never-dispatched job is not a delivered model result. Keep
    // its failure visible without manufacturing a response or usage receipt.
    let delivered = state == "completed"
        || status == "completed"
        || (state == "failed"
            && (response != json!({}) || result.as_deref().is_some_and(|s| !s.is_empty())));
    if !delivered {
        return Ok(Some(projected));
    }
    let draft = draft.ok_or(INVALID)?;
    let job = load_job_for(connection, &scope, id, LoadPurpose::History)?.ok_or(INVALID)?;
    validate_saved_response(&job.response, &job.usage)?;
    let expected = if job.response["valid"] == true {
        "completed"
    } else {
        "failed"
    };
    let confirmed = confirmed_payload(&draft);
    let roles: Vec<_> = draft
        .requested_roles
        .iter()
        .filter(|r| r.as_str() != "coordinator")
        .map(String::as_str)
        .collect();
    if state != expected
        || status != expected
        || role != draft.recipient_role
        || scope.lease_epoch <= 0
        || scope.fencing_token.is_empty()
        || confirmed
            .as_object()
            .ok_or(INVALID)?
            .iter()
            .any(|(k, v)| payload.get(k) != Some(v))
        || roles != vec![job.child.role.as_str()]
        || draft.side_effect_class != "read_only"
        || !draft.requested_contracts.is_empty()
        || draft.proposed_scope_change.is_some()
        || draft.estimated_tokens != PROPOSAL_TOKENS
        || draft.estimated_requests != 1
        || job.revision != draft.revision
        || job.input["kind"] != "human_requested_assessment"
        || job.input["directiveId"] != id
        || job.input["target"] != scope.target_key
        || job.input["request"] != draft.safe_execution_text
        || job.input["referencedFactIds"] != json!(draft.referenced_fact_ids)
        || job.input["constraints"]
            != json!({"webOnly":true,"targetRequests":0,"tools":[],"advisoryOnly":true})
    {
        return Err(INVALID.into());
    }
    let message = result.as_deref().filter(|s| !s.is_empty()).ok_or(INVALID)?;
    super::super::reconciliation::verify_request(connection, &scope, &job)?;
    let expected_payload = json!({"directiveId":id,"summary":job.response["summary"],"assessment":job.response,
        "advisoryOnly":true,"coverageVerified":false,"targetRequests":0});
    super::super::reconciliation::verify_result(
        connection,
        &scope,
        &job,
        message,
        &expected_payload,
        true,
    )?;
    let settled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         JOIN agent_runs root ON root.id=a.coordinator_run_id JOIN agent_budget_ledger b ON b.root_run_id=root.id \
         WHERE a.id=?1 AND r.id=?2 AND a.state=?3 AND r.status='terminal' AND r.terminal_state=?3 \
         AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
         AND r.used_tokens=?4 AND r.used_cached_tokens=?5 AND r.used_requests=?6 \
         AND root.id=?7 AND root.role='coordinator' \
         AND (root.root_run_id=root.id OR (root.root_run_id='' AND root.orchestration_policy='single' AND root.assignment_id='' AND root.parent_run_id IS NULL)) \
         AND root.scan_id=?8 AND root.attempt_number=?9 AND root.target_url=?10 \
         AND r.parent_run_id=root.id AND a.trigger_code=?11 \
         AND b.spent_tokens>=?4 AND b.spent_requests>=?6) \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![job.child.assignment_id,job.child.run_id,expected,job.usage["totalTokens"].as_i64(),
            job.usage["cachedInputTokens"].as_i64(),job.usage["modelRequests"].as_i64(),scope.root_run_id,
            scope.scan_id,scope.attempt_number,scope.target_key,format!("human_directive:{id}")], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    super::super::reconciliation::verify_saved_model_event(connection, &job)?;
    let middle = if !payload["localReconciliation"].is_null() {
        "deferred"
    } else if expected == "completed" {
        "applied"
    } else {
        "assigned"
    };
    let transitions: [Option<i64>;3] = connection.query_row(
        "SELECT MAX(CASE WHEN json_extract(payload_json,'$.status')='assigned' THEN sequence END), \
         MAX(CASE WHEN json_extract(payload_json,'$.status')=?6 THEN sequence END), \
         MAX(CASE WHEN json_extract(payload_json,'$.status')=?5 THEN sequence END) \
         FROM agent_collaboration_events WHERE entity_id=?1 AND entity_type='user_directive' \
         AND event_type='user_directive' AND scan_id=?2 AND attempt_number=?3 \
         AND json_extract(payload_json,'$.sourceDraftId')=?4",
        params![id,scope.scan_id,scope.attempt_number,draft.id,expected,middle], |r|Ok([r.get(0)?,r.get(1)?,r.get(2)?]),
    ).map_err(|e|e.to_string())?;
    if !settled
        || !matches!(transitions,[Some(assigned),Some(middle),Some(finished)] if assigned<=middle && middle<finished)
    {
        return Err(INVALID.into());
    }
    if !payload["localReconciliation"].is_null() {
        let prior = &payload["localReconciliation"];
        let at = prior["completedAt"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(INVALID)?;
        if *prior
            != super::super::reconciliation::receipt(&job, &payload["taskClosure"], message, at)
        {
            return Err(INVALID.into());
        }
    }
    Ok(Some(projected))
}

/// Validate *all* completed records before selecting the five newest pieces
/// of advice. A deleted proposal cannot disappear through an inner join.
pub(crate) fn verified_context(
    connection: &Connection,
    scope: &CoordinatorLease,
) -> Result<Vec<(String, Value)>, String> {
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Deferred)
        .map_err(|e| e.to_string())?;
    let ids = {
        let mut query = tx.prepare("SELECT d.id FROM agent_user_directives d \
            WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.target_key=?3 AND d.root_run_id=?4 \
            AND (d.status IN ('completed','failed') OR EXISTS(SELECT 1 FROM agent_directive_proposals p \
              WHERE p.directive_id=d.id AND p.state IN ('completed','failed'))) ORDER BY d.rowid DESC").map_err(|e|e.to_string())?;
        let rows = query
            .query_map(
                params![
                    scope.scan_id,
                    scope.attempt_number,
                    scope.target_key,
                    scope.root_run_id
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let mut result = Vec::new();
    for id in ids {
        if let Some(receipt) = project_receipt(&tx, &id)? {
            if receipt["state"] == "completed" && result.len() < 5 {
                let raw: String = tx
                    .query_row(
                        "SELECT response_json FROM agent_directive_proposals WHERE directive_id=?1",
                        [&id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                result.push((id, serde_json::from_str(&raw).map_err(|e| e.to_string())?));
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
