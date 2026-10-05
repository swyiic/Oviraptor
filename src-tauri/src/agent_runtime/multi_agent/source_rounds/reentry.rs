//! Read-only audit of a received model round whose local SourceBroker tools
//! were interrupted. This is not a grant to replay provider I/O or renew a
//! fence; the commands layer separately checks the live child authority.
use super::*;

pub(crate) struct ReceivedBoundary {
    pub usage: UsageDelta,
    pub rounds: i64,
    pub settled_finish: bool,
}

pub(crate) fn audit_received_boundary(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<ReceivedBoundary, String> {
    audit_boundary(db, lease, child, false)
}

pub(crate) fn audit_recoverable_boundary(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<ReceivedBoundary, String> {
    audit_boundary(db, lease, child, true)
}

fn audit_boundary(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    allow_settled_finish: bool,
) -> Result<ReceivedBoundary, String> {
    if db.is_autocommit() || !source::is_source_role(child.role) {
        return Err("source_round_reentry_transaction_required".into());
    }
    let (rounds, count): (i64, i64) = db.query_row(
        "SELECT COALESCE(MAX(round_number),0),count(*) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2",
        params![child.assignment_id,child.run_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|e| e.to_string())?;
    if !(1..=3).contains(&rounds) || rounds != count {
        return Err("source_round_reentry_sequence_invalid".into());
    }
    let seed = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number: rounds,
        request: Value::Null,
        reserved_tokens: 0,
    };
    let mut pending = false;
    let mut expected = source::tool_capabilities(child.role)?;
    expected.sort();
    for number in 1..=rounds {
        let call = historical(db, &seed, number)?;
        verify_request(db, &call)?;
        let receipt = received(
            db,
            &call,
            &load(db, &call)?.ok_or("source_round_history_missing")?,
        )?;
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
        if names != expected || receipt.response["rejection"] != "" {
            return Err("source_round_reentry_tools_or_response_invalid".into());
        }
        let calls = receipt.response["toolCalls"]
            .as_array()
            .filter(|calls| !calls.is_empty())
            .ok_or("source_round_reentry_calls_missing")?;
        for (index, tool) in calls.iter().enumerate() {
            if !expected.iter().any(|name| tool["name"] == *name)
                || (number < rounds && tool["name"] == "assignment.finish")
            {
                return Err("source_round_reentry_tool_invalid".into());
            }
            if tool_output(db, &call, index as i64, tool)?.is_none() {
                if number < rounds {
                    return Err("source_round_reentry_prior_tool_pending".into());
                }
                pending = true;
            } else if pending {
                return Err("source_round_reentry_tool_order_invalid".into());
            }
        }
    }
    if !pending && !allow_settled_finish {
        return Err("source_round_reentry_no_local_tool_pending".into());
    }
    let last = historical(db, &seed, rounds)?;
    verify_checkpoint(db, &last)?;
    verify_budget(db, &last)?;
    let usage = cumulative(db, &last, rounds)?;
    let latest = received(
        db,
        &last,
        &load(db, &last)?.ok_or("source_round_history_missing")?,
    )?;
    if latest.usage.total_tokens > last.reserved_tokens {
        return Err("source_round_reentry_over_budget".into());
    }
    let settled_finish = !pending;
    if settled_finish {
        let calls = latest.response["toolCalls"]
            .as_array()
            .ok_or("source_round_tool_list_invalid")?;
        if calls.len() != 1 || calls[0]["name"] != "assignment.finish" {
            return Err("source_round_reentry_no_local_tool_pending".into());
        }
        let proof = audit_completion(db, lease, child)?;
        if proof.usage != usage {
            return Err("source_round_reentry_usage_changed".into());
        }
    }
    Ok(ReceivedBoundary {
        usage,
        rounds,
        settled_finish,
    })
}

/// Only a previously received, single-call terminal round can be settled
/// locally after the model deadline. Nonterminal tools need another model
/// round and must not acquire an implicit extension of that deadline.
pub(crate) fn audit_pending_finish(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<PendingRound>, String> {
    let boundary = audit_received_boundary(db, lease, child)?;
    let seed = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number: boundary.rounds,
        request: Value::Null,
        reserved_tokens: 0,
    };
    let last = historical(db, &seed, boundary.rounds)?;
    let receipt = received(
        db,
        &last,
        &load(db, &last)?.ok_or("source_round_history_missing")?,
    )?;
    let calls = receipt.response["toolCalls"]
        .as_array()
        .ok_or("source_round_tool_list_invalid")?;
    if calls.len() != 1
        || calls[0]["name"] != "assignment.finish"
        || tool_output(db, &last, 0, &calls[0])?.is_some()
    {
        return Ok(None);
    }
    Ok(Some(last))
}

/// A completed terminal receipt can be settled without calling the broker
/// again. The full transcript and child usage must still be provable.
pub(crate) fn audit_settled_finish(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<PendingRound>, String> {
    let boundary = audit_recoverable_boundary(db, lease, child)?;
    if !boundary.settled_finish {
        return Ok(None);
    }
    let seed = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number: boundary.rounds,
        request: Value::Null,
        reserved_tokens: 0,
    };
    historical(db, &seed, boundary.rounds).map(Some)
}

/// `execute_tool` checks its transaction both before and after appending the
/// local receipt. Admit only the saved terminal call in the first state, and
/// require a fully verifiable completion in the second state.
pub(crate) fn audit_saved_finish_transition(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    let state: String = db.query_row(
        "SELECT state FROM agent_source_tool_receipts WHERE assignment_id=?1 AND child_run_id=?2 \
         AND round_number=(SELECT MAX(round_number) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2) \
         AND call_index=0",
        params![child.assignment_id,child.run_id],
        |row| row.get(0),
    ).map_err(|_| "source_round_saved_finish_receipt_missing")?;
    match state.as_str() {
        "planned" => {
            audit_pending_finish(db, lease, child)?.ok_or("source_model_deadline_exceeded")?;
        }
        "completed" => {
            audit_completion(db, lease, child)?;
        }
        _ => return Err("source_round_saved_finish_state_invalid".into()),
    }
    Ok(())
}
