//! Local bookkeeping for a saved, read-only assessment after its root ended.
//! No transport, scheduling, lease renewal, execution capability or target I/O.
use super::proposals::{self, ProposalJob};
use crate::agent_runtime::{multi_agent::lease::CoordinatorLease, store::stable_hash};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};

pub fn reconcile_received(
    connection: &Connection,
    scan_id: &str,
    attempt: i64,
    directive_id: &str,
) -> Result<Value, String> {
    reconcile_scoped(connection, scan_id, attempt, directive_id, false)
}

/// Explicit historical bookkeeping. A later scan attempt cannot grant the
/// former child execution rights: only the original frozen owner, response,
/// model event, request and budget may be settled in the same transaction.
pub fn reconcile_historical_received(
    connection: &Connection,
    scan_id: &str,
    attempt: i64,
    directive_id: &str,
) -> Result<Value, String> {
    reconcile_scoped(connection, scan_id, attempt, directive_id, true)
}

fn reconcile_scoped(
    connection: &Connection,
    scan_id: &str,
    attempt: i64,
    directive_id: &str,
    historical: bool,
) -> Result<Value, String> {
    if attempt < 1 {
        return Err("proposal_reconciliation_scope_unavailable".into());
    }
    let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| format!("proposal_reconciliation_lock:{e}"))?;
    // An expired *unchanged* ownership record can prove historical binding. It
    // never becomes an executable lease: don't renew it or call live APIs here.
    let (scope, payload, status, root_code): (CoordinatorLease, String, String, String) = tx.query_row(
        "SELECT l.scan_id,l.attempt_number,l.target_key,l.root_run_id,l.lease_epoch,l.fencing_token,l.lease_expires_at,\
         d.payload_json,d.status,r.terminal_code FROM agent_user_directives d \
         JOIN agent_runs r ON r.id=d.root_run_id AND r.scan_id=d.scan_id AND r.attempt_number=d.attempt_number AND r.target_url=d.target_key \
         JOIN agent_coordinator_leases l ON l.scan_id=d.scan_id AND l.attempt_number=d.attempt_number AND l.target_key=d.target_key \
           AND l.root_run_id=r.id AND d.claim_run_id=r.id AND l.lease_epoch=d.claim_lease_epoch AND l.fencing_token=d.claim_fencing_token \
         JOIN sentinel_scans s ON s.id=d.scan_id AND ((?4=0 AND s.attempt_count=d.attempt_number) \
           OR (?4=1 AND s.attempt_count>d.attempt_number)) \
         WHERE d.id=?1 AND d.scan_id=?2 AND d.attempt_number=?3 AND r.role='coordinator' AND r.root_run_id=r.id AND r.status='terminal' \
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=d.scan_id)",
        params![directive_id,scan_id,attempt,if historical {1} else {0}], |r| Ok((CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,
            lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
        },r.get(7)?,r.get(8)?,r.get(9)?)),
    ).map_err(|_| "proposal_reconciliation_scope_unavailable".to_string())?;
    let payload: Value =
        serde_json::from_str(&payload).map_err(|_| "proposal_reconciliation_payload_invalid")?;
    let closure = &payload["taskClosure"];
    if closure["disposition"] != "receipt_pending"
        || closure["fromStatus"] != "assigned"
        || closure["requiresReconciliation"] != true
        || closure["automaticRetry"] != false
        || closure["rootTerminalCode"] != root_code
        || !closure["closedAt"].as_str().is_some_and(|s| !s.is_empty())
    {
        return Err("proposal_reconciliation_not_saved_receipt".into());
    }
    let job = proposals::load_job_for_terminal_receipt(&tx, &scope, directive_id)?
        .ok_or("proposal_reconciliation_binding_invalid")?;
    proposals::validate_saved_response(&job.response, &job.usage)?;
    verify_request(&tx, &scope, &job)?;
    verify_saved_model_event(&tx, &job)?;
    let expected = if job.response["valid"] == true {
        "completed"
    } else {
        "failed"
    };
    let result_payload = json!({"directiveId":directive_id,"summary":job.response["summary"],
        "assessment":job.response,"advisoryOnly":true,"coverageVerified":false,"targetRequests":0});
    let prior = &payload["localReconciliation"];
    if !prior.is_null() {
        let message = prior["resultMessageId"]
            .as_str()
            .ok_or("proposal_reconciliation_receipt_invalid")?;
        if status != expected
            || job.state != expected
            || *prior
                != receipt(
                    &job,
                    closure,
                    message,
                    prior["completedAt"].as_str().unwrap_or(""),
                )
            || !prior["completedAt"].as_str().is_some_and(|s| !s.is_empty())
        {
            return Err("proposal_reconciliation_replay_conflict".into());
        }
        verify_result(&tx, &scope, &job, message, &result_payload, true)?;
        verify_settlement(&tx, &scope, &job, expected, message)?;
        proposals::project_receipt(&tx, directive_id)?
            .ok_or("directive_proposal_receipt_unverified")?;
        return Ok(prior.clone());
    }
    if status != "deferred" || job.state != "received" {
        return Err("proposal_reconciliation_state_conflict".into());
    }
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_user_directives d JOIN agent_directive_proposals p ON p.directive_id=d.id \
         WHERE d.id=?1 AND d.rejection_code='directive_task_ended_receipt_pending' AND p.result_message_id='')",
        [directive_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !pending {
        return Err("proposal_reconciliation_pending_conflict".into());
    }
    settle_paused(&tx, &scope, &job, expected)?;
    let message = uuid::Uuid::new_v4().to_string();
    let dedup = result_dedup(&job);
    tx.execute(
        "INSERT INTO agent_messages(id,run_id,root_run_id,from_run_id,to_run_id,from_agent,to_agent,kind,correlation_id,\
         dedup_key,assignment_id,evidence_revision,payload_json) VALUES(?1,?2,?2,?3,?2,?4,'coordinator','human_assessment_result',?5,?6,?7,?8,?9) \
         ON CONFLICT(run_id,dedup_key) DO NOTHING",
        params![message,scope.root_run_id,job.child.run_id,job.child.role.as_str(),directive_id,dedup,
            job.child.assignment_id,job.revision,result_payload.to_string()],
    ).map_err(|e| format!("proposal_reconciliation_mailbox:{e}"))?;
    let message: String = tx
        .query_row(
            "SELECT id FROM agent_messages WHERE run_id=?1 AND dedup_key=?2",
            params![scope.root_run_id, dedup],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    verify_result(&tx, &scope, &job, &message, &result_payload, false)?;
    tx.execute(
        "UPDATE agent_messages SET delivered_at=CASE WHEN delivered_at='' THEN datetime('now','localtime') ELSE delivered_at END,\
         acknowledged_at=datetime('now','localtime'),delivery_attempts=delivery_attempts+1 WHERE id=?1 AND acknowledged_at=''",
        [&message],
    ).map_err(|e| format!("proposal_reconciliation_ack:{e}"))?;
    // A successful SQL call can still affect zero rows (e.g. a legacy trigger
    // using RAISE(IGNORE)). Completion requires a persisted, valid receipt.
    verify_result(&tx, &scope, &job, &message, &result_payload, true)?;
    one(tx.execute("UPDATE agent_directive_proposals SET state=?1,result_message_id=?2,updated_at=datetime('now','localtime') WHERE directive_id=?3 AND state='received'",
        params![expected,message,directive_id]), "proposal")?;
    let completed_at: String = tx
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let receipt = receipt(&job, closure, &message, &completed_at);
    one(tx.execute(
        "UPDATE agent_user_directives SET status=?1,rejection_code='',\
         payload_json=json_set(payload_json,'$.localReconciliation',json(?2)),updated_at=datetime('now','localtime') \
         WHERE id=?3 AND status='deferred' AND json_extract(payload_json,'$.localReconciliation') IS NULL",
        params![expected,receipt.to_string(),directive_id]), "directive")?;
    verify_settlement(&tx, &scope, &job, expected, &message)?;
    proposals::project_receipt(&tx, directive_id)?
        .ok_or("directive_proposal_receipt_unverified")?;
    tx.commit()
        .map_err(|e| format!("proposal_reconciliation_commit:{e}"))?;
    Ok(receipt)
}

pub(super) fn verify_saved_model_event(
    connection: &Connection,
    job: &ProposalJob,
) -> Result<(), String> {
    if proposals::owned::verify_saved(connection, job)? {
        return Ok(());
    }
    let (count, raw): (i64, Option<String>) = connection.query_row(
        "SELECT COUNT(*),MAX(payload_json) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",
        [&job.child.run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|e| format!("proposal_reconciliation_model_event:{e}"))?;
    let event = raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    if count != 1
        || event.as_ref()
            != Some(&json!({
                "turns":1,"modelRequests":job.usage["modelRequests"],"totalTokens":job.usage["totalTokens"],
                "directiveId":job.directive_id,"advisoryOnly":true,"toolCalls":[]
            }))
    {
        return Err("proposal_reconciliation_model_event_unverified".into());
    }
    Ok(())
}

pub(super) fn receipt(job: &ProposalJob, closure: &Value, message: &str, at: &str) -> Value {
    json!({"schemaVersion":1,"kind":"saved_assessment_receipt","status":if job.response["valid"]==true {"completed"} else {"failed"},
        "resultMessageId":message,"assignmentId":job.child.assignment_id,"childRunId":job.child.run_id,
        "responseHash":stable_hash(&job.response.to_string()),"usageHash":stable_hash(&job.usage.to_string()),
        "taskClosureHash":stable_hash(&closure.to_string()),"completedAt":at,
        "modelRequests":0,"targetRequests":0,"advisoryOnly":true,"coverageVerified":false})
}

fn one(result: rusqlite::Result<usize>, part: &str) -> Result<(), String> {
    match result {
        Ok(1) => Ok(()),
        Ok(_) => Err(format!("proposal_reconciliation_{part}_conflict")),
        Err(e) => Err(format!("proposal_reconciliation_{part}:{e}")),
    }
}

pub(super) fn verify_request(
    tx: &Connection,
    scope: &CoordinatorLease,
    job: &ProposalJob,
) -> Result<(), String> {
    let input: Option<String> = tx.query_row(
        "SELECT payload_json FROM agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?2 \
         AND to_run_id=?3 AND from_agent='coordinator' AND to_agent=?4 AND kind='human_assessment_request' \
         AND correlation_id=?5 AND assignment_id=?6 AND evidence_revision=?7 AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts>0",
        params![job.request_message_id,scope.root_run_id,job.child.run_id,job.child.role.as_str(),job.directive_id,job.child.assignment_id,job.revision],
        |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    if input
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .as_ref()
        != Some(&job.input)
    {
        return Err("proposal_reconciliation_request_conflict".into());
    }
    Ok(())
}

fn result_dedup(job: &ProposalJob) -> String {
    format!(
        "{}:human_assessment_result:{}:{}",
        job.child.assignment_id, job.directive_id, job.revision
    )
}

pub(super) fn verify_result(
    tx: &Connection,
    scope: &CoordinatorLease,
    job: &ProposalJob,
    id: &str,
    payload: &Value,
    ack: bool,
) -> Result<(), String> {
    let stored: Option<String> = tx.query_row(
        "SELECT payload_json FROM agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND to_run_id=?2 \
         AND from_run_id=?3 AND to_agent='coordinator' AND from_agent=?4 AND kind='human_assessment_result' \
         AND correlation_id=?5 AND assignment_id=?6 AND evidence_revision=?7 AND dedup_key=?8 \
         AND (?9=0 OR (acknowledged_at<>'' AND delivered_at<>'' AND delivery_attempts>0))",
        params![id,scope.root_run_id,job.child.run_id,job.child.role.as_str(),job.directive_id,job.child.assignment_id,job.revision,result_dedup(job),ack],
        |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    if stored
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .as_ref()
        != Some(payload)
    {
        return Err("proposal_reconciliation_result_conflict".into());
    }
    Ok(())
}

fn settle_paused(
    tx: &Transaction<'_>,
    scope: &CoordinatorLease,
    job: &ProposalJob,
    state: &str,
) -> Result<(), String> {
    let (tokens, requests): (i64,i64) = tx.query_row(
        "SELECT a.reserved_tokens,a.reserved_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.id=?1 AND r.id=?2 AND a.state='paused' AND r.status='paused' AND a.budget_settled_at=''",
        params![job.child.assignment_id,job.child.run_id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_| "proposal_reconciliation_child_not_paused")?;
    let spent = job.usage["totalTokens"]
        .as_i64()
        .ok_or("proposal_reconciliation_usage_invalid")?;
    let cached = job.usage["cachedInputTokens"]
        .as_i64()
        .ok_or("proposal_reconciliation_usage_invalid")?;
    let used_requests = job.usage["modelRequests"]
        .as_i64()
        .ok_or("proposal_reconciliation_usage_invalid")?;
    if spent > tokens || used_requests > requests || cached > spent {
        return Err("proposal_reconciliation_usage_exceeds_reservation".into());
    }
    let usage = crate::agent_runtime::store::UsageDelta {
        input_tokens: job.usage["inputTokens"]
            .as_i64()
            .ok_or("proposal_reconciliation_usage_invalid")?,
        cached_input_tokens: cached,
        output_tokens: job.usage["outputTokens"]
            .as_i64()
            .ok_or("proposal_reconciliation_usage_invalid")?,
        total_tokens: spent,
        model_requests: used_requests,
    };
    super::super::budget::model::settle_terminal_receipt(
        tx,
        scope,
        &job.child.assignment_id,
        &usage,
    )?;
    super::super::budget::limits::release_terminal_slot(tx, scope, &job.child.assignment_id)?;
    one(tx.execute(
        "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens-?1,reserved_requests=reserved_requests-?2,\
         spent_tokens=spent_tokens+?3,spent_requests=spent_requests+?4,updated_at=datetime('now','localtime') \
         WHERE root_run_id=?5 AND lease_epoch=?6 AND fencing_token=?7 AND reserved_tokens>=?1 AND reserved_requests>=?2 \
         AND (total_tokens=0 OR spent_tokens+reserved_tokens-?1+?3<=total_tokens) \
         AND (total_requests=0 OR spent_requests+reserved_requests-?2+?4<=total_requests)",
        params![tokens,requests,spent,used_requests,scope.root_run_id,scope.lease_epoch,scope.fencing_token]), "budget")?;
    one(tx.execute(
        "UPDATE agent_runs SET status='terminal',terminal_state=?1,terminal_code='proposal_receipt_reconciled',\
         terminal_reason='Saved read-only assessment reconciled locally; no suggestions executed',\
         used_tokens=?2,used_cached_tokens=?3,used_requests=?4,finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
         WHERE id=?5 AND root_run_id=?6 AND status='paused'",
        params![state,spent,cached,used_requests,job.child.run_id,scope.root_run_id]), "child")?;
    one(tx.execute(
        "UPDATE agent_assignments SET state=?1,failure_class=CASE WHEN ?1='failed' THEN 'proposal_invalid_assessment' ELSE '' END,\
         reserved_tokens=0,reserved_requests=0,budget_settled_at=datetime('now','localtime'),finished_at=datetime('now','localtime'),\
         updated_at=datetime('now','localtime') WHERE id=?2 AND state='paused' AND budget_settled_at=''",
        params![state,job.child.assignment_id]), "assignment")?;
    tx.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1 AND revoked_at=''",
        [&job.child.run_id]).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM agent_lane_leases WHERE assignment_id=?1",
        [&job.child.assignment_id],
    )
    .map_err(|e| e.to_string())?;
    super::super::attempts::finish(tx, scope, &job.child, state)?;
    Ok(())
}

fn verify_settlement(
    tx: &Transaction<'_>,
    scope: &CoordinatorLease,
    job: &ProposalJob,
    state: &str,
    message: &str,
) -> Result<(), String> {
    let verified: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         JOIN agent_directive_proposals p ON p.assignment_id=a.id JOIN agent_budget_ledger b ON b.root_run_id=a.coordinator_run_id \
         WHERE a.id=?1 AND r.id=?2 AND a.state=?3 AND r.status='terminal' AND r.terminal_state=?3 \
         AND r.terminal_code='proposal_receipt_reconciled' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
         AND r.used_tokens=?4 AND r.used_cached_tokens=?5 AND r.used_requests=?6 \
         AND p.directive_id=?7 AND p.state=?3 AND p.result_message_id=?8 \
         AND b.root_run_id=?9 AND b.lease_epoch=?10 AND b.fencing_token=?11 AND b.spent_tokens>=?4 AND b.spent_requests>=?6) \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![job.child.assignment_id,job.child.run_id,state,job.usage["totalTokens"].as_i64(),job.usage["cachedInputTokens"].as_i64(),
            job.usage["modelRequests"].as_i64(),job.directive_id,message,scope.root_run_id,scope.lease_epoch,scope.fencing_token], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !verified {
        return Err("proposal_reconciliation_settlement_conflict".into());
    }
    Ok(())
}
