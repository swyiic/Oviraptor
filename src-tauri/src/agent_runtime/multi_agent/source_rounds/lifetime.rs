//! Actual Source tools SDK ownership, separate from durable financial rounds.
use super::*;
use crate::agent_runtime::{execution_owner,multi_agent::budget};
const KIND:&str="source-round-sdk";

fn original(db:&Connection,call:&PendingRound)->Result<(),String> {
    if db.is_autocommit() || !matches!(call.child.role,crate::agent_runtime::contract::AgentRole::RepoMapper
        | crate::agent_runtime::contract::AgentRole::SourceAnalyst) {
        return Err("source_round_lifetime_context_invalid".into());
    }
    budget::root::RootOwner::load_original(db,&call.lease.root_run_id)?.require_original_coordinator(db,&call.lease)?;
    budget::receipts::original_owner(db,&call.child.run_id,&call.lease,&call.child.assignment_id)?.verify(db)?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignments a
        ON a.id=r.assignment_id AND a.child_run_id=r.id WHERE r.id=?1 AND r.assignment_id=?2
        AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.scan_id=?4 AND r.attempt_number=?5 AND r.target_url=?6
        AND r.role=?7 AND r.backend='native' AND r.orchestration_policy='multi'
        AND a.coordinator_run_id=?3 AND a.lease_epoch=?8 AND a.fencing_token=?9
        AND json_extract(a.task_slice_json,'$.phase')='source_tools')",
        params![call.child.run_id,call.child.assignment_id,call.lease.root_run_id,call.lease.scan_id,
            call.lease.attempt_number,call.lease.target_key,call.child.role.as_str(),call.lease.lease_epoch,call.lease.fencing_token],
        |r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {return Err("source_round_lifetime_original_worker_conflict".into());}
    Ok(())
}
fn path(db:&Connection)->Result<&std::path::Path,String> {
    db.path().filter(|s|!s.is_empty()).map(std::path::Path::new).ok_or("source_round_lifetime_database_missing".into())
}
pub(super) fn claim(db:&Connection,call:&PendingRound)->Result<execution_owner::NativeInvocationOwner,String> {
    original(db,call)?;
    execution_owner::claim_native_invocation(path(db)?,&call.lease.scan_id,call.lease.attempt_number,KIND,&call.child.run_id)
}
pub(super) fn existing(db:&Connection,call:&PendingRound)->Result<execution_owner::NativeInvocationOwner,String> {
    original(db,call)?;
    execution_owner::probe_native_invocation(path(db)?,&call.lease.scan_id,call.lease.attempt_number,KIND,&call.child.run_id)
        .map_err(|e|format!("source_round_transport_not_idle:{e}"))?
        .ok_or("source_round_transport_original_exit_proof_missing".into())
}

pub(crate) fn require_idle_for_root(db:&Connection,scope:&CoordinatorLease)
    ->Result<Vec<execution_owner::NativeInvocationOwner>,String> {
    if db.is_autocommit() {return Err("source_round_lifetime_transaction_required".into());}
    let mut q=db.prepare("SELECT assignment_id,child_run_id,role,round_number,request_json,reserved_tokens
        FROM agent_source_model_rounds WHERE root_run_id=?1 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows=q.query_map([&scope.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
        r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).map_err(|e|e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut children=std::collections::BTreeMap::new();
    for (assignment_id,run_id,role,number,raw,reserved_tokens) in rows {
        let role=crate::agent_runtime::contract::AgentRole::try_parse(&role).filter(|r|r.as_str()==role)
            .ok_or("source_round_lifetime_role_invalid")?;
        let request:Value=serde_json::from_str(&raw).map_err(|_|"source_round_lifetime_request_invalid")?;
        let canonical=request.to_string();
        if !(1..=256).contains(&number) || reserved_tokens<=0 || canonical!=raw || redact_json(&request)!=request {
            return Err("source_round_lifetime_request_invalid".into());
        }
        let call=PendingRound {lease:scope.clone(),child:ScheduledChild {assignment_id,run_id,role},number,request,reserved_tokens};
        original(db,&call)?;
        let row=load(db,&call)?.ok_or("source_round_lifetime_original_claim_missing")?;
        if !matches!(row.state.as_str(),"executing"|"received"|"uncertain") {
            return Err("source_round_lifetime_original_claim_invalid".into());
        }
        verify_request(db,&call)?;
        children.insert(call.child.run_id.clone(),call);
    }
    // An empty history never creates a physical exit proof. Each original
    // child is probed once, with guards kept through the Root transaction.
    children.values().map(|call|existing(db,call)).collect()
}
