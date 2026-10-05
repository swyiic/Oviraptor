//! Transactional SourceBroker tool delivery and receipt-backed continuation.
use super::*;

pub(super) fn verify_tool_rows(
    db: &Connection,
    call: &PendingRound,
    response: &Value,
) -> Result<(), String> {
    let calls = response["toolCalls"]
        .as_array()
        .ok_or("source_round_tool_list_invalid")?;
    let count:i64=db.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?3",
        params![call.child.assignment_id,call.number,call.child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count != calls.len() as i64 {
        return Err("source_round_tool_receipts_missing".into());
    }
    for (index, tool) in calls.iter().enumerate() {
        let (id,name,args):(String,String,String)=db.query_row("SELECT call_id,tool_name,arguments_json FROM agent_source_tool_receipts WHERE assignment_id=?1 AND round_number=?2 AND call_index=?3 AND child_run_id=?4",
            params![call.child.assignment_id,call.number,index as i64,call.child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if tool["id"] != id || tool["name"] != name || tool["arguments"] != parse(args)? {
            return Err("source_round_tool_binding_changed".into());
        }
        tool_output(db, call, index as i64, tool)?;
    }
    Ok(())
}

pub(super) fn tool_output(
    db: &Connection,
    call: &PendingRound,
    index: i64,
    tool: &Value,
) -> Result<Option<Value>, String> {
    let (state,text,hash_out,sequence):(String,String,String,i64)=db.query_row(
        "SELECT state,output_json,receipt_hash,event_sequence FROM agent_source_tool_receipts WHERE assignment_id=?1 AND round_number=?2 AND call_index=?3 AND child_run_id=?4",
        params![call.child.assignment_id,call.number,index,call.child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    if state == "planned" {
        return Ok(None);
    }
    let output = parse(text)?;
    let event = tool_payload(call, index, tool, &output);
    let persisted:Option<String>=db.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND sequence=?2 AND event_type='tool_invocation_completed'",
        params![call.child.run_id,sequence],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if state != "completed"
        || hash_out != store::stable_hash(&event.to_string())
        || persisted.map(parse).transpose()? != Some(event)
    {
        return Err("source_round_tool_receipt_invalid".into());
    }
    Ok(Some(output))
}

fn tool_payload(call: &PendingRound, index: i64, tool: &Value, output: &Value) -> Value {
    json!({"sourceRound":call.number,"assignmentId":call.child.assignment_id,"callIndex":index,"call":tool,"output":output,"targetRequestsDelta":0})
}

/// The callback MUST be a local SourceBroker call on this transaction: no
/// network, shell, file mutations, or nested connection/transaction. A failed
/// callback rolls back its DB changes; successful output and side effects
/// commit together. It cannot complete the assignment by merely returning text.
pub fn execute_tool(
    db: &Connection,
    call: &PendingRound,
    index: i64,
    check: impl Fn(&Connection) -> Result<(), String>,
    execute: impl FnOnce(&Connection, &str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    authorize(&tx, call)?;
    check(&tx)?;
    verify_checkpoint(&tx, call)?;
    let receipt = received(
        &tx,
        call,
        &load(&tx, call)?.ok_or("source_round_claim_missing")?,
    )?;
    verify_tool_rows(&tx, call, &receipt.response)?;
    let calls = receipt.response["toolCalls"]
        .as_array()
        .ok_or("source_round_tool_list_invalid")?;
    let tool = usize::try_from(index)
        .ok()
        .and_then(|n| calls.get(n))
        .ok_or("source_round_tool_index_invalid")?;
    if let Some(output) = tool_output(&tx, call, index, tool)? {
        return Ok(output);
    }
    for (earlier, previous) in calls.iter().enumerate().take(index as usize) {
        if tool_output(&tx, call, earlier as i64, previous)?.is_none() {
            return Err("source_round_tool_order_invalid".into());
        }
    }
    let name = tool["name"].as_str().ok_or("source_round_tool_invalid")?;
    let output = redact_json(&execute(&tx, name, &tool["arguments"])?);
    if output.to_string().len() > 2_000_000 {
        return Err("source_round_tool_output_too_large".into());
    }
    let event = tool_payload(call, index, tool, &output);
    let sequence = store::append_event(
        &tx,
        &call.child.run_id,
        AgentEventKind::ToolInvocationCompleted,
        &event,
        &[],
    )?;
    let n=tx.execute("UPDATE agent_source_tool_receipts SET state='completed',output_json=?4,receipt_hash=?5,event_sequence=?6 WHERE assignment_id=?1 AND round_number=?2 AND call_index=?3 AND child_run_id=?7 AND state='planned'",
        params![call.child.assignment_id,call.number,index,output.to_string(),store::stable_hash(&event.to_string()),sequence,call.child.run_id]).map_err(|e|e.to_string())?;
    authorize(&tx, call)?;
    check(&tx)?;
    verify_checkpoint(&tx, call)?;
    verify_tool_rows(&tx, call, &receipt.response)?;
    received(
        &tx,
        call,
        &load(&tx, call)?.ok_or("source_round_claim_missing")?,
    )?;
    if n != 1 || tool_output(&tx, call, index, tool)? != Some(output.clone()) {
        return Err("source_round_tool_commit_invalid".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(output)
}

/// Build the next exact transcript only from committed model/tool receipts.
/// A text-only response is not permission to dispatch indefinitely or finish.
pub fn continuation(db: &Connection, call: &PendingRound) -> Result<Value, String> {
    let receipt = received(
        db,
        call,
        &load(db, call)?.ok_or("source_round_claim_missing")?,
    )?;
    verify_tool_rows(db, call, &receipt.response)?;
    let calls = receipt.response["toolCalls"]
        .as_array()
        .ok_or("source_round_tool_list_invalid")?;
    if receipt.response["rejection"] != ""
        || calls.is_empty()
        || calls.iter().any(|c| c["name"] == "assignment.finish")
    {
        return Err("source_round_no_continuation".into());
    }
    let mut request = call.request.clone();
    let messages = request["messages"]
        .as_array_mut()
        .ok_or("source_round_messages_invalid")?;
    let wire=calls.iter().map(|c|json!({"id":c["id"],"type":"function","function":{"name":c["name"],"arguments":c["arguments"].to_string()}})).collect::<Vec<_>>();
    messages.push(json!({"role":"assistant","content":receipt.response["text"],"tool_calls":wire}));
    for (index, tool) in calls.iter().enumerate() {
        let output =
            tool_output(db, call, index as i64, tool)?.ok_or("source_round_tools_pending")?;
        messages
            .push(json!({"role":"tool","tool_call_id":tool["id"],"content":output.to_string()}));
    }
    Ok(request)
}

/// Read-only, receipt-derived finish proof for an atomic commands-layer
/// settlement/mailbox transaction. This grants no execution capability.
pub fn completion(db: &Connection, call: &PendingRound) -> Result<(Value, UsageDelta), String> {
    let row = load(db, call)?.ok_or("source_round_claim_missing")?;
    let receipt = received(db, call, &row)?;
    verify_tool_rows(db, call, &receipt.response)?;
    verify_checkpoint(db, call)?;
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2",
            params![call.child.assignment_id,call.child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let calls = receipt.response["toolCalls"]
        .as_array()
        .ok_or("source_round_tool_list_invalid")?;
    if count != call.number
        || receipt.response["rejection"] != ""
        || calls.len() != 1
        || calls[0]["name"] != "assignment.finish"
    {
        return Err("source_round_finish_receipt_required".into());
    }
    let output = tool_output(db, call, 0, &calls[0])?.ok_or("source_round_tools_pending")?;
    if output["status"] != "finished"
        || output["summary"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
        || output["conclusive"] != false
        || !output["gaps"]
            .as_array()
            .is_some_and(|g| g.contains(&json!("source_review_not_completed")))
    {
        return Err("source_round_finish_contract_invalid".into());
    }
    Ok((output, cumulative(db, call, call.number)?))
}
