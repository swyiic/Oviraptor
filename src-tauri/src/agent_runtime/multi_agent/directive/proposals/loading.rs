use super::*;

#[derive(Debug)]
pub struct ProposalJob {
    pub directive_id: String,
    pub child: scheduler::ScheduledChild,
    pub input: Value,
    pub revision: i64,
    pub request_message_id: String,
    pub state: String,
    pub response: Value,
    pub usage: Value,
}

pub(crate) fn load_job(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
) -> Result<Option<ProposalJob>, String> {
    load_job_for(connection, lease, id, LoadPurpose::Execution)
}

pub(super) enum LoadPurpose {
    Execution,
    TerminalReceipt,
    History,
}

// Only the sibling local-receipt reconciler may load a deferred proposal. This
// does not relax start/record_response/mailbox/lease execution guards.
pub(in super::super) fn load_job_for_terminal_receipt(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
) -> Result<Option<ProposalJob>, String> {
    load_job_for(connection, lease, id, LoadPurpose::TerminalReceipt)
}

pub(super) fn load_job_for(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
    purpose: LoadPurpose,
) -> Result<Option<ProposalJob>, String> {
    // Assigned jobs are not in prepare_model_context's accepted inbox. Recheck
    // their confirmed source on recovery too; assignment existence is not
    // permission to continue a tampered, rejected or rebound instruction.
    let source: Option<(String,i64,String,String,String,String)> = connection.query_row(
        "SELECT source_draft_id,confirmed_revision,confirmed_hash,thread_key,text_redacted,status \
         FROM agent_user_directives WHERE id=?1",[id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((source, revision, hash, thread, text, status)) = source else {
        return Ok(None);
    };
    let draft = load_draft(connection, &source)?.ok_or("directive_draft_missing")?;
    let status_valid = match purpose {
        LoadPurpose::Execution => matches!(status.as_str(), "assigned" | "completed" | "failed"),
        LoadPurpose::TerminalReceipt => {
            matches!(status.as_str(), "deferred" | "completed" | "failed")
        }
        LoadPurpose::History => matches!(status.as_str(), "completed" | "failed"),
    };
    if !status_valid
        || !draft_integrity_valid(&draft)
        || draft.status != "confirmed"
        || draft.confirmed_directive_id != id
        || draft.scan_id != lease.scan_id
        || draft.attempt_number != lease.attempt_number
        || draft.revision != revision
        || draft.draft_hash != hash
        || draft.thread_key != thread
        || draft.safe_execution_text != text
        || draft.intent != "agent_proposal_request"
        || draft.validation_result == "rejected"
        || !((draft.root_run_id.is_empty()
            && draft.target_key.is_empty()
            && draft.bound_lease_epoch == 0
            && draft.bound_fencing_token.is_empty())
            || (draft.root_run_id == lease.root_run_id
                && draft.target_key == lease.target_key
                && draft.bound_lease_epoch == lease.lease_epoch
                && draft.bound_fencing_token == lease.fencing_token))
        || (!matches!(purpose, LoadPurpose::History)
            && (validate_thread_key(connection, &lease.root_run_id, &lease.target_key, &thread)
                .is_err()
                || !fact_refs_current(connection, &lease.root_run_id, &draft.referenced_fact_ids)?))
    {
        return Err("directive_proposal_source_invalid".into());
    }
    let (payload, recipient): (String, String) = connection
        .query_row(
            "SELECT payload_json,recipient_role FROM agent_user_directives WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let payload: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    if recipient != draft.recipient_role
        || confirmed_payload(&draft)
            .as_object()
            .ok_or("directive_proposal_source_invalid")?
            .iter()
            .any(|(k, v)| payload.get(k) != Some(v))
    {
        return Err("directive_proposal_source_invalid".into());
    }
    let row = connection.query_row(
        "SELECT p.assignment_id,p.child_run_id,a.role,a.task_slice_json,a.evidence_revision,\
         p.request_message_id,p.state,p.response_json,p.usage_json FROM agent_directive_proposals p \
         JOIN agent_user_directives d ON d.id=p.directive_id \
         JOIN agent_assignments a ON a.id=p.assignment_id JOIN agent_runs r ON r.id=p.child_run_id \
         WHERE d.id=?1 AND d.scan_id=?2 AND d.attempt_number=?3 AND d.target_key=?4 \
         AND d.root_run_id=?5 AND d.claim_run_id=?5 AND d.claim_lease_epoch=?6 AND d.claim_fencing_token=?7 \
         AND a.coordinator_run_id=?5 AND a.child_run_id=r.id AND a.target_key=?4 \
         AND a.lease_epoch=?6 AND a.fencing_token=?7 AND a.lane='read_only_analysis' \
         AND r.root_run_id=?5 AND r.assignment_id=a.id AND r.role=a.role AND r.scan_id=?2 \
         AND r.attempt_number=?3 AND r.target_url=?4",
        params![id,lease.scan_id,lease.attempt_number,lease.target_key,lease.root_run_id,
            lease.lease_epoch,lease.fencing_token],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,
            row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,String>(5)?,
            row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?)),
    ).optional().map_err(|e| e.to_string())?;
    row.map(
        |(
            assignment_id,
            run_id,
            role,
            input,
            revision,
            request_message_id,
            state,
            response,
            usage,
        )| {
            let role = match role.as_str() {
                "spa_api_mapper" => AgentRole::SpaApiMapper,
                "deep_investigator" => AgentRole::DeepInvestigator,
                _ => return Err("directive_proposal_role_invalid".into()),
            };
            let job = ProposalJob {
                directive_id: id.into(),
                child: scheduler::ScheduledChild {
                    assignment_id,
                    run_id,
                    role,
                },
                input: serde_json::from_str(&input).map_err(|e| e.to_string())?,
                revision,
                request_message_id,
                state,
                response: serde_json::from_str(&response).map_err(|e| e.to_string())?,
                usage: serde_json::from_str(&usage).map_err(|e| e.to_string())?,
            };
            validate_frozen_job(&draft, lease, &job)?;
            Ok(job)
        },
    )
    .transpose()
}

fn validate_frozen_job(
    draft: &HumanDirectiveDraft,
    scope: &CoordinatorLease,
    job: &ProposalJob,
) -> Result<(), String> {
    let roles: Vec<_> = draft
        .requested_roles
        .iter()
        .filter(|r| r.as_str() != "coordinator")
        .map(String::as_str)
        .collect();
    if roles != vec![job.child.role.as_str()]
        || draft.side_effect_class != "read_only"
        || !draft.requested_contracts.is_empty()
        || draft.proposed_scope_change.is_some()
        || draft.estimated_tokens != PROPOSAL_TOKENS
        || draft.estimated_requests != 1
        || job.revision != draft.revision
        || job.input["kind"] != "human_requested_assessment"
        || job.input["directiveId"] != job.directive_id
        || job.input["target"] != scope.target_key
        || job.input["request"] != draft.safe_execution_text
        || job.input["referencedFactIds"] != json!(draft.referenced_fact_ids)
        || job.input["constraints"]
            != json!({"webOnly":true,"targetRequests":0,"tools":[],"advisoryOnly":true})
    {
        return Err("directive_proposal_source_invalid".into());
    }
    Ok(())
}
