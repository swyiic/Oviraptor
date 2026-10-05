//! Durable rounds for the source_tools phase, separate from tool-free initial
//! assessment. Provider I/O is outside SQLite transactions. Only local,
//! transactional SourceBroker operations may run inside `execute_tool`.
//! Receipts record usage, not successful assignment completion or review.
use super::{lease::CoordinatorLease, scheduler::ScheduledChild, source};
use crate::agent_runtime::{
    checkpoint::{self, RunState},
    contract::AgentEventKind,
    model::gateway::ModelResponse,
    secrets::{redact_json, redact_text_with},
    store::{self, UsageDelta},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

mod completion_audit;
mod dispatch;
mod lifetime;
mod reentry;
mod tool_delivery;
mod unsent;
pub(crate) use completion_audit::{audit_completion, audit_exhausted, SourceRoundAudit};
pub(crate) use dispatch::{prepare_next_authorized,start_for_transport};
pub(crate) use lifetime::require_idle_for_root;
pub use dispatch::{record_received, record_uncertain};
#[cfg(test)]
pub use dispatch::start_authorized;
pub(crate) use reentry::{
    audit_pending_finish, audit_recoverable_boundary, audit_saved_finish_transition,
    audit_settled_finish,
};
pub use tool_delivery::{completion, continuation, execute_tool};
use tool_delivery::{tool_output, verify_tool_rows};
pub(crate) use unsent::{audit_unsent_first, record_not_sent};

#[derive(Clone, Debug)]
pub struct PendingRound {
    lease: CoordinatorLease,
    child: ScheduledChild,
    number: i64,
    request: Value,
    reserved_tokens: i64,
}

impl PendingRound {
    /// Send this exact persisted request, never the caller's mutable transcript.
    pub fn request(&self) -> &Value {
        &self.request
    }
}

#[derive(Debug)]
pub struct Receipt {
    pub response: Value,
    pub usage: UsageDelta,
}

#[derive(Debug)]
pub enum Start {
    Dispatch(PendingRound),
    Received(PendingRound, Receipt),
}

struct Row {
    state: String,
    response: Value,
    usage: Value,
    hash: String,
    sequence: i64,
}

fn parse(value: String) -> Result<Value, String> {
    serde_json::from_str(&value).map_err(|_| "source_round_json_invalid".into())
}

fn load(db: &Connection, call: &PendingRound) -> Result<Option<Row>, String> {
    let row = db.query_row(
        "SELECT child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state,response_json,usage_json,response_hash,event_sequence
         FROM agent_source_model_rounds WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?3",
        params![call.child.assignment_id,call.number,call.child.run_id], |r| Ok((
            r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,
            r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,i64>(7)?,
            r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,String>(10)?,r.get::<_,String>(11)?,r.get::<_,i64>(12)?
        )),
    ).optional().map_err(|e|format!("source_round_load:{e}"))?;
    let Some((
        child,
        root,
        role,
        epoch,
        fence,
        request,
        hash,
        reserved,
        state,
        response,
        usage,
        hash_out,
        sequence,
    )) = row
    else {
        return Ok(None);
    };
    if child != call.child.run_id
        || root != call.lease.root_run_id
        || role != call.child.role.as_str()
        || epoch != call.lease.lease_epoch
        || fence != call.lease.fencing_token
        || parse(request.clone())? != call.request
        || hash != store::stable_hash(&request)
        || reserved != call.reserved_tokens
    {
        return Err("source_round_binding_changed".into());
    }
    Ok(Some(Row {
        state,
        response: parse(response)?,
        usage: parse(usage)?,
        hash: hash_out,
        sequence,
    }))
}

fn authorize(db: &Connection, call: &PendingRound) -> Result<(), String> {
    let a = source::authorize_tool(
        db,
        &call.lease.scan_id,
        call.lease.attempt_number,
        &call.lease.target_key,
        &call.child.run_id,
        "assignment.finish",
    )?;
    if a.lease.root_run_id != call.lease.root_run_id
        || a.lease.lease_epoch != call.lease.lease_epoch
        || a.lease.fencing_token != call.lease.fencing_token
    {
        return Err("source_round_coordinator_changed".into());
    }
    let valid:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE a.id=?1 AND a.child_run_id=?2 AND r.assignment_id=a.id AND a.role=?3
         AND a.budget_settled_at='' AND a.state='running' AND r.status='running')",
        params![call.child.assignment_id,call.child.run_id,call.child.role.as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("source_round_assignment_invalid".into());
    }
    let tools = call.request["tools"]
        .as_array()
        .ok_or("source_round_tools_invalid")?;
    let mut names = std::collections::BTreeSet::new();
    for tool in tools {
        let name = tool["function"]["name"]
            .as_str()
            .ok_or("source_round_tool_invalid")?;
        if tool["type"] != "function" || !names.insert(name) || !a.tools.iter().any(|s| s == name) {
            return Err("source_round_tool_not_granted".into());
        }
    }
    if names.len() != a.tools.len() {
        return Err("source_round_tools_changed".into());
    }
    Ok(())
}

fn usage(value: &Value) -> Result<UsageDelta, String> {
    let n = |key| {
        value[key]
            .as_i64()
            .filter(|n| *n >= 0)
            .ok_or("source_round_usage_invalid".to_string())
    };
    let u = UsageDelta {
        input_tokens: n("inputTokens")?,
        cached_input_tokens: n("cachedInputTokens")?,
        output_tokens: n("outputTokens")?,
        total_tokens: n("totalTokens")?,
        model_requests: n("modelRequests")?,
    };
    if u.model_requests != 1
        || u.cached_input_tokens > u.input_tokens
        || u.total_tokens < 1
        || u.input_tokens
            .checked_add(u.output_tokens)
            .is_none_or(|sum| sum > u.total_tokens)
    {
        return Err("source_round_usage_invalid".into());
    }
    Ok(u)
}

fn hash(call: &PendingRound, response: &Value, used: &Value) -> String {
    store::stable_hash(&json!({"assignment":call.child.assignment_id,"child":call.child.run_id,
        "root":call.lease.root_run_id,"epoch":call.lease.lease_epoch,"fence":call.lease.fencing_token,
        "round":call.number,"request":call.request,"reservedTokens":call.reserved_tokens,
        "response":response,"usage":used}).to_string())
}

fn payload(
    db: &Connection,
    call: &PendingRound,
    response: &Value,
    used: &UsageDelta,
) -> Result<Value, String> {
    let ids =
        super::directive::source_guidance::round_ids(db, &call.lease, &call.child, call.number)?;
    Ok(
        json!({"turns":call.number,"modelRequests":1,"totalTokens":used.total_tokens,
        "uncachedInputTokens":used.uncached_input(),"sourceRound":call.number,
        "toolCalls":response["toolCalls"],"usage":used.as_json(),"deliveredDirectiveIds":ids}),
    )
}

fn response_receipt(db: &Connection, call: &PendingRound, row: &Row) -> Result<Receipt, String> {
    if row.state != "received"
        || row.sequence < 1
        || row.hash != hash(call, &row.response, &row.usage)
    {
        return Err("source_round_receipt_invalid".into());
    }
    let used = usage(&row.usage)?;
    let event:Option<String>=db.query_row(
        "SELECT payload_json FROM agent_events WHERE run_id=?1 AND sequence=?2 AND event_type='model_round_completed'",
        params![call.child.run_id,row.sequence],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if event.map(parse).transpose()? != Some(payload(db, call, &row.response, &used)?) {
        return Err("source_round_event_invalid".into());
    }
    super::directive::source_guidance::verify_round_delivery(
        db,
        &call.lease,
        &call.child,
        call.number,
        row.sequence,
    )?;
    Ok(Receipt {
        response: row.response.clone(),
        usage: used,
    })
}

fn received(db: &Connection, call: &PendingRound, row: &Row) -> Result<Receipt, String> {
    let saved = response_receipt(db, call, row)?;
    super::budget::receipts::verify(
        db,
        &call.lease,
        &call.child.assignment_id,
        &format!(
            "source-round:{}:{}:{}",
            call.child.assignment_id,
            call.number,
            store::stable_hash(&call.request.to_string())
        ),
        &row.hash,
        &saved.usage,
    )?;
    Ok(saved)
}

/// Read-only proof of the specified model delivery, even before tool execution or
/// settlement. It deliberately does not claim that this assignment completed.
pub(crate) fn guidance_received_for_audit(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    number: i64,
) -> Result<(), String> {
    if db.is_autocommit() || !source::is_source_role(child.role) {
        return Err("source_completion_receipt_context_invalid".into());
    }
    let seed = PendingRound {
        lease: lease.clone(),
        child: child.clone(),
        number,
        request: Value::Null,
        reserved_tokens: 0,
    };
    let call = historical(db, &seed, number)?;
    verify_request(db, &call)?;
    received(
        db,
        &call,
        &load(db, &call)?.ok_or("source_round_history_missing")?,
    )?;
    Ok(())
}

fn historical(db: &Connection, call: &PendingRound, number: i64) -> Result<PendingRound, String> {
    let (request,reserved):(String,i64)=db.query_row(
        "SELECT request_json,reserved_tokens FROM agent_source_model_rounds WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?3",
        params![call.child.assignment_id,number,call.child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"source_round_history_missing")?;
    Ok(PendingRound {
        number,
        request: parse(request)?,
        reserved_tokens: reserved,
        ..call.clone()
    })
}

fn cumulative(db: &Connection, call: &PendingRound, end: i64) -> Result<UsageDelta, String> {
    let mut sum = UsageDelta::default();
    for number in 1..=end {
        let previous = historical(db, call, number)?;
        let u = received(
            db,
            &previous,
            &load(db, &previous)?.ok_or("source_round_history_missing")?,
        )?
        .usage;
        let add = |a: i64, b: i64| a.checked_add(b).ok_or("source_round_usage_overflow");
        sum = UsageDelta {
            input_tokens: add(sum.input_tokens, u.input_tokens)?,
            cached_input_tokens: add(sum.cached_input_tokens, u.cached_input_tokens)?,
            output_tokens: add(sum.output_tokens, u.output_tokens)?,
            total_tokens: add(sum.total_tokens, u.total_tokens)?,
            model_requests: add(sum.model_requests, 1)?,
        };
    }
    Ok(sum)
}

include!("source_rounds/validation.rs");
