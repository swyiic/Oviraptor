//! Phase-scoped human analysis preferences, not Web queue actions or authority.
//! Empty snapshots matter: later chat must not rewrite an already frozen phase.
use super::*;
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{scheduler::ScheduledChild, source},
    store::stable_hash,
};
use serde_json::Value;

mod freeze;
mod interpretation;
mod projection;

pub(crate) use freeze::{freeze, freeze_review_in_transaction, freeze_round_in_transaction};
pub(super) use interpretation::interpret_focus;
pub(crate) use projection::project_delivery;

fn key(role: AgentRole, tools: bool) -> Result<String, String> {
    if !source::is_source_role(role) {
        return Err("source_guidance_role_invalid".into());
    }
    Ok(format!(
        "{}:{}",
        if tools { "tools" } else { "assessment" },
        role.as_str()
    ))
}

#[derive(Clone, Copy)]
enum Phase {
    Analysis(AgentRole, bool),
    ToolRound(AgentRole, i64),
    CandidateReview,
    CoverageReview,
}

impl Phase {
    fn role(self) -> AgentRole {
        match self {
            Self::Analysis(role, _) | Self::ToolRound(role, _) => role,
            Self::CandidateReview | Self::CoverageReview => AgentRole::EvidenceReviewer,
        }
    }

    fn key(self) -> Result<String, String> {
        match self {
            Self::Analysis(role, tools) => key(role, tools),
            Self::ToolRound(role, number) if (2..=256).contains(&number) => {
                Ok(format!("{}:round:{number}", key(role, true)?))
            }
            Self::ToolRound(..) => Err("source_guidance_round_invalid".into()),
            Self::CandidateReview => Ok("review:source_candidates".into()),
            Self::CoverageReview => Ok("review:source_coverage".into()),
        }
    }

    fn trigger(self) -> &'static str {
        match self {
            Self::Analysis(_, false) => "source_results_ready",
            Self::Analysis(_, true) | Self::ToolRound(..) => "source_tools_ready",
            Self::CandidateReview => "source_candidates_ready",
            Self::CoverageReview => "source_coverage_ready",
        }
    }

    fn tools(self) -> bool {
        matches!(self, Self::Analysis(_, true) | Self::ToolRound(..))
    }

    fn round(self) -> i64 {
        match self {
            Self::ToolRound(_, number) => number,
            _ => 1,
        }
    }

    fn lane(self) -> &'static str {
        match self {
            Self::Analysis(..) | Self::ToolRound(..) => "read_only_analysis",
            _ => "review",
        }
    }

    fn available(self, db: &Connection, lease: &CoordinatorLease) -> Result<bool, String> {
        match self {
            Self::Analysis(role, _) | Self::ToolRound(role, _) => {
                source::task_slice(db, lease, role).map(|_| true)
            }
            Self::CandidateReview => {
                super::super::source_reviewer::task_slice(db, lease).map(|slice| slice.is_some())
            }
            Self::CoverageReview => super::super::source_coverage_reviewer::task_slice(db, lease)
                .map(|slice| slice.is_some()),
        }
    }
}

fn review_phase(coverage: bool) -> Phase {
    if coverage {
        Phase::CoverageReview
    } else {
        Phase::CandidateReview
    }
}

// Reviewer roles are shared by Web and source. Select from the persisted root
// surface and assignment trigger, never merely from the requested role.
fn child_phase(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    tools: bool,
) -> Result<Option<Phase>, String> {
    if source::is_source_role(child.role) {
        return Ok(Some(Phase::Analysis(child.role, tools)));
    }
    if child.role != AgentRole::EvidenceReviewer
        || !source::uses_source_runtime(db, lease, child.role)?
    {
        return Ok(None);
    }
    let trigger: String = db
        .query_row(
            "SELECT trigger_code FROM agent_assignments WHERE id=?1 AND child_run_id=?2
         AND coordinator_run_id=?3 AND role='evidence_reviewer' AND lane='review'
         AND lease_epoch=?4 AND fencing_token=?5 AND target_key=?6",
            params![
                child.assignment_id,
                child.run_id,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token,
                lease.target_key
            ],
            |r| r.get(0),
        )
        .map_err(|_| "source_guidance_assignment_binding_invalid")?;
    match (tools, trigger.as_str()) {
        (false, "source_candidates_ready") => Ok(Some(Phase::CandidateReview)),
        (false, "source_coverage_ready") => Ok(Some(Phase::CoverageReview)),
        _ => Err("source_guidance_phase_invalid".into()),
    }
}

fn load(db: &Connection, lease: &CoordinatorLease, phase: Phase) -> Result<Option<Value>, String> {
    let role = phase.role();
    let phase = phase.key()?;
    let row: Option<(String, i64, String, String, String)> = db.query_row(
        "SELECT role,lease_epoch,fencing_token,guidance_json,guidance_hash FROM agent_source_guidance WHERE root_run_id=?1 AND phase_key=?2",
        params![lease.root_run_id,phase], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((saved_role, epoch, fence, text, hash)) = row else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| "source_guidance_json_invalid")?;
    if saved_role != role.as_str()
        || epoch != lease.lease_epoch
        || fence != lease.fencing_token
        || hash != stable_hash(&text)
        || value["schemaVersion"] != 1
        || value["phase"] != phase
        || value["role"] != role.as_str()
        || !value["directives"].is_array()
    {
        return Err("source_guidance_binding_invalid".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for item in value["directives"].as_array().unwrap() {
        let id = item["id"].as_str().ok_or("source_guidance_item_invalid")?;
        let text = item["text"]
            .as_str()
            .ok_or("source_guidance_item_invalid")?;
        let draft_hash = item["draftHash"]
            .as_str()
            .ok_or("source_guidance_item_invalid")?;
        let source: String = db.query_row("SELECT source_draft_id FROM agent_user_directives WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND root_run_id=?4 AND target_key=?5 AND claim_run_id=?4 AND claim_lease_epoch=?6 AND claim_fencing_token=?7 AND text_redacted=?8 AND confirmed_hash=?9",
            params![id,lease.scan_id,lease.attempt_number,lease.root_run_id,lease.target_key,lease.lease_epoch,lease.fencing_token,text,draft_hash],|r|r.get(0))
            .map_err(|_|"source_guidance_directive_binding_invalid")?;
        let draft = load_draft(db, &source)?.ok_or("source_guidance_draft_missing")?;
        if !ids.insert(id)
            || !draft_integrity_valid(&draft)
            || draft.status != "confirmed"
            || draft.confirmed_directive_id != id
            || draft.draft_hash != item["draftHash"]
            || draft.safe_execution_text != item["text"]
            || draft.root_run_id != lease.root_run_id
            || draft.target_key != lease.target_key
            || draft.bound_lease_epoch != lease.lease_epoch
            || draft.bound_fencing_token != lease.fencing_token
            || (draft.intent == "source_analysis_focus" && draft.requested_roles != [role.as_str()])
            || (phase.contains(":round:")
                && !draft
                    .reason_codes
                    .iter()
                    .any(|code| code == "source_guidance_tool_round_eligible"))
        {
            return Err("source_guidance_draft_invalid".into());
        }
    }
    Ok(Some(value))
}

pub(crate) fn delivered_ids(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    tools: bool,
) -> Result<Vec<String>, String> {
    let Some(phase) = child_phase(db, lease, child, tools)? else {
        return Ok(Vec::new());
    };
    phase_ids(db, lease, phase)
}

fn phase_ids(
    db: &Connection,
    lease: &CoordinatorLease,
    phase: Phase,
) -> Result<Vec<String>, String> {
    let Some(value) = load(db, lease, phase)? else {
        return Ok(Vec::new());
    };
    value["directives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "source_guidance_item_invalid".into())
        })
        .collect()
}

fn tool_round_phase(role: AgentRole, number: i64) -> Phase {
    if number == 1 {
        Phase::Analysis(role, true)
    } else {
        Phase::ToolRound(role, number)
    }
}

pub(crate) fn round_ids(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    number: i64,
) -> Result<Vec<String>, String> {
    phase_ids(db, lease, tool_round_phase(child.role, number))
}

/// Append only the persisted next-round focus; never change prior messages or
/// read newly arrived chat here. An empty snapshot also freezes that boundary.
pub(crate) fn attach_round(
    db: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    number: i64,
    mut request: Value,
) -> Result<Value, String> {
    let input = attach_phase(db, lease, Phase::ToolRound(role, number), json!({}))?;
    if input.get("operatorGuidance").is_some() {
        request["messages"]
            .as_array_mut()
            .ok_or("source_guidance_round_messages_invalid")?
            .push(json!({"role":"user","content":input.to_string()}));
    }
    Ok(request)
}

pub(crate) fn attach(
    db: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    tools: bool,
    input: Value,
) -> Result<Value, String> {
    attach_phase(db, lease, Phase::Analysis(role, tools), input)
}

pub(crate) fn attach_review(
    db: &Connection,
    lease: &CoordinatorLease,
    coverage: bool,
    input: Value,
) -> Result<Value, String> {
    attach_phase(db, lease, review_phase(coverage), input)
}

fn attach_phase(
    db: &Connection,
    lease: &CoordinatorLease,
    phase: Phase,
    mut input: Value,
) -> Result<Value, String> {
    if let Some(value) = load(db, lease, phase)? {
        if !value["directives"].as_array().unwrap().is_empty() {
            input["operatorGuidance"] = value;
        }
    }
    Ok(input)
}

pub(crate) fn validate_input(
    db: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    tools: bool,
    input: &Value,
) -> Result<(), String> {
    let expected = attach(db, lease, role, tools, json!({}))?;
    if input.get("operatorGuidance") != expected.get("operatorGuidance") {
        return Err("source_guidance_input_mismatch".into());
    }
    Ok(())
}

/// Persisted atomically with the actual response/event. This proves delivery,
/// not that a suggested vulnerability was confirmed or an action was performed.
pub(crate) fn record_delivery(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    tools: bool,
    sequence: i64,
) -> Result<(), String> {
    let Some(phase) = child_phase(tx, lease, child, tools)? else {
        return Ok(());
    };
    record_phase_delivery(tx, lease, child, phase, sequence)
}

pub(crate) fn record_round_delivery(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    number: i64,
    sequence: i64,
) -> Result<(), String> {
    record_phase_delivery(
        tx,
        lease,
        child,
        tool_round_phase(child.role, number),
        sequence,
    )
}

fn record_phase_delivery(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    phase: Phase,
    sequence: i64,
) -> Result<(), String> {
    let Some(value) = load(tx, lease, phase)? else {
        return Ok(());
    };
    let items = value["directives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| UserDirective {
            id: item["id"].as_str().unwrap().into(),
            text: item["text"].as_str().unwrap().into(),
            status: "accepted".into(),
        })
        .collect::<Vec<_>>();
    record_model_delivery(tx, lease, &child.run_id, sequence, &items)?;
    for item in items {
        let receipt = json!({"phase":phase.key()?,"assignmentId":child.assignment_id,
            "childRunId":child.run_id,"eventSequence":sequence,"inputHash":stable_hash(&value.to_string()),
            "state":"model_received","advisoryOnly":true});
        let n=tx.execute("UPDATE agent_user_directives SET payload_json=json_set(payload_json,'$.sourceGuidance',json(?2)) WHERE id=?1 AND status='accepted'",
            params![item.id,receipt.to_string()]).map_err(|e|e.to_string())?;
        if n != 1 {
            return Err("source_guidance_delivery_conflict".into());
        }
    }
    verify_phase_delivery(tx, lease, child, phase, sequence)
}

pub(crate) fn verify_delivery(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    tools: bool,
    sequence: i64,
) -> Result<(), String> {
    let Some(phase) = child_phase(db, lease, child, tools)? else {
        return Ok(());
    };
    verify_phase_delivery(db, lease, child, phase, sequence)
}

pub(crate) fn verify_round_delivery(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    number: i64,
    sequence: i64,
) -> Result<(), String> {
    verify_phase_delivery(
        db,
        lease,
        child,
        tool_round_phase(child.role, number),
        sequence,
    )
}

fn verify_phase_delivery(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    phase: Phase,
    sequence: i64,
) -> Result<(), String> {
    let Some(value) = load(db, lease, phase)? else {
        return Ok(());
    };
    for item in value["directives"].as_array().unwrap() {
        let text: String = db
            .query_row(
                "SELECT payload_json FROM agent_user_directives WHERE id=?1",
                [item["id"].as_str()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let payload: Value =
            serde_json::from_str(&text).map_err(|_| "source_guidance_delivery_invalid")?;
        let expected = json!({"phase":phase.key()?,"assignmentId":child.assignment_id,
            "childRunId":child.run_id,"eventSequence":sequence,"inputHash":stable_hash(&value.to_string()),
            "state":"model_received","advisoryOnly":true});
        if payload["sourceGuidance"] != expected
            || payload["modelDelivery"]["runId"] != child.run_id
            || payload["modelDelivery"]["eventSequence"] != sequence
            || payload["modelDelivery"]["state"] != "model_received"
        {
            return Err("source_guidance_delivery_invalid".into());
        }
    }
    Ok(())
}
