//! Original Single SDK metadata/financial binding and existing inode probe only.
use super::*;
use crate::agent_runtime::execution_owner;
pub(crate) const INVOCATION_KIND: &str="single-root-sdk";

pub(crate) fn require_idle_original(
    db:&Connection, owner:&RootOwner,
)->Result<Option<execution_owner::NativeInvocationOwner>,String> {
    if db.is_autocommit() {return Err("single_model_lifetime_transaction_required".into());}
    let path=db.path().filter(|p|!p.is_empty()).ok_or("single_model_lifetime_database_missing")?;
    let original=RootOwner::load_single(db,&owner.root)?;
    if original.id!=owner.id || original.contract!=owner.contract {return Err("budget_root_original_owner_conflict".into());}
    let mut q=db.prepare("SELECT call_id,lease_attempt_id,round,request_hash,receipt_json FROM agent_root_model_journal
        WHERE root_run_id=?1 AND phase='dispatch' ORDER BY round").map_err(|e|e.to_string())?;
    let calls=q.query_map([&owner.root],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,
        r.get::<_,String>(3)?,r.get::<_,String>(4)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let orphan:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_model_journal j WHERE j.root_run_id=?1 AND NOT EXISTS(
        SELECT 1 FROM agent_root_model_journal d WHERE d.root_run_id=j.root_run_id AND d.call_id=j.call_id AND d.phase='dispatch'
        AND d.lease_attempt_id=j.lease_attempt_id AND d.round=j.round AND d.request_hash=j.request_hash))",[&owner.root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if orphan {return Err("single_model_lifetime_original_call_conflict".into());}
    if calls.is_empty() {return Ok(None);}
    for (index,(id,control,round,hash,raw)) in calls.into_iter().enumerate() {
        let value:Value=serde_json::from_str(&raw).map_err(|_|"single_model_lifetime_original_dispatch_invalid")?;
        let tokens=value["reservedTokens"].as_i64().filter(|n|*n>0).ok_or("single_model_lifetime_original_dispatch_invalid")?;
        let canonical=value.to_string();
        if canonical!=raw || value.as_object().is_none_or(|v|v.len()!=2) || value["reservedRequests"]!=1
            || control!=owner.id || round!=i64::try_from(index+1).map_err(|_|"single_model_lifetime_original_call_conflict")?
            || hash.len()!=64 || !hash.bytes().all(|b|b.is_ascii_hexdigit())
            || id!=crate::agent_runtime::store::stable_hash(&json!({"owner":owner.id,"root":owner.root,"round":round,"request":hash}).to_string()) {
            return Err("single_model_lifetime_original_call_conflict".into());
        }
        let call=RootModelCall {owner:owner.clone(),id,round,request:hash,tokens,remaining:Duration::ZERO};
        call.verify(db,"dispatch",&call.dispatch())?;
        for (dimension,amount) in DIMENSIONS[..4].iter().zip([tokens,tokens,tokens,1]) {
            let key=format!("root-model:{}:{dimension}:reserve",call.id);
            let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=''
                AND lease_attempt_id=?2 AND dimension=?3 AND kind='reserve' AND amount=?4 AND source_id=?5 AND idempotency_key=?6)",
                params![owner.root,owner.id,dimension,amount,call.id,budget::root::stored_key(db,&owner.root,&owner.id,&key)?],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !exact {return Err("budget_root_original_reservation_conflict".into());}
        }
    }
    let scope=&owner.contract["root"];
    execution_owner::probe_native_invocation(std::path::Path::new(path),scope["scan"].as_str().ok_or("budget_root_scope_invalid")?,
        scope["attempt"].as_i64().ok_or("budget_root_scope_invalid")?,INVOCATION_KIND,&owner.root)
        .map_err(|e|format!("single_model_transport_not_idle:{e}"))?.map(Some)
        .ok_or("single_model_transport_original_exit_proof_missing".into())
}
