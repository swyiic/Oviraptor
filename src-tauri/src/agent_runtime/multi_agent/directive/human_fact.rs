//! Pure original confirmed fact; no import or repair of older human records.
use super::*;

pub(crate) fn confirmed_fact_for_root(
    db: &Connection,
    actor: &CoordinatorLease,
    id: &str,
) -> Result<serde_json::Value, String> {
    let raw: String = db.query_row("SELECT json_object('id',id,'scan',scan_id,'attempt',attempt_number,
        'root',root_run_id,'target',target_key,'role',recipient_role,'thread',thread_key,'text',text_redacted,
        'status',status,'source',source_draft_id,'revision',confirmed_revision,'hash',confirmed_hash,
        'confirmation',confirmation_at,'claim',claim_run_id,'epoch',claim_lease_epoch,'fence',claim_fencing_token,
        'payload',json(payload_json)) FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0))
        .map_err(|_|"root_human_directive_missing")?;
    let record: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "root_human_directive_invalid")?;
    let invalid = "root_human_original_confirmation_invalid";
    let draft = load_draft(db, record["source"].as_str().ok_or(invalid)?)?.ok_or(invalid)?;
    let review = human_review::receipt_for(db, &draft.id)?
        .ok_or("root_human_original_confirmation_missing")?;
    let expected = confirmed_payload(&draft);
    if !draft_integrity_valid(&draft)
        || draft.status != "confirmed"
        || draft.validation_result == "rejected"
        || draft.confirmed_directive_id != id
        || record["id"] != id
        || record["scan"] != actor.scan_id
        || draft.scan_id != actor.scan_id
        || record["attempt"] != actor.attempt_number
        || draft.attempt_number != actor.attempt_number
        || record["root"] != actor.root_run_id
        || draft.root_run_id != actor.root_run_id
        || record["target"] != actor.target_key
        || draft.target_key != actor.target_key
        || record["claim"] != actor.root_run_id
        || record["epoch"] != actor.lease_epoch
        || record["fence"] != actor.fencing_token
        || draft.bound_lease_epoch != actor.lease_epoch
        || draft.bound_fencing_token != actor.fencing_token
        || record["revision"] != draft.revision
        || record["hash"] != draft.draft_hash
        || record["role"] != draft.recipient_role
        || record["thread"] != draft.thread_key
        || record["text"] != draft.safe_execution_text
        || !matches!(
            record["status"].as_str(),
            Some("accepted" | "applied" | "completed" | "deferred")
        )
        || !record["confirmation"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        || review["kind"] != "approve"
        || review["directiveId"] != id
        || expected
            .as_object()
            .ok_or(invalid)?
            .iter()
            .any(|(k, v)| record["payload"].get(k) != Some(v))
    {
        return Err(invalid.into());
    }
    validate_thread_key(db, &actor.root_run_id, &actor.target_key, &draft.thread_key)?;
    if !fact_refs_current(db, &actor.root_run_id, &draft.referenced_fact_ids)? {
        return Err("root_human_original_facts_changed".into());
    }
    Ok(
        json!({"directiveId":id,"draftId":draft.id,"revision":draft.revision,"draftHash":draft.draft_hash,
        "confirmationReceiptId":review["receiptId"],"threadKey":draft.thread_key,"targetKey":draft.target_key,
        "recipientRole":draft.recipient_role,"intent":draft.intent,"confirmedText":draft.safe_execution_text,
        "requestedRoles":draft.requested_roles,"requestedContracts":draft.requested_contracts,
        "priorityChanges":draft.priority_changes,"referencedFactIds":draft.referenced_fact_ids,
        "proposedScopeChange":draft.proposed_scope_change,"sideEffectClass":draft.side_effect_class}),
    )
}
