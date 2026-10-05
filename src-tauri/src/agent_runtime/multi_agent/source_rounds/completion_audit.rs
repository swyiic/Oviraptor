//! Read-only completion proof for a source specialist.
use super::*;

/// Audit every round, transcript link and tool receipt of a completed source
/// phase. This never executes a tool, renews a lease or grants a capability.
/// Count successful data tools, not finish markers or denied calls.
#[derive(Debug)]
pub(crate) struct SourceRoundAudit {
    pub output: Value,
    pub usage: UsageDelta,
    pub verified_tool_count: i64,
    // Only the receipt verifier below can construct these material entries.
    tool_evidence: Vec<Value>,
}

impl SourceRoundAudit {
    pub(crate) fn tool_evidence(&self) -> &[Value] {
        &self.tool_evidence
    }
}

struct CheckedRounds {
    last: PendingRound,
    verified_tool_count: i64,
    tool_evidence: Vec<Value>,
}

pub(crate) fn audit_completion(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<SourceRoundAudit, String> {
    let checked = audit_rounds(db, lease, child)?;
    let (output, usage) = completion(db, &checked.last)?;
    Ok(SourceRoundAudit {
        output,
        usage,
        verified_tool_count: checked.verified_tool_count,
        tool_evidence: checked.tool_evidence,
    })
}

/// Known exhausted usage proves failure accounting only, never completion.
pub(crate) fn audit_exhausted(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<UsageDelta, String> {
    let checked = audit_rounds(db, lease, child)?;
    let last = &checked.last;
    let count: i64 = db.query_row(
        "SELECT count(*) FROM agent_source_model_rounds WHERE assignment_id=?1 OR child_run_id=?2",
        params![child.assignment_id, child.run_id], |r| r.get(0),
    ).map_err(|e|e.to_string())?;
    if last.number != 3 || count != 3 {
        return Err("source_exhausted_exact_rounds_required".into());
    }
    for number in 1..=3 {
        let call = historical(db, last, number)?;
        let receipt = received(
            db,
            &call,
            &load(db, &call)?.ok_or("source_round_history_missing")?,
        )?;
        if receipt.response["rejection"] != ""
            || receipt.response["toolCalls"]
                .as_array()
                .ok_or("source_round_tool_list_invalid")?
                .iter()
                .any(|t| t["name"] == "assignment.finish")
        {
            return Err("source_exhausted_unfinished_rounds_required".into());
        }
    }
    verify_checkpoint(db, last)?;
    cumulative(db, last, 3)
}

fn audit_rounds(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<CheckedRounds, String> {
    if db.is_autocommit() || !source::is_source_role(child.role) {
        return Err("source_completion_receipt_context_invalid".into());
    }
    let number: i64 = db.query_row(
        "SELECT COALESCE(MAX(round_number),0) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2",
        params![child.assignment_id,child.run_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !(1..=3).contains(&number) {
        return Err("source_completion_round_count_invalid".into());
    }
    let seed = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number,
        request: Value::Null,
        reserved_tokens: 0,
    };
    let last = historical(db, &seed, number)?;
    let mut verified = 0;
    let mut tool_evidence = Vec::new();
    let mut expected = source::tool_capabilities(child.role)?;
    expected.sort();
    for index in 1..=number {
        let call = historical(db, &last, index)?;
        let receipt = received(
            db,
            &call,
            &load(db, &call)?.ok_or("source_round_history_missing")?,
        )?;
        verify_request(db, &call)?;
        verify_tool_rows(db, &call, &receipt.response)?;
        let mut names = call.request["tools"]
            .as_array()
            .ok_or("source_round_tools_invalid")?
            .iter()
            .map(|tool| {
                if tool["type"] != "function" {
                    return Err("source_round_tool_invalid");
                }
                tool["function"]["name"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or("source_round_tool_invalid")
            })
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        if names != expected {
            return Err("source_round_tools_changed".into());
        }
        for (tool_index, tool) in receipt.response["toolCalls"]
            .as_array()
            .ok_or("source_round_tool_list_invalid")?
            .iter()
            .enumerate()
        {
            let output = tool_output(db, &call, tool_index as i64, tool)?
                .ok_or("source_round_tools_pending")?;
            if !expected.iter().any(|name| tool["name"] == *name) {
                return Err("source_round_tool_not_granted".into());
            }
            if tool["name"] != "assignment.finish" && output.get("error").is_none() {
                verified += 1;
                tool_evidence.push(json!({
                    "assignmentId":child.assignment_id,"authorRunId":child.run_id,
                    "rootRunId":lease.root_run_id,"roundNumber":index,"callIndex":tool_index,
                    "call":tool,"output":output,
                }));
            }
        }
    }
    Ok(CheckedRounds {
        last,
        verified_tool_count: verified,
        tool_evidence,
    })
}
