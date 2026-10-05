//! Read-only proof of both initial assessments and both source-tool phases.
//! Scheduler admission, Reviewer delivery and root closure share this exact
//! audit; none may replace mailbox/receipt/usage proof with a completed label.
use super::{
    lease::CoordinatorLease, scheduler::ScheduledChild, source, source_rounds, specialist,
};
use crate::agent_runtime::{contract::AgentRole, secrets::redact_json, store::stable_hash};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};

pub(crate) struct PhaseProof {
    pub bases: Vec<(AgentRole, JsonValue)>,
    pub tokens: i64,
    pub cached: i64,
    pub requests: i64,
    pub verified: i64,
    pub manifest: Vec<JsonValue>,
    pub assessments: Vec<JsonValue>,
    pub review_material: JsonValue,
}

pub(crate) fn audit(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<PhaseProof, String> {
    audit_exact(connection, lease, 4)
}

// A crash after the first source-tool assignment has completed must not
// schedule it again with a newly calculated reservation. Reuse only a fully
// audited three-phase prefix; an in-flight round has no completion proof.
pub(crate) fn audit_first_tool_prefix(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<PhaseProof, String> {
    audit_exact(connection, lease, 3)
}

// Initial assessment recovery used to rely on assignment idempotency alone.
// Audit the completed prefix before reacquiring a lease or activating the root,
// so corrupt receipts cannot turn a retry into an erroneous terminal write.
pub(crate) fn audit_initial_prefix(
    connection: &Connection,
    lease: &CoordinatorLease,
    count: usize,
) -> Result<PhaseProof, String> {
    if !matches!(count, 1 | 2) {
        return Err("source_completion_initial_prefix_count_invalid".into());
    }
    audit_exact(connection, lease, count)
}

// A saved but undelivered initial response is the one permitted extra row.
// Its binding, receipt and reservation are audited separately by admission;
// this proof covers only the already acknowledged completed prefix.
pub(crate) fn audit_initial_received_prefix(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<PhaseProof, String> {
    audit_exact_rows(connection, lease, 1, true)
}

// A pending source-tools child is not a completed phase. Audit only its
// acknowledged predecessors while the separate round audit proves that the
// sole extra child owns a known, durable model response.
pub(crate) fn audit_completed_tool_prefix(
    connection: &Connection,
    lease: &CoordinatorLease,
    count: usize,
) -> Result<PhaseProof, String> {
    if !matches!(count, 2 | 3) {
        return Err("source_completion_tool_prefix_count_invalid".into());
    }
    audit_exact_rows(connection, lease, count, true)
}

fn audit_exact(
    connection: &Connection,
    lease: &CoordinatorLease,
    expected_count: usize,
) -> Result<PhaseProof, String> {
    audit_exact_rows(connection, lease, expected_count, false)
}

fn audit_exact_rows(
    connection: &Connection,
    lease: &CoordinatorLease,
    expected_count: usize,
    completed_only: bool,
) -> Result<PhaseProof, String> {
    if connection.is_autocommit() {
        return Err("source_completion_transaction_required".into());
    }
    let bases = source::completion_task_slices(connection, lease)?;
    let rows=connection.prepare("SELECT id,child_run_id,role,task_slice_json,evidence_revision FROM agent_assignments WHERE coordinator_run_id=?1 AND role<>'evidence_reviewer' AND (?2=0 OR state='completed') ORDER BY id")
        .map_err(|e|e.to_string())?.query_map(params![lease.root_run_id,completed_only],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?)))
        .map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if rows.len() != expected_count {
        return Err("source_completion_assignments_missing_or_extra".into());
    }
    let mut phases = std::collections::BTreeSet::new();
    let mut tokens = 0_i64;
    let mut cached = 0_i64;
    let mut requests = 0_i64;
    let mut verified = 0_i64;
    let mut manifest = Vec::new();
    let mut assessments = Vec::new();
    let mut source_audits = Vec::new();
    for (assignment_id, run_id, role, slice, revision) in rows {
        let (agent_role, base) = bases
            .iter()
            .find(|(r, _)| r.as_str() == role)
            .ok_or("source_completion_role_invalid")?;
        let child = ScheduledChild {
            assignment_id,
            run_id,
            role: *agent_role,
        };
        let slice: JsonValue =
            serde_json::from_str(&slice).map_err(|_| "source_completion_slice_invalid")?;
        let tools = slice["phase"] == "source_tools";
        if !phases.insert((role.clone(), tools)) {
            return Err("source_completion_duplicate_phase".into());
        }
        let mut expected = base.clone();
        if tools {
            expected["phase"] = json!("source_tools");
            expected["evidenceRevision"] = json!(revision);
            expected["toolsGranted"] = json!(source::tool_capabilities(child.role)?);
        } else if revision != 1 {
            return Err("source_completion_assessment_revision_invalid".into());
        }
        if slice != expected || revision < 1 {
            return Err("source_completion_contract_changed".into());
        }
        let bound:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
            WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.role=?4 AND a.lane='read_only_analysis'
            AND a.target_key=?5 AND a.lease_epoch=?6 AND a.fencing_token=?7 AND a.state='completed'
            AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at<>''
            AND r.assignment_id=a.id AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.role=a.role AND r.lane=a.lane
            AND r.scan_id=?8 AND r.attempt_number=?9 AND r.target_url=?5 AND r.backend='native'
            AND r.status='terminal' AND r.terminal_state='completed' AND r.cancel_requested_at='' AND r.finished_at<>'')
            AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)
            AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')",
            params![child.assignment_id,child.run_id,lease.root_run_id,role,lease.target_key,lease.lease_epoch,lease.fencing_token,lease.scan_id,lease.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !bound {
            return Err("source_completion_child_binding_invalid".into());
        }
        let (payload, usage, tool_count, kind, prefix) = if tools {
            let audit = source_rounds::audit_completion(connection, lease, &child)?;
            let (output, usage, count) =
                (audit.output.clone(), audit.usage, audit.verified_tool_count);
            source_audits.push(audit);
            (
                redact_json(
                    &json!({"sourceTask":slice,"summary":output["summary"],"result":output,"usage":usage.as_json(),"independentReviewCompleted":false}),
                ),
                usage,
                count,
                "source_tool_result",
                "source-tools",
            )
        } else {
            let receipt = specialist::source_received_for_completion(connection, lease, &child)?;
            if !receipt.rejection.is_empty() {
                return Err("source_completion_assessment_rejected".into());
            }
            (
                redact_json(&json!({"sourceTask":slice,"summary":receipt.text})),
                receipt.usage,
                0,
                "evidence_summary",
                if role == "repo_mapper" {
                    "repo-mapper"
                } else {
                    "source-analyst"
                },
            )
        };
        let correlation = format!("{prefix}:{}", child.assignment_id);
        let dedup = format!("{}:{kind}:{correlation}:{revision}", child.assignment_id);
        let message:(String,String)=connection.query_row("SELECT id,payload_json FROM agent_messages WHERE assignment_id=?1
            AND root_run_id=?2 AND run_id=?2 AND from_run_id=?3 AND to_run_id=?2 AND from_agent=?4 AND to_agent='coordinator'
            AND kind=?5 AND evidence_revision=?6 AND correlation_id=?7 AND dedup_key=?8
            AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1
            AND (SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind IN ('source_tool_result','evidence_summary'))=1",
            params![child.assignment_id,lease.root_run_id,child.run_id,role,kind,revision,correlation,dedup],|r|Ok((r.get(0)?,r.get(1)?)))
            .map_err(|_|"source_completion_mailbox_missing_or_unconfirmed")?;
        if serde_json::from_str::<JsonValue>(&message.1)
            .map_err(|_| "source_completion_mailbox_invalid")?
            != payload
        {
            return Err("source_completion_mailbox_receipt_mismatch".into());
        }
        let used: (i64, i64, i64) = connection
            .query_row(
                "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
                [&child.run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        if used
            != (
                usage.total_tokens,
                usage.cached_input_tokens,
                usage.model_requests,
            )
        {
            return Err("source_completion_usage_mismatch".into());
        }
        let add = |a: i64, b: i64| a.checked_add(b).ok_or("source_completion_usage_overflow");
        tokens = add(tokens, usage.total_tokens)?;
        cached = add(cached, usage.cached_input_tokens)?;
        requests = add(requests, usage.model_requests)?;
        verified = add(verified, tool_count)?;
        manifest.push(json!({"assignmentId":child.assignment_id,"runId":child.run_id,"role":role,
            "phase":if tools {"source_tools"} else {"initial_assessment"},"evidenceRevision":revision,
            "messageId":message.0,"payloadHash":stable_hash(&payload.to_string()),"usage":usage.as_json(),"verifiedToolResults":tool_count}));
        let mut projection =
            json!({"role":role,"assignmentId":child.assignment_id,"runId":child.run_id});
        projection[if tools { "result" } else { "assessment" }] = payload;
        assessments.push((tools, role != "repo_mapper", projection));
    }
    if expected_count < 4 {
        let mut prefix = vec![("repo_mapper".to_string(), false)];
        if expected_count >= 2 {
            prefix.push(("source_analyst".to_string(), false));
        }
        if expected_count == 3 {
            prefix.push(("repo_mapper".to_string(), true));
        }
        if phases != prefix.into_iter().collect() {
            return Err("source_completion_partial_phase_not_prefix".into());
        }
    }
    assessments.sort_by_key(|(tools, analyst, _)| (*tools, *analyst));
    let review_material = if expected_count == 4 {
        crate::agent_runtime::multi_agent::source_review::capture(
            connection,
            lease,
            &source_audits,
        )?
    } else {
        JsonValue::Null
    };
    Ok(PhaseProof {
        bases,
        tokens,
        cached,
        requests,
        verified,
        manifest,
        assessments: assessments.into_iter().map(|(_, _, value)| value).collect(),
        review_material,
    })
}
