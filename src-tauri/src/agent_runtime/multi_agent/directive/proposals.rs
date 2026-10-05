//! Durable, read-only human-requested assessments. Producing a proposal is not
//! executing its suggestions and cannot confirm a finding or widen Web scope.
use super::*;
use crate::agent_runtime::{
    contract::{AgentLane, AgentRole},
    multi_agent::{mailbox, scheduler},
    secrets::redact_json,
};
use serde_json::Value;

pub const PROPOSAL_TOKENS: i64 = 4_000;

mod loading;
pub(crate) mod owned;
mod receipts;
mod scheduling;
pub(crate) use loading::load_job;
pub(super) use loading::load_job_for_terminal_receipt;
pub use loading::ProposalJob;
use loading::{load_job_for, LoadPurpose};
pub(crate) use receipts::{project_receipt, verified_context};
#[cfg(test)]
pub use scheduling::prepare_next;
pub(crate) use scheduling::prepare_next_authorized;

pub(crate) fn result_for_mailbox(
    db: &Connection,
    lease: &CoordinatorLease,
    id: &str,
) -> Result<ProposalJob, String> {
    if db.is_autocommit() {
        return Err("directive_proposal_mailbox_transaction_required".into());
    }
    validate_coordinator_lease(db, lease)?;
    require_executable_coordinator(db, lease)?;
    let job = load_job(db, lease, id)?.ok_or("directive_proposal_binding_invalid")?;
    if !matches!(job.state.as_str(), "received" | "completed" | "failed") {
        return Err("directive_proposal_received_result_required".into());
    }
    validate_saved_response(&job.response, &job.usage)?;
    super::reconciliation::verify_request(db, lease, &job)?;
    super::reconciliation::verify_saved_model_event(db, &job)?;
    Ok(job)
}

/// Only the worker winning this CAS may call the provider. A crash after this
/// commit is deliberately not an automatic retry, even if no HTTP was sent.
#[cfg(test)]
pub fn start(connection: &Connection, lease: &CoordinatorLease, id: &str) -> Result<bool, String> {
    start_authorized(connection, lease, id, |_| Ok(()))
}

pub(crate) fn start_authorized(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
    authorize: impl Fn(&Connection) -> Result<(), String>,
) -> Result<bool, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    validate_coordinator_lease(&transaction, lease)?;
    require_executable_coordinator(&transaction, lease)?;
    authorize(&transaction)?;
    let job = load_job(&transaction, lease, id)?.ok_or("directive_proposal_binding_invalid")?;
    if job.state != "prepared" {
        return Ok(false);
    }
    super::reconciliation::verify_request(&transaction, lease, &job)
        .map_err(|_| "directive_proposal_request_not_consumed")?;
    scheduler::mark_child_running_in_transaction(&transaction, lease, &job.child)?;
    let changed=transaction.execute("UPDATE agent_directive_proposals SET state='executing',updated_at=datetime('now','localtime') WHERE directive_id=?1 AND state='prepared'",[id])
        .map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("directive_proposal_start_not_persisted".into());
    }
    let stored = load_job(&transaction, lease, id)?.ok_or("directive_proposal_binding_invalid")?;
    if stored.state != "executing" || stored.child != job.child || stored.input != job.input {
        return Err("directive_proposal_start_not_persisted".into());
    }
    authorize(&transaction)?;
    validate_coordinator_lease(&transaction, lease)?;
    require_executable_coordinator(&transaction, lease)?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
pub fn record_response(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
    response: &Value,
    usage: &Value,
) -> Result<(), String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    validate_coordinator_lease(&transaction, lease)?;
    require_executable_coordinator(&transaction, lease)?;
    let job = load_job(&transaction, lease, id)?.ok_or("directive_proposal_binding_invalid")?;
    if job.state != "executing" {
        return Err("directive_proposal_response_state_conflict".into());
    }
    validate_saved_response(response, usage)?;
    let event = json!({"turns":1,"modelRequests":usage["modelRequests"],"totalTokens":usage["totalTokens"],
            "directiveId":id,"advisoryOnly":true,"toolCalls":[]});
    let sequence = crate::agent_runtime::store::append_event(
        &transaction,
        &job.child.run_id,
        crate::agent_runtime::contract::AgentEventKind::ModelRoundCompleted,
        &event,
        &[],
    )?;
    let changed=transaction.execute("UPDATE agent_directive_proposals SET state='received',response_json=?1,usage_json=?2,updated_at=datetime('now','localtime') WHERE directive_id=?3 AND state='executing'",
        params![redact_json(response).to_string(),usage.to_string(),id]).map_err(|e|e.to_string())?;
    let persisted: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_directive_proposals WHERE directive_id=?1 AND state='received' AND response_json=?2 AND usage_json=?3) \
         AND EXISTS(SELECT 1 FROM agent_events WHERE run_id=?4 AND sequence=?5 AND event_type='model_round_completed' AND payload_json=?6)",
        params![id,redact_json(response).to_string(),usage.to_string(),job.child.run_id,sequence,event.to_string()],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if changed != 1 || !persisted {
        return Err("directive_proposal_response_not_persisted".into());
    }
    transaction.commit().map_err(|e| e.to_string())
}

pub(super) fn validate_saved_response(response: &Value, usage: &Value) -> Result<(), String> {
    if ![
        "inputTokens",
        "cachedInputTokens",
        "outputTokens",
        "totalTokens",
        "modelRequests",
    ]
    .iter()
    .all(|key| usage[*key].as_i64().is_some_and(|value| value >= 0))
        || usage["modelRequests"] != 1
        || !response["valid"].is_boolean()
        || response["summary"].as_str().is_none()
        || response.to_string().len() > 16_000
    {
        return Err("directive_proposal_response_invalid".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn mark_uncertain(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
) -> Result<(), String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    validate_coordinator_lease(&transaction, lease)?;
    let job = load_job(&transaction, lease, id)?.ok_or("directive_proposal_binding_invalid")?;
    if job.state != "executing" {
        return Err("directive_proposal_response_state_conflict".into());
    }
    let changed=transaction.execute("UPDATE agent_directive_proposals SET state='uncertain',error_code='proposal_model_outcome_unknown',updated_at=datetime('now','localtime') WHERE directive_id=?1 AND state='executing'",[id])
        .map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("directive_proposal_uncertain_not_persisted".into());
    }
    transaction.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
pub fn complete(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
    result_message_id: &str,
) -> Result<(), String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
    complete_in_transaction(&transaction, lease, id, result_message_id)?;
    transaction.commit().map_err(|e| e.to_string())
}

pub(crate) fn complete_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    id: &str,
    result_message_id: &str,
) -> Result<(), String> {
    validate_coordinator_lease(transaction, lease)?;
    require_active_attempt(transaction, &lease.scan_id, lease.attempt_number)?;
    let job = load_job(transaction, lease, id)?.ok_or("directive_proposal_binding_invalid")?;
    let replay = matches!(job.state.as_str(), "completed" | "failed");
    if job.state != "received" && !replay {
        return Err("directive_proposal_result_missing".into());
    }
    let success = job.response["valid"] == true;
    let state = if success { "completed" } else { "failed" };
    let verified: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         JOIN agent_messages m ON m.assignment_id=a.id WHERE a.id=?1 AND a.state=?6 \
         AND r.status='terminal' AND r.terminal_state=?6 AND a.budget_settled_at<>'' AND m.id=?2 \
         AND m.from_run_id=r.id AND m.to_run_id=?3 AND m.kind='human_assessment_result' \
         AND m.correlation_id=?4 AND m.evidence_revision=?5 AND m.acknowledged_at<>'')",
        params![job.child.assignment_id,result_message_id,lease.root_run_id,id,job.revision,state],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if !verified {
        return Err("directive_proposal_result_not_consumed".into());
    }
    let payload: String = transaction
        .query_row(
            "SELECT payload_json FROM agent_messages WHERE id=?1",
            [result_message_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let payload: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    if payload["assessment"] != job.response
        || payload["summary"] != job.response["summary"]
        || payload["directiveId"] != id
        || payload["advisoryOnly"] != true
        || payload["coverageVerified"] != false
        || payload["targetRequests"] != 0
    {
        return Err("directive_proposal_result_payload_conflict".into());
    }
    if replay {
        let matches: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_directive_proposals p JOIN agent_user_directives d ON d.id=p.directive_id \
             WHERE p.directive_id=?1 AND p.result_message_id=?2 AND p.state=?3 AND d.status=?3)",
            params![id,result_message_id,state], |r|r.get(0),
        ).map_err(|e|e.to_string())?;
        return if matches {
            project_receipt(transaction, id)?.ok_or("directive_proposal_receipt_unverified")?;
            Ok(())
        } else {
            Err("directive_proposal_completion_replay_conflict".into())
        };
    }
    transaction.execute("UPDATE agent_directive_proposals SET state=?3,result_message_id=?1,updated_at=datetime('now','localtime') WHERE directive_id=?2",
        params![result_message_id,id,state]).map_err(|e|e.to_string())?;
    if success {
        transition_directive_in_transaction(transaction, lease, id, "assigned", "applied")?;
        transition_directive_in_transaction(transaction, lease, id, "applied", "completed")?;
    } else {
        transition_directive_in_transaction(transaction, lease, id, "assigned", "failed")?;
    }
    let receipt =
        project_receipt(transaction, id)?.ok_or("directive_proposal_receipt_unverified")?;
    if receipt["state"] != state {
        return Err("directive_proposal_receipt_unverified".into());
    }
    Ok(())
}
