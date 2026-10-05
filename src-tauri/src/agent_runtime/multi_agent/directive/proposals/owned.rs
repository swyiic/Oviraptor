//! New human proposal dispatch proof lives in the immutable original specialist
//! request. No parallel SDK, fabricated worker or rewrite of old receipts.
use super::*;
use crate::agent_runtime::{
    multi_agent::{attempts, budget, specialist},
    store::stable_hash,
};

pub(crate) fn metadata(
    db: &Connection,
    scope: &CoordinatorLease,
    job: &ProposalJob,
) -> Result<Value, String> {
    if db.is_autocommit() {
        return Err("proposal_owned_transaction_required".into());
    }
    let draft = load_draft(
        db,
        &db.query_row(
            "SELECT source_draft_id FROM agent_user_directives WHERE id=?1",
            [&job.directive_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?,
    )?
    .ok_or("directive_draft_missing")?;
    if !draft_integrity_valid(&draft)
        || draft.status != "confirmed"
        || draft.confirmed_directive_id != job.directive_id
        || draft.scan_id != scope.scan_id
        || draft.attempt_number != scope.attempt_number
        || draft.revision != job.revision
        || draft
            .requested_roles
            .iter()
            .filter(|r| r.as_str() != "coordinator")
            .map(String::as_str)
            .collect::<Vec<_>>()
            != vec![job.child.role.as_str()]
        || draft.estimated_tokens != PROPOSAL_TOKENS
        || draft.estimated_requests != 1
        || draft.side_effect_class != "read_only"
        || !draft.requested_contracts.is_empty()
        || draft.proposed_scope_change.is_some()
    {
        return Err("proposal_owned_source_invalid".into());
    }
    let owner = budget::root::RootOwner::load_original(db, &scope.root_run_id)?;
    owner.require_original_coordinator(db, scope)?;
    let (root_finance_id, root_contract): (String, String) = db
        .query_row(
            "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&scope.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let worker = attempts::current(db, scope, &job.child.assignment_id)?;
    let original =
        budget::receipts::original_owner(db, &job.child.run_id, scope, &job.child.assignment_id)?;
    original.verify(db)?;
    if worker.child_run_id != job.child.run_id || original.attempt_id() != worker.id {
        return Err("proposal_owned_worker_invalid".into());
    }
    super::super::reconciliation::verify_request(db, scope, job)?;
    Ok(
        json!({"schemaVersion":1,"kind":"human_readonly_assessment_dispatch",
        "directiveId":job.directive_id,"sourceDraftId":draft.id,"confirmedRevision":draft.revision,"confirmedHash":draft.draft_hash,
        "scanId":scope.scan_id,"attempt":scope.attempt_number,"targetKey":scope.target_key,"rootRunId":scope.root_run_id,
        "rootFinanceId":root_finance_id,"rootNativeContractHash":stable_hash(&root_contract),
        "coordinatorEpoch":scope.lease_epoch,"coordinatorFence":scope.fencing_token,
        "assignmentId":job.child.assignment_id,"childRunId":job.child.run_id,"role":job.child.role.as_str(),
        "leaseAttemptId":worker.id,"workerId":worker.worker_id,"workerEpoch":worker.lease_epoch,"workerFence":worker.fencing_token,
        "requestMessageId":job.request_message_id,"evidenceRevision":job.revision,"inputHash":stable_hash(&job.input.to_string()),
        "modelRequests":1,"modelTokenCeiling":PROPOSAL_TOKENS,"maxOutputTokens":512,"targetRequests":0,"advisoryOnly":true}),
    )
}

pub(crate) fn authorize(
    db: &Connection,
    scope: &CoordinatorLease,
    job: &ProposalJob,
    proof: &Value,
) -> Result<(), String> {
    validate_coordinator_lease(db, scope)?;
    require_executable_coordinator(db, scope)?;
    let current =
        load_job(db, scope, &job.directive_id)?.ok_or("directive_proposal_binding_invalid")?;
    if current.state != "executing"
        || current.child != job.child
        || current.input != job.input
        || current.revision != job.revision
        || metadata(db, scope, &current)? != *proof
    {
        return Err("proposal_owned_dispatch_scope_changed".into());
    }
    attempts::require_live_for_run(db, &job.child.run_id)?;
    // Authorizer is called both before and after the original dispatch INSERT.
    // Once present, its complete canonical request must bind the same proof.
    if let Some((raw, hash)) = db
        .query_row(
            "SELECT request_json,request_hash FROM agent_specialist_calls WHERE child_run_id=?1",
            [&job.child.run_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| "proposal_owned_dispatch_invalid")?;
        if stable_hash(&raw) != hash || value["humanDirectiveDispatch"] != *proof {
            return Err("proposal_owned_dispatch_invalid".into());
        }
    }
    Ok(())
}

pub(crate) fn response(
    db: &Connection,
    scope: &CoordinatorLease,
    job: &ProposalJob,
) -> Result<Option<specialist::StoredResponse>, String> {
    if db.is_autocommit() {
        return Err("proposal_owned_transaction_required".into());
    }
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?1)",
            [&job.child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Ok(None);
    }
    let proof = metadata(db, scope, job)?;
    let saved = specialist::received_for_owned_proposal(db, scope, &job.child, &proof)?;
    if !matches!(
        saved.rejection.as_str(),
        "" | "empty_response" | "unexpected_tool_calls" | "response_too_large"
    ) {
        return Err("proposal_original_response_withheld".into());
    }
    Ok(Some(saved))
}

pub(crate) fn verify_saved(db: &Connection, job: &ProposalJob) -> Result<bool, String> {
    let scope = original_scope(db, &job.directive_id)?;
    let Some(saved) = response(db, &scope, job)? else {
        return Ok(false);
    };
    if saved.usage.as_json() != job.usage || project_response(&saved) != job.response {
        return Err("proposal_original_response_projection_changed".into());
    }
    let count:i64=db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&job.child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("proposal_original_model_event_ambiguous".into());
    }
    Ok(true)
}

pub(crate) fn record_received(
    connection: &Connection,
    scope: &CoordinatorLease,
    id: &str,
) -> Result<ProposalJob, String> {
    let private = write_guard::private_connection(connection)?;
    let tx = rusqlite::Transaction::new_unchecked(&private, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    validate_coordinator_lease(&tx, scope)?;
    require_executable_coordinator(&tx, scope)?;
    let job = load_job(&tx, scope, id)?.ok_or("directive_proposal_binding_invalid")?;
    if !matches!(job.state.as_str(), "executing" | "received") {
        return Err("proposal_owned_response_state_conflict".into());
    }
    let saved = response(&tx, scope, &job)?.ok_or("proposal_original_dispatch_required")?;
    let response = project_response(&saved);
    let usage = saved.usage.as_json();
    validate_saved_response(&response, &usage)?;
    let guard = write_guard::Writer::install(&private, id)?;
    if job.state == "executing" {
        let changed=tx.execute("UPDATE agent_directive_proposals SET state='received',response_json=?1,usage_json=?2,updated_at=datetime('now','localtime') WHERE directive_id=?3 AND state='executing'",params![response.to_string(),usage.to_string(),id]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("proposal_owned_response_not_persisted".into());
        }
    } else if job.response != response || job.usage != usage {
        return Err("proposal_owned_response_replay_conflict".into());
    }
    let stored = load_job(&tx, scope, id)?.ok_or("directive_proposal_binding_invalid")?;
    if stored.state != "received"
        || stored.response != response
        || stored.usage != usage
        || !verify_saved(&tx, &stored)?
    {
        return Err("proposal_owned_response_not_persisted".into());
    }
    guard.verify()?;
    validate_coordinator_lease(&tx, scope)?;
    require_executable_coordinator(&tx, scope)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(stored)
}

// Local original receipt recovery only. An executing/uncertain provider call
// has no safe reply to recover and cannot become permission for a second HTTP.
pub(crate) fn recover_received(db: &Connection, scope: &CoordinatorLease) -> Result<(), String> {
    let ids = {
        let mut q=db.prepare("SELECT p.directive_id FROM agent_directive_proposals p JOIN agent_user_directives d ON d.id=p.directive_id
        JOIN agent_specialist_calls c ON c.child_run_id=p.child_run_id AND c.assignment_id=p.assignment_id
        WHERE p.state='executing' AND c.state='received' AND d.status='assigned' AND d.root_run_id=?1
        AND d.scan_id=?2 AND d.attempt_number=?3 AND d.target_key=?4 ORDER BY p.rowid").map_err(|e|e.to_string())?;
        let ids = q
            .query_map(
                params![
                    scope.root_run_id,
                    scope.scan_id,
                    scope.attempt_number,
                    scope.target_key
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        ids
    };
    for id in ids {
        record_received(db, scope, &id)?;
    }
    Ok(())
}

fn original_scope(db: &Connection, id: &str) -> Result<CoordinatorLease, String> {
    db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,claim_lease_epoch,claim_fencing_token FROM agent_user_directives WHERE id=?1",[id],|r|Ok(CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:String::new()})).map_err(|e|e.to_string())
}

fn project_response(saved: &specialist::StoredResponse) -> Value {
    if saved.rejection.is_empty() {
        assessment(&saved.text)
    } else {
        assessment("")
    }
}

// This is the original bounded, redacted three-field projection. The original
// financial/model response remains byte-exact in its existing receipt contract.
pub(crate) fn assessment(text: &str) -> Value {
    let parsed = serde_json::from_str::<Value>(text).ok();
    let valid = text.len() <= 12_000
        && parsed.as_ref().is_some_and(|value| {
            value.as_object().is_some_and(|v| v.len() == 3)
                && value["summary"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s.len() <= 4_000)
                && ["suggestions", "limitations"].iter().all(|key| {
                    value[*key].as_array().is_some_and(|v| {
                        v.len() <= 12
                            && v.iter()
                                .all(|s| s.as_str().is_some_and(|s| s.len() <= 1_000))
                    })
                })
        });
    let mut response = if valid {
        parsed.unwrap()
    } else {
        json!({"summary":"模型未返回有效的只读评估提案；未执行任何建议。","suggestions":[],"limitations":["proposal_model_response_invalid"]})
    };
    response["valid"] = valid.into();
    redact_json(&response)
}

mod dispatch_guard;
mod write_guard;
pub(crate) use dispatch_guard::DispatchGuard;

mod delivery_guard;
pub(crate) use delivery_guard::DeliveryGuard;
