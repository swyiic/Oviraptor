// A closed readonly metadata assessment conveys no target or identity grant.
#[derive(serde::Deserialize,serde::Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct NativeCoordinatorIdentityFact {
    summary:String,
    observed_authentication:Vec<String>,
    evidence_gaps:Vec<String>,
    authorization_proven:bool,
}
fn native_coordinator_identity_semantic(text:&str)->Result<JsonValue,String> {
    let output:NativeCoordinatorIdentityFact=serde_json::from_str(text).map_err(|_|"root_identity_schema_invalid")?;
    if output.authorization_proven || output.summary.trim().is_empty() || output.summary.chars().count()>2048
        || output.summary.chars().any(char::is_control) || output.evidence_gaps.is_empty()
        || [&output.observed_authentication,&output.evidence_gaps].iter().any(|a|a.len()>16 || a.iter().any(|s|
            s.trim().is_empty() || s.chars().count()>512 || s.chars().any(char::is_control))) {
        return Err("root_identity_schema_invalid".into());
    }
    serde_json::to_value(output).map(|v|crate::agent_runtime::secrets::redact_json(&v)).map_err(|e|e.to_string())
}
fn native_coordinator_identity_task(
    db:&rusqlite::Connection, actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
)->Result<JsonValue,String> {
    use crate::agent_runtime::contract::AgentRole;
    if child.role!=AgentRole::IdentitySession {return Err("root_identity_role_invalid".into());}
    let (raw,capabilities):(String,String)=db.query_row("SELECT task_slice_json,capability_lease_json FROM agent_assignments
        WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND target_key=?4
        AND role='identity_session' AND lane='read_only_analysis'",
        params![child.assignment_id,child.run_id,actor.root_run_id,actor.target_key],|r|Ok((r.get(0)?,r.get(1)?)))
        .map_err(|_|"root_identity_metadata_scope_invalid")?;
    let task:JsonValue=serde_json::from_str(&raw).map_err(|_|"root_identity_task_invalid")?;
    let grants:Vec<String>=serde_json::from_str(&capabilities).map_err(|_|"root_identity_metadata_scope_invalid")?;
    let (mode,handles)=crate::auth_session::validated_scan_identities(db,&actor.scan_id,&actor.target_key)?;
    if handles.is_empty() || handles.len()>16 || task["identityMode"]!=format!("{mode:?}")
        || task["identityCount"]!=handles.len() || grants!=vec!["evidence.read","mailbox.write"] {
        return Err("root_identity_metadata_scope_invalid".into());
    }
    Ok(task)
}
impl NativeCoordinatorFrame {
    fn identity_session(
        tx:&rusqlite::Transaction<'_>,actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    )->Result<Self,String> {
        use crate::agent_runtime::{multi_agent::{mailbox,specialist},store};
        let task=native_coordinator_identity_task(tx,actor,child)?;
        let receipt=specialist::received_for_reconciliation(tx,actor,child)?;
        if !receipt.rejection.is_empty() {return Err("root_identity_paid_output_rejected".into());}
        let output=native_coordinator_identity_semantic(&receipt.text)?;
        let id:String=tx.query_row("SELECT id FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2
            AND from_run_id=?3 AND to_run_id=?1 AND from_agent='identity_session' AND to_agent='coordinator'
            AND kind='identity_assessment' AND correlation_id=?4 AND evidence_revision=1",
            params![actor.root_run_id,child.assignment_id,child.run_id,format!("identity:{}",child.assignment_id)],|r|r.get(0))
            .map_err(|_|"root_identity_output_missing")?;
        let message=mailbox::verified_completed_output(tx,actor,&id)?;
        if message.payload!=json!({"summary":receipt.text,"identityMode":task["identityMode"]}) {
            return Err("root_identity_output_receipt_mismatch".into());
        }
        native_coordinator_identity_message_events(tx,actor,&id)?;
        let mut rows=Self::closed_rows(tx,actor,child)?;
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM sentinel_scan_contexts WHERE scan_id=?1",&[&actor.scan_id])?);
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM browser_auth_sessions WHERE owner_scan_id=?1
            AND id IN (SELECT value FROM sentinel_scan_contexts c,json_each(c.policy_json,'$.authSessionIds') WHERE c.scan_id=?1) ORDER BY rowid",&[&actor.scan_id])?);
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN (?1,?2,?3) ORDER BY sequence",
            &[&child.run_id,&child.assignment_id,&id])?);
        Ok(Self {kind:"identity-session-output",
            semantic:json!({"messageId":id,"role":"identity_session","assignmentId":child.assignment_id,"runId":child.run_id,
                "taskHash":store::stable_hash(&task.to_string()),"identityMode":task["identityMode"],"identityCount":task["identityCount"],
                "output":output,"targetRequestsGranted":0,"authorizationProven":false}),
            rows,review_refs:None,review_meta:None})
    }
    fn verify_identity_source(&self,db:&rusqlite::Connection,actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease)->Result<(),String> {
        use crate::agent_runtime::{contract::AgentRole,multi_agent::scheduler::ScheduledChild,store};
        let child=ScheduledChild {assignment_id:self.semantic["assignmentId"].as_str().ok_or("root_identity_scope_invalid")?.into(),
            run_id:self.semantic["runId"].as_str().ok_or("root_identity_scope_invalid")?.into(),role:AgentRole::IdentitySession};
        let task=native_coordinator_identity_task(db,actor,&child)?;
        if self.semantic["taskHash"]!=store::stable_hash(&task.to_string()) {return Err("root_identity_original_task_changed".into());}
        native_coordinator_identity_message_events(db,actor,self.semantic["messageId"].as_str().ok_or("root_identity_output_missing")?)?;
        Ok(())
    }
}
