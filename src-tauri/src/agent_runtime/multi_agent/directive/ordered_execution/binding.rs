use super::super::ordered_plan::{OrderedAssessmentAction, OrderedAssessmentPlan};
use super::*;
// Typed SQL row projections keep the selected column order explicit.
type SourceRecord = (String,i64,String,String,String,String,String,String,String,i64,String,String,String,i64,String);
type ActionRecord = (String,String,String,String,String,String,String,String,String,String,String,String,String,i64,String);
#[derive(Clone, Debug)]
pub(crate) struct ActionJob {
    pub directive_id: String,
    pub draft: HumanDirectiveDraft,
    pub plan: OrderedAssessmentPlan,
    pub action: OrderedAssessmentAction,
    pub scope: CoordinatorLease,
    pub child: scheduler::ScheduledChild,
    pub input: Value,
    pub request_message_id: String,
    pub state: String,
    pub response: Value,
    pub usage: Value,
    pub result_message_id: String,
}
pub(crate) fn source(
    db: &Connection,
    id: &str,
) -> Result<(HumanDirectiveDraft, CoordinatorLease, String), String> {
    let (source,revision,hash,raw,text,recipient,thread,state,scan,attempt,root,target,claim,epoch,fence):
      SourceRecord=db.query_row(
      "SELECT source_draft_id,confirmed_revision,confirmed_hash,payload_json,text_redacted,recipient_role,thread_key,status,
       scan_id,attempt_number,root_run_id,target_key,claim_run_id,claim_lease_epoch,claim_fencing_token FROM agent_user_directives WHERE id=?1",
      [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?))).map_err(|_|"ordered_source_missing")?;
    let draft = load_draft(db, &source)?.ok_or("ordered_source_missing")?;
    let payload: Value = serde_json::from_str(&raw).map_err(|_| "ordered_source_unverified")?;
    let plan = draft
        .readonly_assessment_plan
        .as_ref()
        .filter(|p| p.schema_version == 3 && p.dispatch_state == "requires_dispatch_checks")
        .ok_or("ordered_v3_confirmation_required")?;
    if !draft_integrity_valid(&draft)
        || draft.status != "confirmed"
        || draft.confirmed_directive_id != id
        || draft.revision != revision
        || draft.draft_hash != hash
        || draft.safe_execution_text != text
        || draft.recipient_role != recipient
        || draft.thread_key != thread
        || draft.scan_id != scan
        || draft.attempt_number != attempt
        || draft.root_run_id != root
        || draft.target_key != target
        || (if state == "pending" {
            !claim.is_empty() || epoch != 0 || !fence.is_empty()
        } else {
            draft.bound_lease_epoch != epoch || draft.bound_fencing_token != fence || claim != root
        })
        || !matches!(
            state.as_str(),
            "pending" | "claimed" | "accepted" | "assigned" | "completed" | "failed" | "deferred"
        )
        || !payload_matches(db, &draft, &payload, &state)?
        || plan.actions.len() != 2
    {
        return Err("ordered_source_unverified".into());
    }
    super::super::human_review::receipt_for(db, &draft.id)?
        .ok_or("ordered_original_human_receipt_required")?;
    let scope = CoordinatorLease {
        scan_id: scan,
        attempt_number: attempt,
        root_run_id: root,
        target_key: target,
        lease_epoch: draft.bound_lease_epoch,
        fencing_token: draft.bound_fencing_token.clone(),
        lease_expires_at: String::new(),
    };
    Ok((draft, scope, state))
}

pub(crate) fn load(db: &Connection, id: &str, order: i64) -> Result<Option<ActionJob>, String> {
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_directive_ordered_actions WHERE directive_id=?1 AND action_order=?2)",
        params![id,order],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {
        return Ok(None);
    }
    let (draft, scope, _) = source(db, id)?;
    let plan = draft
        .readonly_assessment_plan
        .clone()
        .ok_or("ordered_plan_missing")?;
    let action = plan
        .actions
        .get(usize::try_from(order - 1).map_err(|_| "ordered_order_invalid")?)
        .cloned()
        .ok_or("ordered_order_invalid")?;
    let (action_id,plan_hash,previous,assignment,child,message,input,state,response,usage,result,role,slice,rev,trigger):
      ActionRecord=db.query_row(
      "SELECT x.action_id,x.plan_hash,x.previous_action_id,x.assignment_id,x.child_run_id,x.request_message_id,x.input_json,
        x.state,x.response_json,x.usage_json,x.result_message_id,a.role,a.task_slice_json,a.evidence_revision,a.trigger_code
       FROM agent_directive_ordered_actions x JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_runs r ON r.id=x.child_run_id
       WHERE x.directive_id=?1 AND x.action_order=?2 AND a.child_run_id=r.id AND r.assignment_id=a.id AND a.coordinator_run_id=?3
       AND r.root_run_id=?3 AND r.parent_run_id=?3 AND a.target_key=?4 AND r.target_url=?4 AND r.scan_id=?5 AND r.attempt_number=?6
       AND a.lease_epoch=?7 AND a.fencing_token=?8 AND a.lane='read_only_analysis' AND r.lane=a.lane AND r.role=a.role",
      params![id,order,scope.root_run_id,scope.target_key,scope.scan_id,scope.attempt_number,scope.lease_epoch,scope.fencing_token],
      |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?))).map_err(|_|"ordered_job_unverified")?;
    let role = match role.as_str() {
        "spa_api_mapper" => AgentRole::SpaApiMapper,
        "deep_investigator" => AgentRole::DeepInvestigator,
        _ => return Err("ordered_role_unimplemented".into()),
    };
    if action_id != action.action_id
        || plan_hash != plan.plan_hash
        || previous != action.previous_action_id.clone().unwrap_or_default()
        || action.role != role.as_str()
        || rev != draft.revision
        || input != slice
        || trigger != format!("human_ordered:{id}:{order}")
    {
        return Err("ordered_job_unverified".into());
    }
    let input: Value = serde_json::from_str(&input).map_err(|_| "ordered_input_unverified")?;
    if input["kind"] != "human_ordered_readonly_assessment"
        || input["directiveId"] != id
        || input["actionId"] != action_id
        || input["order"] != order
        || input["planHash"] != plan_hash
        || input["target"] != scope.target_key
        || input["request"] != draft.safe_execution_text
        || input["referencedFactIds"] != json!(draft.referenced_fact_ids)
        || input["constraints"]
            != json!({"webOnly":true,"targetRequests":0,"tools":[],"advisoryOnly":true,"independentReviewApproved":false})
        || input.to_string().len() > 2600
    {
        return Err("ordered_input_unverified".into());
    }
    let job = ActionJob {
        directive_id: id.into(),
        draft,
        plan,
        action,
        scope,
        child: scheduler::ScheduledChild {
            assignment_id: assignment,
            run_id: child,
            role,
        },
        input,
        request_message_id: message,
        state,
        response: serde_json::from_str(&response).map_err(|_| "ordered_response_unverified")?,
        usage: serde_json::from_str(&usage).map_err(|_| "ordered_usage_unverified")?,
        result_message_id: result,
    };
    verify_request(db, &job, false)?;
    Ok(Some(job))
}

pub(super) fn verify_request(
    db: &Connection,
    job: &ActionJob,
    consumed: bool,
) -> Result<(), String> {
    let (raw,delivered,ack):(String,String,String)=db.query_row("SELECT payload_json,delivered_at,acknowledged_at FROM agent_messages
       WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?2 AND to_run_id=?3 AND from_agent='coordinator' AND to_agent=?4
       AND kind='human_ordered_assessment_request' AND correlation_id=?5 AND assignment_id=?6 AND evidence_revision=?7 AND dedup_key=?8",
       params![job.request_message_id,job.scope.root_run_id,job.child.run_id,job.child.role.as_str(),job.action.action_id,job.child.assignment_id,job.draft.revision,format!("{}:human_ordered_assessment_request:{}:{}",job.child.assignment_id,job.action.action_id,job.draft.revision)],
       |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"ordered_request_unverified")?;
    let value: Value = serde_json::from_str(&raw).map_err(|_| "ordered_request_unverified")?;
    if value != job.input || (consumed && (delivered.is_empty() || ack.is_empty())) {
        return Err("ordered_request_unverified".into());
    }
    Ok(())
}

fn payload_matches(
    db: &Connection,
    draft: &HumanDirectiveDraft,
    payload: &Value,
    state: &str,
) -> Result<bool, String> {
    let mut original = payload.clone();
    let closure = original
        .as_object_mut()
        .ok_or("ordered_source_unverified")?
        .remove("taskClosure");
    if original != confirmed_payload(draft) {
        return Ok(false);
    }
    let Some(closure) = closure else {
        return Ok(true);
    };
    let keys = [
        "fromStatus",
        "disposition",
        "rootTerminalCode",
        "requiresReconciliation",
        "automaticRetry",
        "closedAt",
    ];
    let shape = closure
        .as_object()
        .is_some_and(|v| v.len() == keys.len() && keys.iter().all(|k| v.contains_key(*k)));
    if !shape
        || state != "deferred"
        || closure["automaticRetry"] != false
        || !matches!(
            closure["fromStatus"].as_str(),
            Some("pending" | "claimed" | "accepted" | "assigned" | "applied")
        )
        || !matches!(
            closure["disposition"].as_str(),
            Some("not_applied" | "reconciliation_required")
        )
        || closure["requiresReconciliation"]
            != (closure["disposition"] == "reconciliation_required")
        || !closure["closedAt"].as_str().is_some_and(|s| !s.is_empty())
    {
        return Ok(false);
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_user_directives d ON d.root_run_id=r.id
      WHERE d.id=?1 AND r.status='terminal' AND r.terminal_code=?2 AND d.finished_at=?3
      AND EXISTS(SELECT 1 FROM agent_collaboration_events e WHERE e.scan_id=d.scan_id AND e.attempt_number=d.attempt_number
       AND e.event_type='user_directive' AND e.entity_id=d.id AND json_extract(e.payload_json,'$.status')='deferred'
       AND json_extract(e.payload_json,'$.sourceDraftId')=d.source_draft_id AND json_extract(e.payload_json,'$.rejectionCode')=d.rejection_code))",
      params![draft.confirmed_directive_id,closure["rootTerminalCode"].as_str(),closure["closedAt"].as_str()],|r|r.get(0)).map_err(|e|e.to_string())
}
