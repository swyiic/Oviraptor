//! Durable model request claims and response receipt persistence.
use super::*;

/// Freeze the next round's human focus before budget admission. No provider I/O
/// or delivery receipt occurs here. Re-entry uses the same immutable snapshot,
/// even when empty; existing legacy rounds never acquire new input.
pub(crate) fn prepare_next_authorized(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    number: i64,
    request: &Value,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<Value, String> {
    if !(2..=256).contains(&number) {
        return Err("source_round_sequence_conflict".into());
    }
    let seed = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number,
        request: request.clone(),
        reserved_tokens: 1,
    };
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    authorize(&tx, &seed)?;
    check(&tx)?;
    verify_checkpoint(&tx, &seed)?;
    let previous = historical(&tx, &seed, number - 1)?;
    if *request != continuation(&tx, &previous)? {
        return Err("source_round_transcript_changed".into());
    }
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?3)",
        params![child.assignment_id, number,child.run_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !exists {
        verify_budget(&tx, &seed)?;
        super::super::directive::source_guidance::freeze_round_in_transaction(
            &tx, lease, child.role, number,
        )?;
    }
    let result = super::super::directive::source_guidance::attach_round(
        &tx,
        lease,
        child.role,
        number,
        request.clone(),
    )?;
    if exists {
        let saved = historical(&tx, &seed, number)?;
        if saved.request != result {
            return Err("source_round_transcript_changed".into());
        }
        verify_request(&tx, &saved)?;
        load(&tx, &saved)?.ok_or("source_round_history_missing")?;
    }
    authorize(&tx, &seed)?;
    check(&tx)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

/// One durable claim per round. A second caller gets the existing receipt or
/// an unknown-outcome error, never permission to repeat provider I/O.
#[cfg(test)]
pub fn start_authorized(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    number: i64,
    request: &Value,
    reserved_tokens: i64,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<Start, String> {
    start_inner(db,lease,child,number,request,reserved_tokens,check,false).map(|(start,_)|start)
}

// Only the actual gateway keeps this non-cloneable owner through SDK and local
// publication. Financial PendingRound clones never carry execution ownership.
pub(crate) fn start_for_transport(
    db:&Connection,lease:&CoordinatorLease,child:&ScheduledChild,number:i64,
    request:&Value,reserved_tokens:i64,check:impl Fn(&Connection)->Result<(),String>,
)->Result<(Start,Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>),String> {
    start_inner(db,lease,child,number,request,reserved_tokens,check,true)
}

#[allow(clippy::too_many_arguments)]
fn start_inner(
    db:&Connection,lease:&CoordinatorLease,child:&ScheduledChild,number:i64,
    request:&Value,reserved_tokens:i64,check:impl Fn(&Connection)->Result<(),String>,
    transport:bool,
)->Result<(Start,Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>),String> {
    if !(1..=256).contains(&number)
        || reserved_tokens < 1
        || request.to_string().len() > 6_000_000
        || redact_json(request) != *request
    {
        return Err("source_round_request_invalid".into());
    }
    let call = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number,
        request: request.clone(),
        reserved_tokens,
    };
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    authorize(&tx, &call)?;
    check(&tx)?;
    verify_checkpoint(&tx, &call)?;
    unsent::require_no_terminal_fact(&tx, &call)?;
    if let Some(row) = load(&tx, &call)? {
        return if row.state == "received" {
            let receipt = received(&tx, &call, &row)?;
            verify_tool_rows(&tx, &call, &receipt.response)?;
            let owner=if transport {Some(super::lifetime::existing(&tx,&call)?)} else {None};
            Ok((Start::Received(call, receipt),owner))
        } else {
            Err("source_round_outcome_unknown_requires_reconciliation".into())
        };
    }
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2",
            params![child.assignment_id,child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != number - 1 {
        return Err("source_round_sequence_conflict".into());
    }
    let legacy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?1)
        OR (?2=1 AND (EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed')
        OR EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?1)))",params![child.run_id,number],|r|r.get(0)).map_err(|e|e.to_string())?;
    if legacy {
        return Err("source_round_history_conflict".into());
    }
    verify_request(&tx, &call)?;
    verify_budget(&tx, &call)?;
    let text = request.to_string();
    super::super::budget::admission::require_determinate(&tx, &lease.root_run_id)?;
    let owner=if transport {Some(super::lifetime::claim(&tx,&call)?)} else {None};
    super::super::budget::clock::sample(&tx, lease, &child.assignment_id)?;
    tx.execute("INSERT INTO agent_source_model_rounds(assignment_id,round_number,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'executing')",
        params![child.assignment_id,number,child.run_id,lease.root_run_id,child.role.as_str(),lease.lease_epoch,lease.fencing_token,text,store::stable_hash(&text),reserved_tokens])
        .map_err(|e|format!("source_round_claim:{e}"))?;
    if load(&tx, &call)?.is_none_or(|r| r.state != "executing") {
        return Err("source_round_claim_missing".into());
    }
    authorize(&tx, &call)?;
    check(&tx)?;
    verify_request(&tx, &call)?;
    verify_budget(&tx, &call)?;
    verify_checkpoint(&tx, &call)?;
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2",
            params![child.assignment_id,child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != number {
        return Err("source_round_sequence_conflict".into());
    }
    super::super::budget::admission::require_determinate(&tx, &lease.root_run_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok((Start::Dispatch(call),owner))
}

pub fn record_uncertain(db: &Connection, call: &PendingRound, code: &str) -> Result<(), String> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if super::super::attempts::require_unexpired_response_worker(&tx, &call.lease, &call.child)
        .is_err()
    {
        super::super::budget::model_facts::source_round(
            &tx,
            &call.lease,
            &call.child,
            call.number,
            &store::stable_hash(&call.request.to_string()),
            call.reserved_tokens,
            super::super::budget::model_facts::Fact::Uncertain,
        )?;
        return tx.commit().map_err(|e| e.to_string());
    }
    unsent::require_no_terminal_fact(&tx, call)?;
    if load(&tx, call)?.is_none_or(|r| r.state != "executing") {
        return Err("source_round_not_executing".into());
    }
    let n=tx.execute("UPDATE agent_source_model_rounds SET state='uncertain',failure_code=?3,finished_at=datetime('now','localtime') WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?4 AND state='executing'",
        params![call.child.assignment_id,call.number,redact_text_with(code,None),call.child.run_id]).map_err(|e|e.to_string())?;
    if n != 1 || load(&tx, call)?.is_none_or(|r| r.state != "uncertain") {
        return Err("source_round_uncertain_missing".into());
    }
    super::super::budget::model::forfeit_call(
        &tx,
        &call.lease,
        &call.child.assignment_id,
        &format!(
            "source-round:{}:{}:{}",
            call.child.assignment_id,
            call.number,
            store::stable_hash(&call.request.to_string())
        ),
        Some(call.reserved_tokens),
    )?;
    tx.commit().map_err(|e| e.to_string())
}

/// Persist late provider results even after revocation: accounting is not an
/// execution grant. Tools and the next dispatch always recheck authorization.
pub fn record_received(
    db: &Connection,
    call: &PendingRound,
    result: &ModelResponse,
) -> Result<Receipt, String> {
    let used = usage(&result.usage.as_json())?;
    let mut rejection = "";
    let mut ids = std::collections::BTreeSet::new();
    let mut actions = std::collections::BTreeSet::new();
    let mut calls = Vec::new();
    if result.text.len() > 1_048_576 || result.tool_calls.len() > 64 {
        rejection = "source_round_response_too_large"
    }
    if rejection.is_empty() {
        for tool in &result.tool_calls {
            let allowed = call.request["tools"]
                .as_array()
                .is_some_and(|list| list.iter().any(|s| s["function"]["name"] == tool.name));
            if tool.id.is_empty()
                || tool.id.len() > 256
                || !ids.insert(&tool.id)
                || !actions.insert((&tool.name, tool.arguments.to_string()))
                || !allowed
                || !tool.arguments.is_object()
                || tool.arguments.to_string().len() > 1_048_576
                || redact_json(&tool.arguments) != tool.arguments
                || redact_text_with(&tool.id, None) != tool.id
            {
                rejection = "source_round_tool_call_invalid";
                break;
            }
            calls.push(json!({"id":tool.id,"name":tool.name,"arguments":tool.arguments}));
        }
    }
    if used.total_tokens > call.reserved_tokens {
        rejection = "source_round_provider_over_budget"
    }
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    for id in ids {
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_source_tool_receipts WHERE assignment_id=?1 AND call_id=?2 AND round_number<>?3 AND child_run_id=?4)",
            params![call.child.assignment_id,id,call.number,call.child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if exists {
            rejection = "source_round_tool_id_reused"
        }
    }
    if calls.len() > 1 && calls.iter().any(|c| c["name"] == "assignment.finish") {
        rejection = "source_round_finish_must_be_separate"
    }
    if !result.usage_reported {
        rejection = "model_usage_requires_reconciliation";
    }
    if !rejection.is_empty() {
        calls.clear()
    }
    let text = if result.text.len() > 1_048_576 {
        String::new()
    } else {
        redact_text_with(&result.text, None)
    };
    let response = json!({"text":text,"toolCalls":calls,"rejection":rejection,"usageReported":result.usage_reported});
    if let Err(error) =
        super::super::attempts::require_unexpired_response_worker(&tx, &call.lease, &call.child)
    {
        super::super::budget::model_facts::source_round(
            &tx,
            &call.lease,
            &call.child,
            call.number,
            &store::stable_hash(&call.request.to_string()),
            call.reserved_tokens,
            super::super::budget::model_facts::Fact::Received {
                hash: &hash(call, &response, &used.as_json()),
                usage: &used,
                reported: result.usage_reported,
            },
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        return Err(error);
    }
    unsent::require_no_terminal_fact(&tx, call)?;
    let row = load(&tx, call)?.ok_or("source_round_claim_missing")?;
    if row.state == "received" {
        if row.response != response || row.usage != used.as_json() {
            return Err("source_round_response_changed".into());
        }
        verify_tool_rows(&tx, call, &row.response)?;
        verify_checkpoint(&tx, call)?;
        return received(&tx, call, &row);
    }
    if row.state != "executing" {
        return Err("source_round_not_executing".into());
    }
    verify_checkpoint(&tx, call)?;
    let sequence = store::append_event(
        &tx,
        &call.child.run_id,
        AgentEventKind::ModelRoundCompleted,
        &payload(&tx, call, &response, &used)?,
        &[],
    )?;
    let n=tx.execute("UPDATE agent_source_model_rounds SET state='received',response_json=?3,usage_json=?4,response_hash=?5,event_sequence=?6,finished_at=datetime('now','localtime')
        WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?7 AND state='executing'",
        params![call.child.assignment_id,call.number,response.to_string(),used.as_json().to_string(),hash(call,&response,&used.as_json()),sequence,call.child.run_id]).map_err(|e|e.to_string())?;
    if n != 1 {
        return Err("source_round_receipt_missing".into());
    }
    super::super::directive::source_guidance::record_round_delivery(
        &tx,
        &call.lease,
        &call.child,
        call.number,
        sequence,
    )?;
    for (index, tool) in calls.iter().enumerate() {
        tx.execute("INSERT INTO agent_source_tool_receipts(assignment_id,round_number,call_index,call_id,tool_name,arguments_json,state,child_run_id) VALUES(?1,?2,?3,?4,?5,?6,'planned',?7)",
            params![call.child.assignment_id,call.number,index as i64,tool["id"].as_str(),tool["name"].as_str(),tool["arguments"].to_string(),call.child.run_id]).map_err(|e|e.to_string())?;
    }
    response_receipt(
        &tx,
        call,
        &load(&tx, call)?.ok_or("source_round_receipt_missing")?,
    )?;
    let source = format!(
        "source-round:{}:{}:{}",
        call.child.assignment_id,
        call.number,
        store::stable_hash(&call.request.to_string())
    );
    let receipt_hash = hash(call, &response, &used.as_json());
    if result.usage_reported {
        super::super::budget::receipts::record(
            &tx,
            &call.lease,
            &call.child.assignment_id,
            &source,
            &receipt_hash,
            &used,
            Some(call.reserved_tokens),
        )?;
    } else {
        super::super::budget::receipts::record_estimated(
            &tx,
            &call.lease,
            &call.child.assignment_id,
            &source,
            &receipt_hash,
            &used,
            Some(call.reserved_tokens),
        )?;
    }
    let sum = cumulative(&tx, call, call.number)?;
    let mut state = RunState::new(&call.child.run_id, Vec::new());
    state.turns = call.number;
    state.model_requests = sum.model_requests;
    state.input_tokens = sum.input_tokens;
    state.cached_input_tokens = sum.cached_input_tokens;
    state.output_tokens = sum.output_tokens;
    state.used_tokens = sum.total_tokens;
    state.progress_signature = store::stable_hash(&response.to_string());
    checkpoint::write_checkpoint(&tx, &state)?;
    let saved = received(
        &tx,
        call,
        &load(&tx, call)?.ok_or("source_round_receipt_missing")?,
    )?;
    verify_tool_rows(&tx, call, &saved.response)?;
    verify_checkpoint(&tx, call)?;
    received(
        &tx,
        call,
        &load(&tx, call)?.ok_or("source_round_receipt_missing")?,
    )?;
    verify_checkpoint(&tx, call)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(saved)
}
